---
name: eguidev
description: Drive and test the real Design Computer window (click ribbon buttons, pick edges in the 3D view, read app state, take screenshots) through the eguidev MCP server. Use for UI behaviour, interaction bugs and regression tests.
---

# Testing Design Computer through its UI

`edev mcp` (registered in `.mcp.json`) starts the app from `.edev.toml` with the `devtools` feature, and exposes `start`, `stop`, `restart`, `status` plus `script_api` / `script_eval`. Read `script_api` first: scripts are strict Luau and the API names are global functions (`widget`, `expect`, `fixture`, `diagnostic`, `wait_until`, `dump_text`, `root`).

Without a display (CI, this sandbox) run under Xvfb: `xvfb-run -a -s "-screen 0 1600x1000x24" edev smoke`.

## What is addressable

Widget ids are stable strings, tagged in `crates/dc-app/src/devtools.rs` and at the widget:
- `tab.features|sketch|evaluate|view`: ribbon tabs; `ribbon.<command id>`: ribbon buttons (command ids are the registry's, e.g. `ribbon.feature.cut`, `ribbon.sketch.tool.line`); `ribbon.menu.<label>` and `menu.<command id>`: drop-downs and their rows.
- `left.tab.features|properties`, `tree.<node key>` (e.g. `tree.sketch:holes`, `tree.feature:plate`, `tree.origin`), `page.ok`, `page.cancel`, `page.profile.<id>`, `dock.console|activity`, `context.<action>`.
- `viewport`: the 3D view. Click or move in it with `root():raw_pointer_move` / `raw_pointer_button` at points inside `widget("viewport"):state().rect` (this is how edges and faces are picked).
- `doc.epoch`: counts applied fixtures (fixtures wait for it, so state is never stale).

State as data: `diagnostic("app")` returns revision, state hash, `features`, `result`, `hidden`, `sketches`, `selected`, `sketching`, `draft` (the open feature page: kind, profiles, edges, problem), `messages`, `volumes` (exact, per feature) and `ui`.

## Fixtures

`fixture("empty" | "demo" | "showcase" | "plate_with_four_holes" | "shaft")` replaces the document. Add new ones in `Dev::new` and `Dev::apply_fixtures`.

## Workflow

1. Reproduce through the UI with a script (`edev eval path.luau` or `script_eval`), assert on `diagnostic("app")` and `expect(...)`, not on pixels; use `widget(id):screenshot()` only to look.
2. Every fixed interaction bug gets a script in `smoketest/` (`edev smoke` runs them all); every script must fail before the fix.
3. Anything an agent cannot reach by id is a gap: tag it (`devtools::tag`) rather than clicking by coordinates.
