//! Minimal Model Context Protocol server (stdio, newline-delimited JSON-RPC). Exposes the same
//! methods as the native protocol as MCP tools.

use crate::rpc::*;
use crate::schema;
use crate::session::{CommandRequest, Session, PROTOCOL_VERSION};
use dc_doc::Query;
use serde_json::{json, Value};

const DEFAULT_MCP_VERSION: &str = "2024-11-05";

const INSTRUCTIONS: &str = "\
Design Computer: a parametric CAD document you view and edit through commands and queries. \
You are a first-class actor: you see exactly what the human sees, and every change you make is \
recorded with your identity. Workflow: call session_info, then query {type:summary}; use \
dry_run_command to preview a change, then submit_command. Use `expected` preconditions so a \
concurrent edit is detected instead of overwritten. Failures return a structured diagnostic \
(code, entities, suggestion). All queries are paginated and bounded; start with summaries.\n\n\
Sketches: create with sketch.create, add points first and then lines/circles/arcs that reference \
them by ID, then constraints (horizontal, length, fixed, tangent ...). Every change re-solves the \
sketch; a command that would make constraints contradictory is rejected with code `unsolvable` \
naming the culprit, so committed sketches are always solved. Read `sketch.summary` for degrees of \
freedom and which entities are still free (`under_constrained`). Use sketch.set_value to change a \
dimension and sketch.drag_point / sketch.drag to move geometry. Item IDs go in `entity_id` / `constraint_id`; \
`id` is the command's own idempotency key. Group related edits in a `transaction`.\n\n\
Cuts and bosses: feature.cut removes a sketch profile from an existing feature, from the sketch \
plane out to `depth` (omit it for through-all); feature.boss adds a profile out to `depth`. Both work on \
parts built along one axis: through-holes, blind pockets, counterbores, steps and stacked bosses. \
Set `reverse` to go against the sketch normal (a cut from a part's top face goes into the part with \
reverse true). A sketch on a plane that is not parallel to the part, or boundaries that merely touch \
at a point, are refused with code `unsupported` and a reason.\n\n\
Sketching on faces: create a sketch with plane {base:\"face\", feature, face} (add `offset` to move \
it along the face normal). Face names come from query feature.faces and are stable: cap_start / \
cap_end are the base's ends, cap_end:<feature> is the top a cut or boss introduced (a pocket floor, \
a boss top), side:<entity> are walls. A face-attached sketch follows its face when the part changes, \
and everything is rebuilt in dependency order. An edit that would remove a face a sketch is on is \
rejected naming the sketch. Looking at a face from outside, its sketch normal points out of the part.\n\n\
Looking: render_view returns an image (iso by default) so you can see what the person sees; use mode face_ids to map pixels to face names. Check your work visually after non-trivial edits.\n\n\
Solids: feature.extrude turns a closed sketch profile (see query sketch.profiles) into a solid; \
sketch edits rebuild it automatically, and an edit that would break it is rejected naming the \
feature. Read feature.summary for exact volume/area/bounds and feature.faces for stable face \
names. export_stl writes the solid to the host-approved folder; save_document saves the whole design as a .dcad file there. Undo with history.undo / history.redo (yours only; refused if later work touched the same things).";

fn tools() -> Value {
    json!([
        {
            "name": "session_info",
            "description": "Who you are in this session (actor), your role (full / propose_only / read_only), the current revision and state hash. Call first.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "query",
            "description": "Read the document. Types: summary (revision, counts, state hash; start here), param, params, sketches (list with DOF), sketch.summary (full solver analysis: dof, under_constrained, redundant, conflicting), sketch.entities, sketch.constraints, log (history with actor attribution). Lists are paginated: pass `next` back as `after`. All results are bounded.",
            "inputSchema": schema::of::<Query>()
        },
        {
            "name": "dry_run_command",
            "description": "Preview a command: returns exactly what submit_command would change and the resulting revision, without committing anything. Rejections return the same diagnostic as a real submit.",
            "inputSchema": schema::of::<CommandRequest>()
        },
        {
            "name": "render_view",
            "description": "See the design: returns an image of what the viewport shows, plus the camera used. Choose a standard `view` (iso, front, back, left, right, top, bottom; fitted to the part) or give an exact `camera`. mode `face_ids` paints every face a distinct flat colour and returns a `legend` mapping colours to feature and face names: use it to find which face is at a pixel, or to learn face names. `highlight` marks one face in orange. Sketches are drawn as teal outlines. Optionally `save_as` a .png in the host's output folder.",
            "inputSchema": schema::of::<crate::view::RenderRequest>()
        },
        {
            "name": "save_document",
            "description": "Save the whole design as a native .dcad file (a zip with the full command history) in the host-approved folder. Give only a plain file name ending in .dcad. Everything (sketches, features, undo history) is restored when the file is opened. Refused if the host did not enable an output folder.",
            "inputSchema": schema::of::<crate::save::SaveRequest>()
        },
        {
            "name": "export_stl",
            "description": "Write a feature's solid to an STL file in the directory the host approved (you cannot choose the location; give only a plain file name ending in .stl). The mesh is checked to be watertight first. Returns the path, triangle count, a digest of the bytes, and the mesh vs exact volume so you can see the approximation error. Refused if the host did not enable export.",
            "inputSchema": schema::of::<crate::export::ExportStlRequest>()
        },
        {
            "name": "submit_command",
            "description": "Apply a command (or an atomic `transaction` of several). It is attributed to you in the history. Add `expected` to guard against stale values. If your role is propose_only the command is refused with code proposal_required.",
            "inputSchema": schema::of::<CommandRequest>()
        }
    ])
}

fn text_result(value: &Value, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(value).expect("json")}],
        "isError": is_error
    })
}

pub(crate) fn handle(session: &mut Session, line: &str) -> Option<String> {
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
    let method = v.get("method").and_then(Value::as_str).unwrap_or("");
    let params = v.get("params").cloned().unwrap_or(Value::Null);

    let result: Result<Value, RpcError> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_MCP_VERSION),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "design-computer", "version": PROTOCOL_VERSION},
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = match params.get("arguments") {
                Some(a) if !a.is_null() => a.clone(),
                _ => json!({}),
            };
            let target = match name {
                "session_info" => "session.info",
                "query" => "query",
                "dry_run_command" => "command.dry_run",
                "submit_command" => "command.submit",
                "export_stl" => "export.stl",
                "save_document" => "document.save",
                "render_view" => "render.view",
                other => {
                    return id.map(|id| {
                        response_err(
                            id,
                            &RpcError::invalid_params(format!("unknown tool '{other}'")),
                        )
                    })
                }
            };
            // Tool failures are reported *inside* the result (isError) so the model can read and
            // act on the diagnostic, as MCP intends.
            Ok(match session.call(target, args) {
                Ok(mut r) if name == "render_view" => {
                    // The picture goes in an image block (what a vision model actually looks at);
                    // the legend and camera go alongside as text, without repeating the pixels.
                    let png = r["png_base64"].take();
                    r["png_base64"] = json!("(sent as the image above)");
                    json!({
                        "content": [
                            {"type": "image", "data": png, "mimeType": "image/png"},
                            {"type": "text", "text": serde_json::to_string_pretty(&r).expect("json")}
                        ],
                        "isError": false
                    })
                }
                Ok(r) => text_result(&r, false),
                Err(e) => text_result(&e.data.clone().unwrap_or_else(|| e.to_value()), true),
            })
        }
        m if m.starts_with("notifications/") => return None,
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("unknown method '{other}'"),
        )),
    };

    let id = id?;
    Some(match result {
        Ok(r) => response_ok(id, r),
        Err(e) => response_err(id, &e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_doc::{Actor, Role};

    fn send(s: &mut Session, req: Value) -> Value {
        serde_json::from_str(&s.handle_mcp(&req.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn handshake_and_tool_listing() {
        let mut s = Session::new(Actor::Agent("claude".into()));
        let init = send(
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(s
            .handle_mcp(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
        let list = send(
            &mut s,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "session_info",
                "query",
                "dry_run_command",
                "render_view",
                "save_document",
                "export_stl",
                "submit_command"
            ]
        );
        for t in list["result"]["tools"].as_array().unwrap() {
            assert_eq!(
                t["inputSchema"]["type"], "object",
                "{} needs an object schema",
                t["name"]
            );
        }
    }

    #[test]
    fn agent_workflow_including_diagnostic_it_can_read() {
        let mut s = Session::open(None, Actor::Agent("claude".into()), Some(Role::Full)).unwrap();
        let call = |s: &mut Session, id: u64, name: &str, args: Value| {
            send(
                s,
                json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}),
            )
        };
        let ok = call(
            &mut s,
            1,
            "submit_command",
            json!({"type":"param.set","name":"depth","value":10}),
        );
        assert_eq!(ok["result"]["isError"], false);
        let bad = call(
            &mut s,
            2,
            "submit_command",
            json!({"type":"param.set","name":"depth","value":12,"expected":{"value":3}}),
        );
        assert_eq!(bad["result"]["isError"], true);
        let text = bad["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("stale_expectation") && text.contains("suggestion"),
            "{text}"
        );
        let unknown = call(&mut s, 3, "does_not_exist", json!({}));
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn default_agent_gets_actionable_refusal() {
        let mut s = Session::new(Actor::Agent("claude".into()));
        let r = send(
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_command","arguments":{"type":"param.set","name":"x","value":1}}}),
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("proposal_required"));
    }
}

#[cfg(test)]
mod render_tool_tests {
    use super::*;
    use dc_doc::{Actor, Role};

    #[test]
    fn render_view_returns_a_real_image_block_and_a_text_block_with_the_legend() {
        let mut s = Session::open(None, Actor::Agent("claude".into()), Some(Role::Full)).unwrap();
        let call = |s: &mut Session, id: u64, name: &str, args: Value| -> Value {
            serde_json::from_str(&s.handle_mcp(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string()).unwrap()).unwrap()
        };
        let mut ops = vec![json!({"type":"sketch.create","sketch":"s"})];
        for (i, (x, y)) in [(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)]
            .iter()
            .enumerate()
        {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":format!("p{i}"),"entity":{"type":"point","x":x,"y":y}}));
        }
        for i in 0..4 {
            ops.push(json!({"type":"sketch.add_entity","sketch":"s","entity_id":format!("e{i}"),"entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i + 1) % 4)}}));
        }
        ops.push(json!({"type":"feature.extrude","feature":"b","sketch":"s","profiles":["profile.e0"],"depth":10}));
        assert_eq!(
            call(
                &mut s,
                1,
                "submit_command",
                json!({"type":"transaction","label":"b","ops":ops})
            )["result"]["isError"],
            false
        );

        let r = call(
            &mut s,
            2,
            "render_view",
            json!({"width":160,"height":120,"mode":"face_ids"}),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        let content = r["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(
            (content[0]["type"].as_str(), content[0]["mimeType"].as_str()),
            (Some("image"), Some("image/png"))
        );
        let png = dc_render::b64::decode(content[0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let meta: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(meta["legend"].as_array().unwrap().len(), 6);
        assert!(
            !content[1]["text"]
                .as_str()
                .unwrap()
                .contains(content[0]["data"].as_str().unwrap()),
            "pixels are not duplicated into the text"
        );
        // Errors are still readable text.
        let bad = call(&mut s, 3, "render_view", json!({"width":1}));
        assert_eq!(bad["result"]["isError"], true);
        assert!(bad["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("out of range"));
    }
}
