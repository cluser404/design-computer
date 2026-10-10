use crate::rpc::*;
use crate::schema;
use dc_doc::{Actor, Authority, Command, CommandKind, Hash128, Policy, Query, Revision, Role};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: &str = "0.1";

/// What a client sends to change the document. It has no `actor` field: the server knows who is
/// connected. `id` and `base_revision` are optional conveniences for simple clients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandRequest {
    /// Unique command ID. Omit to get a deterministic one derived from (actor, revision, command);
    /// resubmitting an applied command is then rejected as a duplicate, which makes retries safe.
    #[serde(default)]
    pub id: Option<String>,
    /// Revision your view of the document was based on. Omit to mean "the current revision".
    /// Use `expected` preconditions inside the command to detect concurrent edits.
    #[serde(default)]
    pub base_revision: Option<Revision>,
    #[serde(flatten)]
    pub kind: CommandKind,
}

/// A connection to one document, acting as one actor.
pub struct Session {
    authority: Authority,
    actor: Actor,
    journal: Option<File>,
    /// Where exports may be written. Host configuration; clients cannot change it.
    export_dir: Option<PathBuf>,
    /// Unknown entries from an opened `.dcad`, written back unchanged on save.
    extras: std::collections::BTreeMap<String, Vec<u8>>,
}

impl Session {
    /// In-memory session with default policy.
    pub fn new(actor: Actor) -> Self {
        Self {
            authority: Authority::default(),
            actor,
            journal: None,
            export_dir: None,
            extras: Default::default(),
        }
    }

    /// Opens a native `.dcad` file: replays and verifies its history. `role` is host config.
    pub fn from_dcad(path: &Path, actor: Actor, role: Option<Role>) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let d = dc_io::read(&bytes, Policy::default()).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        })?;
        let mut authority = d.authority;
        if let Some(r) = role {
            authority.set_role(&actor, r);
        }
        Ok(Self {
            authority,
            actor,
            journal: None,
            export_dir: None,
            extras: d.extras,
        })
    }

    /// Allows STL export into `dir` (created if missing). Without this, export is refused.
    pub fn with_export_dir(mut self, dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        self.export_dir = Some(fs::canonicalize(dir)?);
        Ok(self)
    }

    /// Opens a session, optionally backed by an append-only journal at `log`. An existing journal
    /// is replayed (verifying every state hash); each later commit is appended and synced.
    /// `role` is host configuration (e.g. a CLI flag); clients cannot change it.
    pub fn open(log: Option<&Path>, actor: Actor, role: Option<Role>) -> io::Result<Self> {
        let mut authority = match log {
            Some(p) if p.exists() => {
                let text = fs::read_to_string(p)?;
                Authority::replay(&text, Policy::default()).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", p.display()))
                })?
            }
            _ => Authority::default(),
        };
        if let Some(r) = role {
            authority.set_role(&actor, r);
        }
        let journal = match log {
            Some(p) => {
                let needs_newline = fs::read(p)
                    .map(|b| !b.is_empty() && b.last() != Some(&b'\n'))
                    .unwrap_or(false);
                let mut f = OpenOptions::new().create(true).append(true).open(p)?;
                if needs_newline {
                    f.write_all(b"\n")?;
                }
                Some(f)
            }
            None => None,
        };
        Ok(Self {
            authority,
            actor,
            journal,
            export_dir: None,
            extras: Default::default(),
        })
    }

    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn role(&self) -> Role {
        self.authority.role_of(&self.actor)
    }

    fn build_command(&self, req: CommandRequest) -> Command {
        let base = req
            .base_revision
            .unwrap_or_else(|| self.authority.revision());
        let id = req.id.unwrap_or_else(|| {
            let kind = serde_json::to_string(&req.kind).expect("commands always serialize");
            let h = Hash128::of(&[
                self.actor.key().as_bytes(),
                &base.0.to_le_bytes(),
                kind.as_bytes(),
            ]);
            format!("auto-{h}")
        });
        Command {
            id,
            actor: self.actor.clone(),
            base_revision: base,
            kind: req.kind,
        }
    }

    fn parse_request(&self, params: Value) -> Result<CommandRequest, RpcError> {
        if params.get("actor").is_some() {
            return Err(RpcError::invalid_params(
                "'actor' must not be sent: the server binds the actor to the session",
            ));
        }
        serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))
    }

    /// The single entry point for every method, regardless of transport.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "session.info" => Ok(json!({
                "protocol": PROTOCOL_VERSION,
                "actor": self.actor.key(),
                "role": self.role(),
                "revision": self.authority.revision(),
                "state_hash": self.authority.state_hash(),
            })),
            "command.dry_run" => {
                let cmd = self.build_command(self.parse_request(params)?);
                let out = self
                    .authority
                    .dry_run(&cmd)
                    .map_err(|d| RpcError::from_diagnostic(&d))?;
                Ok(serde_json::to_value(out).expect("outcomes always serialize"))
            }
            "command.submit" => {
                let cmd = self.build_command(self.parse_request(params)?);
                let out = self
                    .authority
                    .submit(cmd)
                    .map_err(|d| RpcError::from_diagnostic(&d))?;
                if let Some(j) = self.journal.as_mut() {
                    let entry = self.authority.log().last().expect("just committed");
                    let mut line = serde_json::to_string(entry).expect("entries always serialize");
                    line.push('\n');
                    j.write_all(line.as_bytes())
                        .and_then(|()| j.sync_data())
                        .map_err(|e| {
                            RpcError::new(
                                INTERNAL_ERROR,
                                format!(
                                    "command committed at revision {} but the journal write failed: {e}",
                                    out.revision.0
                                ),
                            )
                        })?;
                }
                Ok(serde_json::to_value(out).expect("outcomes always serialize"))
            }
            "query" => {
                let q: Query = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(e.to_string()))?;
                let res = self
                    .authority
                    .query(&q)
                    .map_err(|d| RpcError::from_diagnostic(&d))?;
                Ok(serde_json::to_value(res).expect("results always serialize"))
            }
            "document.save" => {
                let req: crate::save::SaveRequest = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(e.to_string()))?;
                let res = crate::save::save_dcad(
                    &self.authority,
                    &self.extras,
                    self.export_dir.as_deref(),
                    &req,
                )
                .map_err(|d| RpcError::from_diagnostic(&d))?;
                Ok(serde_json::to_value(res).expect("results always serialize"))
            }
            "render.view" => {
                let req: crate::view::RenderRequest = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(e.to_string()))?;
                let res =
                    crate::view::render_view(&self.authority, self.export_dir.as_deref(), &req)
                        .map_err(|d| RpcError::from_diagnostic(&d))?;
                Ok(serde_json::to_value(res).expect("results always serialize"))
            }
            "export.stl" => {
                let req: crate::export::ExportStlRequest = serde_json::from_value(params)
                    .map_err(|e| RpcError::invalid_params(e.to_string()))?;
                let res =
                    crate::export::export_stl(&self.authority, self.export_dir.as_deref(), &req)
                        .map_err(|d| RpcError::from_diagnostic(&d))?;
                Ok(serde_json::to_value(res).expect("results always serialize"))
            }
            "schema" => Ok(schema::all()),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("unknown method '{other}'"),
            )),
        }
    }

    /// Handles one line of JSON-RPC 2.0. Returns the response line, or `None` for notifications.
    pub fn handle_jsonrpc(&mut self, line: &str) -> Option<String> {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(response_err(
                    Value::Null,
                    &RpcError::new(PARSE_ERROR, e.to_string()),
                ))
            }
        };
        let id = v.get("id").cloned();
        let Some(method) = v.get("method").and_then(Value::as_str) else {
            return id
                .map(|id| response_err(id, &RpcError::new(INVALID_REQUEST, "missing 'method'")));
        };
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let result = self.call(method, params);
        let id = id?; // notification: no response
        Some(match result {
            Ok(r) => response_ok(id, r),
            Err(e) => response_err(id, &e),
        })
    }

    /// Handles one line of the Model Context Protocol (JSON-RPC based; stdio transport).
    pub fn handle_mcp(&mut self, line: &str) -> Option<String> {
        crate::mcp::handle(self, line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn human() -> Session {
        Session::new(Actor::Human("ann".into()))
    }

    fn rpc(s: &mut Session, id: u64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        serde_json::from_str(&s.handle_jsonrpc(&req.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn submit_then_query() {
        let mut s = human();
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"depth","value":10}),
        );
        assert_eq!(r["result"]["revision"], 1);
        assert_eq!(r["result"]["changes"][0]["after"], 10.0);
        let q = rpc(&mut s, 2, "query", json!({"type":"param","name":"depth"}));
        assert_eq!(q["result"]["value"], 10.0);
        let q = rpc(&mut s, 3, "query", json!({"type":"summary"}));
        assert_eq!(q["result"]["param_count"], 1);
    }

    #[test]
    fn rejection_carries_structured_diagnostic() {
        let mut s = human();
        rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"d","value":10}),
        );
        let r = rpc(
            &mut s,
            2,
            "command.submit",
            json!({"type":"param.set","name":"d","value":12,"expected":{"value":99}}),
        );
        assert_eq!(r["error"]["code"], COMMAND_REJECTED);
        assert_eq!(r["error"]["data"]["code"], "stale_expectation");
        assert_eq!(r["error"]["data"]["entities"][0], "d");
        assert!(r["error"]["data"]["suggestion"].is_string());
    }

    #[test]
    fn actor_cannot_be_spoofed_or_chosen_by_client() {
        let mut s = Session::new(Actor::Agent("claude".into()));
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"x","value":1,"actor":{"kind":"human","name":"ann"}}),
        );
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        // And the agent role is the conservative default, so direct commits are refused.
        let r = rpc(
            &mut s,
            2,
            "command.submit",
            json!({"type":"param.set","name":"x","value":1}),
        );
        assert_eq!(r["error"]["data"]["code"], "proposal_required");
    }

    #[test]
    fn host_grants_role_and_attribution_is_recorded() {
        let mut s = Session::open(None, Actor::Agent("claude".into()), Some(Role::Full)).unwrap();
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"x","value":1}),
        );
        assert!(r["error"].is_null());
        let log = rpc(&mut s, 2, "query", json!({"type":"log","from":1}));
        assert_eq!(
            log["result"]["entries"][0]["command"]["actor"]["name"],
            "claude"
        );
        assert_eq!(
            log["result"]["entries"][0]["command"]["actor"]["kind"],
            "agent"
        );
        let info = rpc(&mut s, 3, "session.info", Value::Null);
        assert_eq!(info["result"]["actor"], "agent:claude");
        assert_eq!(info["result"]["role"], "full");
    }

    #[test]
    fn omitted_ids_make_retries_safe() {
        let mut s = human();
        let p = json!({"type":"param.set","name":"x","value":1});
        let first = rpc(&mut s, 1, "command.submit", p.clone());
        assert!(first["error"].is_null());
        // Same command at the same base revision: but base defaults to *current*, so this is a
        // new revision and a different derived ID, hence a legitimate second command.
        let second = rpc(&mut s, 2, "command.submit", p.clone());
        assert!(second["error"].is_null());
        // An explicit base revision makes the retry detectably identical.
        let a = rpc(
            &mut s,
            3,
            "command.submit",
            json!({"base_revision":2,"type":"param.set","name":"y","value":1}),
        );
        assert!(a["error"].is_null());
        let b = rpc(
            &mut s,
            4,
            "command.submit",
            json!({"base_revision":2,"type":"param.set","name":"y","value":1}),
        );
        assert_eq!(b["error"]["data"]["code"], "duplicate_command");
    }

    #[test]
    fn dry_run_changes_nothing() {
        let mut s = human();
        let d = rpc(
            &mut s,
            1,
            "command.dry_run",
            json!({"type":"param.set","name":"x","value":5}),
        );
        assert_eq!(d["result"]["revision"], 1);
        let q = rpc(&mut s, 2, "query", json!({"type":"summary"}));
        assert_eq!(q["result"]["revision"], 0);
        assert_eq!(q["result"]["param_count"], 0);
    }

    #[test]
    fn jsonrpc_edge_cases() {
        let mut s = human();
        let p: Value = serde_json::from_str(&s.handle_jsonrpc("{not json").unwrap()).unwrap();
        assert_eq!(p["error"]["code"], PARSE_ERROR);
        assert!(
            s.handle_jsonrpc(r#"{"jsonrpc":"2.0","method":"session.info"}"#)
                .is_none(),
            "notification"
        );
        let m = rpc(&mut s, 1, "no.such.method", Value::Null);
        assert_eq!(m["error"]["code"], METHOD_NOT_FOUND);
        let bad = rpc(&mut s, 2, "command.submit", json!({"type":"param.set"}));
        assert_eq!(bad["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn schema_is_generated_from_types() {
        let mut s = human();
        let r = rpc(&mut s, 1, "schema", Value::Null);
        let text = r["result"].to_string();
        for needle in [
            "param.set",
            "param.delete",
            "transaction",
            "stale_expectation",
            "state_hash",
        ] {
            assert!(text.contains(needle), "schema should mention {needle}");
        }
        assert!(!r["result"]["command_request"]["properties"]
            .as_object()
            .is_some_and(|p| p.contains_key("actor")));
    }

    #[test]
    fn journal_persists_and_reopens_with_verified_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.jsonl");
        let hash_after;
        {
            let mut s = Session::open(Some(&path), Actor::Human("ann".into()), None).unwrap();
            for i in 0..5 {
                let r = rpc(
                    &mut s,
                    i,
                    "command.submit",
                    json!({"type":"param.set","name":format!("p{i}"),"value":i}),
                );
                assert!(r["error"].is_null());
            }
            hash_after = s.authority().state_hash();
        }
        let mut s = Session::open(Some(&path), Actor::Human("ann".into()), None).unwrap();
        assert_eq!(s.authority().state_hash(), hash_after);
        assert_eq!(s.authority().revision(), Revision(5));
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"p9","value":9}),
        );
        assert_eq!(r["result"]["revision"], 6);
        drop(s);
        // Corrupting the file is detected on open, not silently accepted.
        let text = fs::read_to_string(&path)
            .unwrap()
            .replacen("\"value\":2.0", "\"value\":2.5", 1);
        fs::write(&path, text).unwrap();
        let err = Session::open(Some(&path), Actor::Human("ann".into()), None)
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("hash mismatch"), "{err}");
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    fn rpc(s: &mut Session, id: u64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        serde_json::from_str(&s.handle_jsonrpc(&req.to_string()).unwrap()).unwrap()
    }

    /// A session with a 30 x 20 rectangle extruded 10 mm as feature `e1`.
    fn with_box(dir: &Path) -> Session {
        let mut s = Session::new(Actor::Human("ann".into()))
            .with_export_dir(dir)
            .unwrap();
        let pts = [
            ("a", 0.0, 0.0),
            ("b", 30.0, 0.0),
            ("c", 30.0, 20.0),
            ("d", 0.0, 20.0),
        ];
        let mut ops = vec![json!({"type":"sketch.create","sketch":"s"})];
        for (id, x, y) in pts {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":id,"entity":{"type":"point","x":x,"y":y}}));
        }
        for (id, p, q) in [
            ("ab", "a", "b"),
            ("bc", "b", "c"),
            ("cd", "c", "d"),
            ("da", "d", "a"),
        ] {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":id,"entity":{"type":"line","a":p,"b":q}}));
        }
        ops.push(json!({"type":"feature.extrude","feature":"e1","sketch":"s","profiles":["profile.ab"],"depth":10}));
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"transaction","label":"box","ops":ops}),
        );
        assert!(r["error"].is_null(), "{r}");
        s
    }

    #[test]
    fn exports_a_watertight_stl_into_the_approved_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_box(dir.path());
        let r = rpc(
            &mut s,
            2,
            "export.stl",
            json!({"feature":"e1","file_name":"box.stl"}),
        );
        assert!(r["error"].is_null(), "{r}");
        let res = &r["result"];
        assert_eq!(res["triangles"], 12);
        assert_eq!(res["bytes"], 84 + 50 * 12);
        assert_eq!(res["exact_volume"], 6000.0);
        assert_eq!(res["mesh_volume"], 6000.0, "a planar solid meshes exactly");
        let file = dir.path().join("box.stl");
        let bytes = fs::read(&file).unwrap();
        assert_eq!(bytes.len(), 84 + 50 * 12);
        assert_eq!(
            dc_doc::Hash128::of(&[&bytes]).to_hex(),
            res["digest"].as_str().unwrap()
        );
        assert!(
            !dir.path().join(".box.stl.tmp").exists(),
            "temp file cleaned up"
        );

        // Re-exporting after an edit overwrites with the new geometry.
        let e = rpc(
            &mut s,
            3,
            "command.submit",
            json!({"type":"feature.set_depth","feature":"e1","depth":20}),
        );
        assert!(e["error"].is_null(), "{e}");
        let r = rpc(
            &mut s,
            4,
            "export.stl",
            json!({"feature":"e1","file_name":"box.stl"}),
        );
        assert_eq!(r["result"]["exact_volume"], 12000.0);
        assert_ne!(fs::read(&file).unwrap(), bytes);

        // ASCII is available too.
        let r = rpc(
            &mut s,
            5,
            "export.stl",
            json!({"feature":"e1","file_name":"box-ascii.stl","format":"ascii"}),
        );
        assert!(r["error"].is_null(), "{r}");
        let text = fs::read_to_string(dir.path().join("box-ascii.stl")).unwrap();
        assert!(text.starts_with("solid box-ascii\n"));
        assert_eq!(text.matches("facet normal").count(), 12);
    }

    #[test]
    fn a_client_cannot_choose_where_files_go() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut s = with_box(dir.path());
        let evil = [
            "../escape.stl".to_string(),
            format!("{}/abs.stl", outside.path().display()),
            "sub/dir.stl".to_string(),
            "..\\win.stl".to_string(),
            ".hidden.stl".to_string(),
            "no-extension".to_string(),
            "wrong.txt".to_string(),
            "sp ace.stl".to_string(),
            ".stl".to_string(),
            String::new(),
            format!("{}.stl", "x".repeat(200)),
            "nul\0.stl".to_string(),
        ];
        for (i, name) in evil.iter().enumerate() {
            let r = rpc(
                &mut s,
                i as u64 + 10,
                "export.stl",
                json!({"feature":"e1","file_name":name}),
            );
            assert_eq!(
                r["error"]["data"]["code"], "invalid_name",
                "{name:?} must be refused: {r}"
            );
        }
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing was written"
        );
        assert_eq!(
            fs::read_dir(outside.path()).unwrap().count(),
            0,
            "nothing escaped"
        );
    }

    #[test]
    fn export_is_off_unless_the_host_enables_it_and_errors_are_clear() {
        let mut off = Session::new(Actor::Human("ann".into()));
        let r = rpc(
            &mut off,
            1,
            "export.stl",
            json!({"feature":"e1","file_name":"a.stl"}),
        );
        assert_eq!(r["error"]["data"]["code"], "permission_denied");
        assert!(r["error"]["data"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("--export-dir"));

        let dir = tempfile::tempdir().unwrap();
        let mut s = with_box(dir.path());
        let r = rpc(
            &mut s,
            2,
            "export.stl",
            json!({"feature":"nope","file_name":"a.stl"}),
        );
        assert_eq!(r["error"]["data"]["code"], "not_found");
        let r = rpc(
            &mut s,
            3,
            "export.stl",
            json!({"feature":"e1","file_name":"a.stl","chord_tol":0.0}),
        );
        assert_eq!(r["error"]["data"]["code"], "invalid_value");
        let r = rpc(&mut s, 4, "export.stl", json!({"feature":"e1"}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn schema_includes_the_export_and_feature_types() {
        let mut s = Session::new(Actor::Human("ann".into()));
        let text = rpc(&mut s, 1, "schema", Value::Null)["result"].to_string();
        for needle in [
            "export_stl_request",
            "feature.extrude",
            "feature.set_depth",
            "sketch.profiles",
            "feature.faces",
            "file_name",
        ] {
            assert!(text.contains(needle), "schema should mention {needle}");
        }
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;

    fn rpc(s: &mut Session, id: u64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        serde_json::from_str(&s.handle_jsonrpc(&req.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn save_then_open_restores_everything_including_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new(Actor::Human("ann".into()))
            .with_export_dir(dir.path())
            .unwrap();
        for (i, v) in [1.0, 2.0, 3.0].iter().enumerate() {
            rpc(
                &mut s,
                i as u64,
                "command.submit",
                json!({"type":"param.set","name":format!("p{i}"),"value":v}),
            );
        }
        rpc(&mut s, 10, "command.submit", json!({"type":"history.undo"}));
        let r = rpc(
            &mut s,
            11,
            "document.save",
            json!({"file_name":"design.dcad"}),
        );
        assert!(r["error"].is_null(), "{r}");
        let res = &r["result"];
        assert_eq!(res["revision"], 4);
        let path = dir.path().join("design.dcad");
        let bytes = fs::read(&path).unwrap();
        assert_eq!(res["bytes"].as_u64().unwrap() as usize, bytes.len());
        assert_eq!(
            dc_doc::Hash128::of(&[&bytes]).to_hex(),
            res["digest"].as_str().unwrap()
        );
        assert!(!dir.path().join(".design.dcad.tmp").exists());

        let mut t = Session::from_dcad(&path, Actor::Human("ann".into()), None).unwrap();
        assert_eq!(t.authority().state_hash(), s.authority().state_hash());
        let q = rpc(&mut t, 1, "query", json!({"type":"param","name":"p2"}));
        assert_eq!(
            q["result"]["value"],
            Value::Null,
            "the undo was part of the saved history"
        );
        let r = rpc(&mut t, 2, "command.submit", json!({"type":"history.redo"}));
        assert!(r["error"].is_null(), "undo history survives the file: {r}");
        assert_eq!(
            rpc(&mut t, 3, "query", json!({"type":"param","name":"p2"}))["result"]["value"],
            3.0
        );
    }

    #[test]
    fn saving_follows_the_same_host_rules_as_export() {
        let mut off = Session::new(Actor::Human("ann".into()));
        let r = rpc(&mut off, 1, "document.save", json!({"file_name":"a.dcad"}));
        assert_eq!(r["error"]["data"]["code"], "permission_denied");

        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new(Actor::Human("ann".into()))
            .with_export_dir(dir.path())
            .unwrap();
        for (i, name) in [
            "../a.dcad",
            "a/b.dcad",
            "a.stl",
            "a.dcad.exe",
            ".x.dcad",
            ".dcad",
            "",
            "sp ace.dcad",
        ]
        .iter()
        .enumerate()
        {
            let r = rpc(&mut s, i as u64, "document.save", json!({"file_name":name}));
            assert_eq!(r["error"]["data"]["code"], "invalid_name", "{name:?}: {r}");
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        // Saving is allowed for read-only actors: it writes only into the host-approved folder
        // and changes nothing in the document.
        let mut ro = Session::open(None, Actor::Human("viewer".into()), Some(Role::ReadOnly))
            .unwrap()
            .with_export_dir(dir.path())
            .unwrap();
        assert!(rpc(&mut ro, 1, "document.save", json!({"file_name":"v.dcad"}))["error"].is_null());
    }

    #[test]
    fn a_damaged_file_is_refused_with_a_readable_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new(Actor::Human("ann".into()))
            .with_export_dir(dir.path())
            .unwrap();
        rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"param.set","name":"x","value":1}),
        );
        rpc(&mut s, 2, "document.save", json!({"file_name":"d.dcad"}));
        let path = dir.path().join("d.dcad");
        let mut bytes = fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        fs::write(&path, &bytes).unwrap();
        let err = Session::from_dcad(&path, Actor::Human("ann".into()), None)
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("d.dcad"), "{err}");
        assert!(Session::from_dcad(
            &dir.path().join("missing.dcad"),
            Actor::Human("a".into()),
            None
        )
        .is_err());
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    fn rpc(s: &mut Session, id: u64, method: &str, params: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        serde_json::from_str(&s.handle_jsonrpc(&req.to_string()).unwrap()).unwrap()
    }

    fn with_box() -> Session {
        let mut s = Session::new(Actor::Human("ann".into()));
        let mut ops = vec![json!({"type":"sketch.create","sketch":"s"})];
        for (i, (x, y)) in [(0.0, 0.0), (80.0, 0.0), (80.0, 40.0), (0.0, 40.0)]
            .iter()
            .enumerate()
        {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":format!("p{i}"),"entity":{"type":"point","x":x,"y":y}}));
        }
        for i in 0..4 {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":format!("e{i}"),"entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i + 1) % 4)}}));
        }
        ops.push(json!({"type":"feature.extrude","feature":"box","sketch":"s","profiles":["profile.e0"],"depth":20}));
        let r = rpc(
            &mut s,
            1,
            "command.submit",
            json!({"type":"transaction","label":"box","ops":ops}),
        );
        assert!(r["error"].is_null(), "{r}");
        s
    }

    fn png_dims(b64: &str) -> (u32, u32) {
        let png = dc_render::b64::decode(b64).expect("valid base64");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn renders_a_png_with_the_camera_it_used() {
        let mut s = with_box();
        let r = rpc(
            &mut s,
            2,
            "render.view",
            json!({"width":320,"height":200,"view":"front","projection":"orthographic"}),
        );
        assert!(r["error"].is_null(), "{r}");
        let res = &r["result"];
        assert_eq!(png_dims(res["png_base64"].as_str().unwrap()), (320, 200));
        assert_eq!(res["features"], json!(["box"]));
        assert_eq!(res["camera"]["projection"], "orthographic");
        assert_eq!(res["camera"]["yaw_deg"], -90.0, "front view looks along +y");
        // Fitted to the part: the target is the middle of the box.
        let t = &res["camera"]["target"];
        assert!(
            (t[0].as_f64().unwrap() - 40.0).abs() < 1e-9
                && (t[2].as_f64().unwrap() - 10.0).abs() < 1e-9,
            "{t}"
        );
        assert_eq!(res["legend"], json!([]), "no legend in shaded mode");
    }

    #[test]
    fn face_id_mode_returns_a_legend_naming_every_face() {
        let mut s = with_box();
        let r = rpc(
            &mut s,
            2,
            "render.view",
            json!({"mode":"face_ids","view":"top"}),
        );
        let legend = r["result"]["legend"].as_array().unwrap();
        let faces: Vec<&str> = legend.iter().map(|l| l["face"].as_str().unwrap()).collect();
        for f in [
            "cap_end",
            "cap_start",
            "side:e0",
            "side:e1",
            "side:e2",
            "side:e3",
        ] {
            assert!(faces.contains(&f), "{f} in {faces:?}");
        }
        assert!(legend
            .iter()
            .all(|l| l["feature"] == "box" && l["color"].as_str().unwrap().len() == 7));
        // The pixels agree with the legend: decode the PNG's top view and check its centre colour is cap_end.
        // (Verified at pixel level in dc-render; here we only check the colours are distinct.)
        let mut colors: Vec<&str> = legend
            .iter()
            .map(|l| l["color"].as_str().unwrap())
            .collect();
        colors.sort();
        colors.dedup();
        assert_eq!(colors.len(), 6);
    }

    #[test]
    fn an_explicit_camera_is_used_exactly() {
        let mut s = with_box();
        let cam = json!({"target":[1.0,2.0,3.0],"distance":150.0,"yaw_deg":30.0,"pitch_deg":20.0});
        let r = rpc(
            &mut s,
            2,
            "render.view",
            json!({"camera":cam,"projection":"perspective"}),
        );
        let c = &r["result"]["camera"];
        assert_eq!(
            (c["target"].clone(), c["distance"].as_f64().unwrap()),
            (json!([1.0, 2.0, 3.0]), 150.0)
        );
        assert!(
            (c["yaw_deg"].as_f64().unwrap() - 30.0).abs() < 1e-9
                && (c["pitch_deg"].as_f64().unwrap() - 20.0).abs() < 1e-9
        );
        for bad in [
            json!({"target":[0,0,0],"distance":-1,"yaw_deg":0,"pitch_deg":0}),
            json!({"target":[0,0,0],"distance":0,"yaw_deg":0,"pitch_deg":0}),
        ] {
            let r = rpc(&mut s, 3, "render.view", json!({"camera":bad}));
            assert_eq!(r["error"]["data"]["code"], "invalid_value");
        }
    }

    #[test]
    fn size_limits_and_empty_documents_are_handled() {
        let mut s = with_box();
        for (w, h) in [(0, 100), (100, 5), (5000, 100), (100, 5000)] {
            let r = rpc(&mut s, 2, "render.view", json!({"width":w,"height":h}));
            assert_eq!(r["error"]["data"]["code"], "invalid_value", "{w}x{h}");
        }
        let mut empty = Session::new(Actor::Human("ann".into()));
        let r = rpc(
            &mut empty,
            1,
            "render.view",
            json!({"width":64,"height":48}),
        );
        assert!(
            r["error"].is_null(),
            "an empty document still renders (just a grid): {r}"
        );
        assert_eq!(r["result"]["features"], json!([]));
    }

    #[test]
    fn saving_the_image_follows_the_host_directory_rules() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = with_box().with_export_dir(dir.path()).unwrap();
        let r = rpc(
            &mut s,
            2,
            "render.view",
            json!({"width":200,"height":150,"save_as":"iso.png"}),
        );
        assert!(r["error"].is_null(), "{r}");
        let on_disk = fs::read(dir.path().join("iso.png")).unwrap();
        assert_eq!(on_disk.len() as u64, r["result"]["bytes"].as_u64().unwrap());
        assert_eq!(
            dc_render::b64::encode(&on_disk),
            r["result"]["png_base64"].as_str().unwrap(),
            "the file is exactly the returned image"
        );
        for bad in ["../x.png", "a/b.png", "x.stl", ".h.png", "x.png.exe"] {
            let r = rpc(&mut s, 3, "render.view", json!({"save_as":bad}));
            assert_eq!(r["error"]["data"]["code"], "invalid_name", "{bad}");
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        let mut off = with_box();
        let r = rpc(&mut off, 2, "render.view", json!({"save_as":"a.png"}));
        assert_eq!(r["error"]["data"]["code"], "permission_denied");
    }

    #[test]
    fn highlighting_changes_the_image_and_rendering_is_stable() {
        let mut s = with_box();
        let plain = rpc(&mut s, 2, "render.view", json!({}))["result"]["png_base64"].clone();
        let again = rpc(&mut s, 3, "render.view", json!({}))["result"]["png_base64"].clone();
        assert_eq!(plain, again, "same request, same bytes");
        let lit = rpc(
            &mut s,
            4,
            "render.view",
            json!({"highlight":{"feature":"box","face":"cap_end"}}),
        )["result"]["png_base64"]
            .clone();
        assert_ne!(plain, lit);
    }
}
