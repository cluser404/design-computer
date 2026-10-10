//! End-to-end: drive the real binary over stdio, like an agent or script would.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn run(args: &[&str], requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_designcomputer"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
    } // closing stdin ends the session
    let lines: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).expect("stdout must contain only JSON"))
        .collect();
    assert!(child.wait().unwrap().success());
    lines
}

#[test]
fn headless_roundtrip_with_persistence_across_processes() {
    let dir = std::env::temp_dir().join(format!("dc-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("d.jsonl");
    let _ = std::fs::remove_file(&log);
    let log = log.to_str().unwrap();

    let r = run(
        &["--headless", "--log", log],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"command.submit","params":{"type":"param.set","name":"width","value":80}}),
            json!({"jsonrpc":"2.0","id":2,"method":"command.submit","params":{"type":"param.set","name":"height","value":40}}),
        ],
    );
    assert_eq!(r[1]["result"]["revision"], 2);

    // A new process picks up where the last one left off.
    let r = run(
        &["--headless", "--log", log],
        &[json!({"jsonrpc":"2.0","id":1,"method":"query","params":{"type":"params"}})],
    );
    assert_eq!(
        r[0]["result"]["items"],
        json!([["height", 40.0], ["width", 80.0]])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_agent_session_over_stdio() {
    let r = run(
        &["--mcp", "--actor", "agent:claude", "--role", "full"],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"submit_command","arguments":{"type":"param.set","name":"x","value":1}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"session_info","arguments":{}}}),
        ],
    );
    assert_eq!(r.len(), 3, "the notification must not get a response");
    let info = r[2]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        info.contains("agent:claude") && info.contains("\"revision\": 1"),
        "{info}"
    );
}

#[test]
fn bad_arguments_exit_nonzero() {
    let out = Command::new(env!("CARGO_BIN_EXE_designcomputer"))
        .arg("--nope")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

fn tool(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}

fn text(reply: &Value) -> Value {
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// An agent builds a dimensioned rectangle over real MCP stdio, makes a mistake, reads the
/// diagnostic, and sees the same degrees-of-freedom data a human would.
#[test]
fn agent_builds_and_debugs_a_sketch_over_mcp() {
    let pt = |id: &str, x: f64, y: f64| json!({"type":"sketch.add_entity","sketch":"s","entity_id":id,"entity":{"type":"point","x":x,"y":y}});
    let ln = |id: &str, a: &str, b: &str| json!({"type":"sketch.add_entity","sketch":"s","entity_id":id,"entity":{"type":"line","a":a,"b":b}});
    let k = |id: &str, c: Value| json!({"type":"sketch.add_constraint","sketch":"s","constraint_id":id,"constraint":c});
    let build = json!({
        "type": "transaction", "label": "dimensioned rectangle",
        "ops": [
            {"type":"sketch.create","sketch":"s"},
            pt("p1", 0.0, 0.0), pt("p2", 70.0, 5.0), pt("p3", 75.0, 45.0), pt("p4", 3.0, 38.0),
            ln("l1","p1","p2"), ln("l2","p2","p3"), ln("l3","p3","p4"), ln("l4","p4","p1"),
            k("h1", json!({"type":"horizontal","line":"l1"})),
            k("v2", json!({"type":"vertical","line":"l2"})),
            k("h3", json!({"type":"horizontal","line":"l3"})),
            k("v4", json!({"type":"vertical","line":"l4"})),
            k("w", json!({"type":"length","line":"l1","value":80.0})),
            k("h", json!({"type":"length","line":"l2","value":40.0})),
        ]
    });
    let r = run(
        &["--mcp", "--actor", "agent:claude", "--role", "full"],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            tool(2, "submit_command", build),
            tool(3, "query", json!({"type":"sketch.summary","sketch":"s"})),
            // Mistake: contradicts the 80 mm width.
            tool(
                4,
                "submit_command",
                k(
                    "bad",
                    json!({"type":"distance","a":"p1","b":"p2","value":90.0}),
                ),
            ),
            // Fix: anchor the sketch.
            tool(
                5,
                "submit_command",
                k(
                    "anchor",
                    json!({"type":"fixed","point":"p1","x":0.0,"y":0.0}),
                ),
            ),
            tool(6, "query", json!({"type":"sketch.summary","sketch":"s"})),
            tool(
                7,
                "query",
                json!({"type":"sketch.entities","sketch":"s","limit":4}),
            ),
        ],
    );
    assert_eq!(r[1]["result"]["isError"], false, "{}", r[1]);

    // Before anchoring: well-formed but free to translate (2 DOF), and it says what is free.
    let before = text(&r[2]);
    assert_eq!(before["report"]["dof"], 2);
    assert_eq!(before["info"]["fully_constrained"], false);

    // The contradiction is refused and blames the constraint the agent just added.
    assert_eq!(r[3]["result"]["isError"], true);
    let diag = text(&r[3]);
    assert_eq!(diag["code"], "unsolvable");
    assert_eq!(diag["entities"], json!(["sketch/s/bad"]));
    assert!(diag["suggestion"].is_string());

    // After anchoring: fully constrained, and the solved geometry is readable.
    assert_eq!(r[4]["result"]["isError"], false);
    let after = text(&r[5]);
    assert_eq!(after["info"]["fully_constrained"], true);
    assert_eq!(after["report"]["dof"], 0);
    let ents = text(&r[6]);
    assert_eq!(
        ents["items"][0],
        json!(["l1", {"type":"line","a":"p1","b":"p2"}])
    );
    assert!(
        ents["next"].is_string(),
        "pagination cursor expected: {ents}"
    );
}

/// Volume of a binary STL, computed from the file bytes alone.
fn stl_volume(bytes: &[u8]) -> (usize, f64) {
    let n = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(
        bytes.len(),
        84 + 50 * n,
        "binary STL size must match its triangle count"
    );
    let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as f64;
    let mut vol = 0.0;
    for t in 0..n {
        let b = 84 + 50 * t + 12;
        let v = |k: usize| [f(b + 12 * k), f(b + 12 * k + 4), f(b + 12 * k + 8)];
        let (a, p, q) = (v(0), v(1), v(2));
        vol += (a[0] * (p[1] * q[2] - p[2] * q[1]) - a[1] * (p[0] * q[2] - p[2] * q[0])
            + a[2] * (p[0] * q[1] - p[1] * q[0]))
            / 6.0;
    }
    (n, vol)
}

/// The whole milestone, as an agent experiences it: sketch -> extrude -> STL on disk -> edit ->
/// STL again, with the file verified independently of the engine that wrote it.
#[test]
fn agent_goes_from_sketch_to_stl_and_edits_the_design() {
    let dir = std::env::temp_dir().join(format!("dc-e2e-stl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir_s = dir.to_str().unwrap();

    let pt = |id: &str, x: f64, y: f64| json!({"type":"sketch.add_entity","sketch":"plate","entity_id":id,"entity":{"type":"point","x":x,"y":y}});
    let ln = |id: &str, a: &str, b: &str| json!({"type":"sketch.add_entity","sketch":"plate","entity_id":id,"entity":{"type":"line","a":a,"b":b}});
    let kc = |id: &str, c: Value| json!({"type":"sketch.add_constraint","sketch":"plate","constraint_id":id,"constraint":c});
    let build = json!({
        "type": "transaction", "label": "plate with a hole",
        "ops": [
            {"type":"sketch.create","sketch":"plate"},
            pt("a", 0.0, 0.0), pt("b", 98.0, 1.0), pt("c", 99.0, 61.0), pt("d", 2.0, 59.0), pt("m", 48.0, 31.0),
            ln("ab","a","b"), ln("bc","b","c"), ln("cd","c","d"), ln("da","d","a"),
            {"type":"sketch.add_entity","sketch":"plate","entity_id":"hole","entity":{"type":"circle","center":"m","radius":8.0}},
            kc("h1", json!({"type":"horizontal","line":"ab"})),
            kc("v1", json!({"type":"vertical","line":"bc"})),
            kc("h2", json!({"type":"horizontal","line":"cd"})),
            kc("v2", json!({"type":"vertical","line":"da"})),
            kc("w",  json!({"type":"length","line":"ab","value":100.0})),
            kc("h",  json!({"type":"length","line":"bc","value":60.0})),
            kc("anchor", json!({"type":"fixed","point":"a","x":0.0,"y":0.0})),
            kc("r",  json!({"type":"radius","target":"hole","value":8.0})),
            kc("hx", json!({"type":"fixed","point":"m","x":50.0,"y":30.0})),
            {"type":"feature.extrude","feature":"body","sketch":"plate","profiles": ["profile.ab"],"depth":12.0},
        ]
    });
    let r = run(
        &[
            "--mcp",
            "--actor",
            "agent:claude",
            "--role",
            "full",
            "--export-dir",
            dir_s,
        ],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            tool(2, "submit_command", build),
            tool(
                3,
                "query",
                json!({"type":"feature.summary","feature":"body"}),
            ),
            tool(
                4,
                "export_stl",
                json!({"feature":"body","file_name":"plate.stl"}),
            ),
            // Edit the design: widen the plate. The solid follows; export again.
            tool(
                5,
                "submit_command",
                json!({"type":"sketch.set_value","sketch":"plate","constraint_id":"w","value":120.0,"expected":{"value":100.0}}),
            ),
            tool(
                6,
                "export_stl",
                json!({"feature":"body","file_name":"plate-wide.stl"}),
            ),
            // The agent tries to escape the export folder.
            tool(
                7,
                "export_stl",
                json!({"feature":"body","file_name":"../../etc/passwd.stl"}),
            ),
            // And to break the solid by deleting its boundary edge: refused, naming the feature.
            tool(
                8,
                "submit_command",
                json!({"type":"transaction","label":"open it","ops":[
                {"type":"sketch.remove_constraint","sketch":"plate","constraint_id":"h1"},
                {"type":"sketch.remove_constraint","sketch":"plate","constraint_id":"w"},
                {"type":"sketch.remove_entity","sketch":"plate","entity_id":"ab"}]}),
            ),
        ],
    );
    assert_eq!(r[1]["result"]["isError"], false, "{}", r[1]);

    let exact = 12.0 * (100.0 * 60.0 - std::f64::consts::PI * 64.0);
    let sum = text(&r[2]);
    let v = sum["info"]["summary"]["volume"].as_f64().unwrap();
    assert!((v - exact).abs() < 1e-6, "{v} vs {exact}");
    assert_eq!(
        sum["info"]["summary"]["bounds_max"],
        json!([100.0, 60.0, 12.0])
    );

    // First export: the file on disk is a valid binary STL with (about) the right volume.
    let e1 = text(&r[3]);
    let bytes = std::fs::read(dir.join("plate.stl")).unwrap();
    assert_eq!(e1["bytes"].as_u64().unwrap() as usize, bytes.len());
    let (tris, vol) = stl_volume(&bytes);
    assert_eq!(e1["triangles"].as_u64().unwrap() as usize, tris);
    assert!(
        (vol - exact).abs() / exact < 5e-3,
        "file volume {vol} vs exact {exact}"
    );

    // After the edit the exported solid is wider, and the first file was not disturbed.
    assert_eq!(r[4]["result"]["isError"], false, "{}", r[4]);
    let wide = std::fs::read(dir.join("plate-wide.stl")).unwrap();
    let exact_wide = 12.0 * (120.0 * 60.0 - std::f64::consts::PI * 64.0);
    let (_, vol_wide) = stl_volume(&wide);
    assert!(
        (vol_wide - exact_wide).abs() / exact_wide < 5e-3,
        "{vol_wide} vs {exact_wide}"
    );
    assert_eq!(std::fs::read(dir.join("plate.stl")).unwrap(), bytes);

    // Escape attempt: refused, nothing written outside, nothing extra inside.
    assert_eq!(r[6]["result"]["isError"], true);
    assert_eq!(text(&r[6])["code"], "invalid_name");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["plate-wide.stl", "plate.stl"]);

    // Breaking the profile is refused with a diagnostic the agent can act on.
    assert_eq!(r[7]["result"]["isError"], true);
    let d = text(&r[7]);
    assert_eq!(d["code"], "feature_failed");
    assert!(
        d["entities"]
            .as_array()
            .unwrap()
            .contains(&json!("feature/body")),
        "{d}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_needs_the_host_to_opt_in() {
    let r = run(
        &["--mcp", "--actor", "agent:claude", "--role", "full"],
        &[tool(
            1,
            "export_stl",
            json!({"feature":"x","file_name":"a.stl"}),
        )],
    );
    assert_eq!(r[0]["result"]["isError"], true);
    let d = text(&r[0]);
    assert_eq!(d["code"], "permission_denied");
    assert!(d["suggestion"].as_str().unwrap().contains("--export-dir"));
}

/// Two separate processes: the first builds and saves, the second opens the file and keeps
/// working on it, including undoing something from the first process's history.
#[test]
fn a_saved_design_opens_in_a_new_process_and_remains_editable() {
    let dir = std::env::temp_dir().join(format!("dc-e2e-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir_s = dir.to_str().unwrap();

    let first = run(
        &["--headless", "--export-dir", dir_s],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"command.submit","params":{"type":"param.set","name":"width","value":80}}),
            json!({"jsonrpc":"2.0","id":2,"method":"command.submit","params":{"type":"param.set","name":"height","value":40}}),
            json!({"jsonrpc":"2.0","id":3,"method":"document.save","params":{"file_name":"part.dcad"}}),
        ],
    );
    assert!(first[2]["error"].is_null(), "{}", first[2]);
    let file = dir.join("part.dcad");
    let saved_hash = first[2]["result"]["state_hash"].clone();

    let second = run(
        &[
            "--headless",
            "--open",
            file.to_str().unwrap(),
            "--actor",
            "human:local",
        ],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"session.info","params":null}),
            json!({"jsonrpc":"2.0","id":2,"method":"query","params":{"type":"params"}}),
            // The same actor can undo what the previous process did.
            json!({"jsonrpc":"2.0","id":3,"method":"command.submit","params":{"type":"history.undo"}}),
            json!({"jsonrpc":"2.0","id":4,"method":"query","params":{"type":"params"}}),
        ],
    );
    assert_eq!(second[0]["result"]["revision"], 2);
    assert_eq!(second[0]["result"]["state_hash"], saved_hash);
    assert_eq!(
        second[1]["result"]["items"],
        json!([["height", 40.0], ["width", 80.0]])
    );
    assert!(second[2]["error"].is_null(), "{}", second[2]);
    assert_eq!(second[3]["result"]["items"], json!([["width", 80.0]]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_a_missing_or_corrupt_file_fails_cleanly() {
    let out = Command::new(env!("CARGO_BIN_EXE_designcomputer"))
        .args(["--headless", "--open", "/nonexistent/none.dcad"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot open document"));
    let both = Command::new(env!("CARGO_BIN_EXE_designcomputer"))
        .args(["--headless", "--open", "a.dcad", "--log", "b.jsonl"])
        .output()
        .unwrap();
    assert_eq!(both.status.code(), Some(2));
}

/// An agent cuts a hole and a slot out of a plate over MCP, hits the documented limits, and the
/// exported STL (read back from disk) matches the exact result.
#[test]
fn agent_cuts_a_plate_and_is_told_clearly_what_is_not_supported_yet() {
    let dir = std::env::temp_dir().join(format!("dc-e2e-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pt = |sk: &str, id: &str, x: f64, y: f64| json!({"type":"sketch.add_entity","sketch":sk,"entity_id":id,"entity":{"type":"point","x":x,"y":y}});
    let ln = |sk: &str, id: &str, a: &str, b: &str| json!({"type":"sketch.add_entity","sketch":sk,"entity_id":id,"entity":{"type":"line","a":a,"b":b}});
    let rect = |sk: &str, x0: f64, y0: f64, x1: f64, y1: f64, plane: Value| {
        let mut ops = vec![json!({"type":"sketch.create","sketch":sk,"plane":plane})];
        let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        for (i, (x, y)) in c.iter().enumerate() {
            ops.push(pt(sk, &format!("p{i}"), *x, *y));
        }
        for i in 0..4 {
            ops.push(ln(
                sk,
                &format!("e{i}"),
                &format!("p{i}"),
                &format!("p{}", (i + 1) % 4),
            ));
        }
        ops
    };
    let mut build = rect("base", 0.0, 0.0, 100.0, 60.0, json!({"base":"xy"}));
    build.push(json!({"type":"feature.extrude","feature":"plate","sketch":"base","profiles": ["profile.e0"],"depth":10}));
    let build = json!({"type":"transaction","label":"plate","ops":build});

    let hole = json!({"type":"transaction","label":"hole tool","ops":[
        {"type":"sketch.create","sketch":"hole","plane":{"base":"xy"}},
        pt("hole","m",30.0,30.0),
        {"type":"sketch.add_entity","sketch":"hole","entity_id":"c","entity":{"type":"circle","center":"m","radius":8.0}},
        {"type":"feature.cut","feature":"drilled","target":"plate","sketch":"hole","profiles": ["profile.c"]}]});
    let mut slot_ops = rect("slot", 60.0, 20.0, 110.0, 40.0, json!({"base":"xy"}));
    slot_ops.push(json!({"type":"feature.cut","feature":"slotted","target":"drilled","sketch":"slot","profiles": ["profile.e0"]}));
    let slot = json!({"type":"transaction","label":"slot","ops":slot_ops});
    // A blind pocket (4 deep into the 10 mm plate, from the sketch's plane at the bottom): works.
    let blind = json!({"type":"transaction","label":"pocket","ops":[
        {"type":"sketch.create","sketch":"pk","plane":{"base":"xy"}},
        {"type":"sketch.add_entity","sketch":"pk","entity_id":"m","entity":{"type":"point","x":80.0,"y":50.0}},
        {"type":"sketch.add_entity","sketch":"pk","entity_id":"c","entity":{"type":"circle","center":"m","radius":5.0}},
        {"type":"feature.cut","feature":"pocket","target":"slotted","sketch":"pk","profiles": ["profile.c"],"depth":4}]});
    // A sketch on a plane the part is not built along: built by the general boolean now. This one
    // faces away from the part, so it removes nothing, and the refusal says which way to go.
    let side_sketch = json!({"type":"transaction","label":"side tool","ops":[
        {"type":"sketch.create","sketch":"side","plane":{"base":"xz"}},
        {"type":"sketch.add_entity","sketch":"side","entity_id":"m","entity":{"type":"point","x":50.0,"y":5.0}},
        {"type":"sketch.add_entity","sketch":"side","entity_id":"c","entity":{"type":"circle","center":"m","radius":3.0}},
        {"type":"feature.cut","feature":"sidehole","target":"slotted","sketch":"side","profiles": ["profile.c"]}]});

    let r = run(
        &[
            "--mcp",
            "--actor",
            "agent:claude",
            "--role",
            "full",
            "--export-dir",
            dir.to_str().unwrap(),
        ],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            tool(2, "submit_command", build),
            tool(3, "submit_command", hole),
            tool(4, "submit_command", slot),
            tool(
                5,
                "query",
                json!({"type":"feature.summary","feature":"slotted"}),
            ),
            tool(
                6,
                "export_stl",
                json!({"feature":"slotted","file_name":"cut-plate.stl"}),
            ),
            tool(7, "submit_command", blind),
            tool(
                8,
                "query",
                json!({"type":"feature.summary","feature":"pocket"}),
            ),
            tool(9, "submit_command", side_sketch),
            tool(10, "query", json!({"type":"features"})),
        ],
    );
    for i in [1, 2, 3] {
        assert_eq!(r[i]["result"]["isError"], false, "step {i}: {}", r[i]);
    }
    let exact = 10.0 * (6000.0 - std::f64::consts::PI * 64.0 - 40.0 * 20.0);
    let v = text(&r[4])["info"]["summary"]["volume"].as_f64().unwrap();
    assert!((v - exact).abs() < 1e-6, "{v} vs {exact}");

    let bytes = std::fs::read(dir.join("cut-plate.stl")).unwrap();
    let (tris, vol) = stl_volume(&bytes);
    assert!(
        tris > 100,
        "a drilled and slotted plate has many triangles, got {tris}"
    );
    assert!(
        (vol - exact).abs() / exact < 5e-3,
        "STL volume {vol} vs exact {exact}"
    );

    // The pocket worked, and removed exactly the right volume.
    assert_eq!(r[6]["result"]["isError"], false, "{}", r[6]);
    let pocket = text(&r[7])["info"]["summary"]["volume"].as_f64().unwrap();
    assert!(
        (pocket - (exact - 25.0 * std::f64::consts::PI * 4.0)).abs() < 1e-6,
        "{pocket}"
    );

    // The unsupported direction is reported as data the agent can act on.
    assert_eq!(r[8]["result"]["isError"], true);
    let d = text(&r[8]);
    assert_eq!(d["code"], "feature_failed");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("does not remove any material"),
        "{d}"
    );
    assert!(d["suggestion"].as_str().unwrap().contains("reverse"), "{d}");
    // And nothing from the refused transaction (not even its sketch) was left behind.
    let names: Vec<String> = text(&r[9])["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["drilled", "plate", "pocket", "slotted"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole point of sketch-on-face: an agent stacks features on each other's faces, then changes
/// the base, and everything above follows. The exported STL (read back from disk) proves it.
#[test]
fn agent_stacks_features_on_faces_and_the_stack_follows_the_base() {
    let dir = std::env::temp_dir().join(format!("dc-e2e-stack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rect = |sk: &str, plane: Value, x0: f64, y0: f64, x1: f64, y1: f64| {
        let mut ops = vec![json!({"type":"sketch.create","sketch":sk,"plane":plane})];
        for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
            ops.push(json!({"type":"sketch.add_entity","sketch":sk,"entity_id":format!("p{i}"),"entity":{"type":"point","x":x,"y":y}}));
        }
        for i in 0..4 {
            ops.push(json!({"type":"sketch.add_entity","sketch":sk,"entity_id":format!("e{i}"),"entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i + 1) % 4)}}));
        }
        let k = |id: &str, c: Value| json!({"type":"sketch.add_constraint","sketch":sk,"constraint_id":id,"constraint":c});
        ops.push(k("ha", json!({"type":"horizontal","line":"e0"})));
        ops.push(k("vb", json!({"type":"vertical","line":"e1"})));
        ops.push(k("hc", json!({"type":"horizontal","line":"e2"})));
        ops.push(k("vd", json!({"type":"vertical","line":"e3"})));
        ops.push(k("w", json!({"type":"length","line":"e0","value":x1 - x0})));
        ops.push(k("h", json!({"type":"length","line":"e1","value":y1 - y0})));
        ops.push(k("fix", json!({"type":"fixed","point":"p0","x":x0,"y":y0})));
        ops
    };
    let mut build = rect("base", json!({"base":"xy"}), 0.0, 0.0, 100.0, 60.0);
    build.push(json!({"type":"feature.extrude","feature":"plate","sketch":"base","profiles": ["profile.e0"],"depth":12}));
    let mut tower = rect(
        "tower",
        json!({"base":"face","feature":"plate","face":"cap_end"}),
        30.0,
        20.0,
        70.0,
        40.0,
    );
    tower.push(json!({"type":"feature.boss","feature":"boss","target":"plate","sketch":"tower","profiles": ["profile.e0"],"depth":10}));
    let pocket = json!({"type":"transaction","label":"pocket in the boss","ops":[
        {"type":"sketch.create","sketch":"pk","plane":{"base":"face","feature":"boss","face":"cap_end:boss"}},
        {"type":"sketch.add_entity","sketch":"pk","entity_id":"m","entity":{"type":"point","x":50.0,"y":30.0}},
        {"type":"sketch.add_entity","sketch":"pk","entity_id":"c","entity":{"type":"circle","center":"m","radius":5.0}},
        {"type":"feature.cut","feature":"pocket","target":"boss","sketch":"pk","profiles": ["profile.c"],"depth":4,"reverse":true}]});
    let bad_face = json!({"type":"sketch.create","sketch":"oops","plane":{"base":"face","feature":"plate","face":"cap_top"}});

    let r = run(
        &[
            "--mcp",
            "--actor",
            "agent:claude",
            "--role",
            "full",
            "--export-dir",
            dir.to_str().unwrap(),
        ],
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            tool(
                2,
                "submit_command",
                json!({"type":"transaction","label":"plate","ops":build}),
            ),
            tool(
                3,
                "query",
                json!({"type":"feature.faces","feature":"plate"}),
            ),
            tool(
                4,
                "submit_command",
                json!({"type":"transaction","label":"boss","ops":tower}),
            ),
            tool(5, "submit_command", pocket),
            tool(
                6,
                "export_stl",
                json!({"feature":"pocket","file_name":"before.stl"}),
            ),
            // Make the plate thicker, then widen its base: boss and pocket must follow both.
            tool(
                7,
                "submit_command",
                json!({"type":"feature.set_depth","feature":"plate","depth":20}),
            ),
            tool(
                8,
                "submit_command",
                json!({"type":"sketch.set_value","sketch":"base","constraint_id":"w","value":140.0,"expected":{"value":100.0}}),
            ),
            tool(9, "query", json!({"type":"sketch.summary","sketch":"pk"})),
            tool(
                10,
                "query",
                json!({"type":"feature.summary","feature":"pocket"}),
            ),
            tool(
                11,
                "export_stl",
                json!({"feature":"pocket","file_name":"after.stl"}),
            ),
            tool(12, "submit_command", bad_face),
        ],
    );
    for (i, reply) in r.iter().enumerate().take(9).skip(1) {
        assert_eq!(reply["result"]["isError"], false, "step {}: {reply}", i + 1);
    }
    // The agent can discover the face names to use.
    let faces: Vec<String> = text(&r[2])["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        faces.contains(&"cap_end".to_string()) && faces.contains(&"cap_start".to_string()),
        "{faces:?}"
    );

    let pi = std::f64::consts::PI;
    let before = 100.0 * 60.0 * 12.0 + 40.0 * 20.0 * 10.0 - 25.0 * pi * 4.0;
    let (_, vb) = stl_volume(&std::fs::read(dir.join("before.stl")).unwrap());
    assert!(
        (vb - before).abs() / before < 5e-3,
        "before: {vb} vs {before}"
    );

    // After both edits the document's exact volume is the new formula, and the pocket's sketch is
    // at the boss's new top: plate 20 + boss 10 = 30.
    let after = 140.0 * 60.0 * 20.0 + 40.0 * 20.0 * 10.0 - 25.0 * pi * 4.0;
    let exact = text(&r[9])["info"]["summary"]["volume"].as_f64().unwrap();
    assert!((exact - after).abs() < 1e-6, "{exact} vs {after}");
    assert_eq!(
        text(&r[8])["info"]["frame"]["origin"][2].as_f64().unwrap(),
        30.0
    );
    let (_, va) = stl_volume(&std::fs::read(dir.join("after.stl")).unwrap());
    assert!((va - after).abs() / after < 5e-3, "after: {va} vs {after}");

    // A wrong face name is a readable error that lists the real ones.
    assert_eq!(r[11]["result"]["isError"], true);
    let d = text(&r[11]);
    assert_eq!(d["code"], "not_found");
    assert!(d["message"].as_str().unwrap().contains("cap_end"), "{d}");
    let _ = std::fs::remove_dir_all(&dir);
}
