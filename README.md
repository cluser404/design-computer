# Design Computer

A parametric CAD system for mechanical parts, written in Rust, built so that people and AI agents work on the same design through the same interface.

**Status: alpha.** Milestone 1 (sketch → feature → export) is working. File formats and face names may still change, and there is no backward-compatibility promise.

## The idea

A design is a shared, ordered, attributed stream of intent, applied to a deterministic geometry engine. The file is that history. The GUI, scripts, AI agents and remote users are all clients of it.

- **Intent is the source of truth.** The file stores what was meant ("extrude this profile 12 mm"), not the resulting geometry. Geometry is rebuilt from it.
- **One kind of participant.** Humans, agents and scripts are *actors* that submit commands to one authority. Attribution, permissions and undo are built once.
- **Nothing is hidden.** Everything the GUI shows has a structured, queryable form with stable IDs, and errors are data with entity IDs and suggested fixes.
- **Deterministic.** The same command log gives the same model on every supported OS and CPU. Each revision carries a state hash, and replay verifies it.

Everything is written in-house: the B-rep kernel, the constraint solver and the feature engine. There is no OpenCascade and no other CAD kernel. See [`AGENTS.md`](AGENTS.md) for the full design and the decision log.

## What works today

- **Sketching:** lines, arcs, circles, rectangles, construction geometry, about 20 constraint kinds, dimensions, a live degrees-of-freedom display, trim/extend/split, and drag with live preview.
- **Features:** extrude, cut, boss, revolve (full and partial), fillet, chamfer, shell, loft and sweep (straight edges), patterns, sketch-on-face, suppress and rollback.
- **Kernel:** planes and cylinders (plus cone and torus patches for rounded rims), a general boolean for planes and cylinders, exact area, volume and bounds, and watertight tessellation.
- **Files:** native `.dcad` (a deterministic zip holding the command log), per-actor undo/redo, and STL export.
- **Agent access:** JSON-RPC and an MCP server over the same command and query layer, plus a headless renderer.
- **Desktop app:** an egui ribbon UI with a GPU viewport, an optional RealView shading mode, a design tree, property pages, and 3D selection (Ctrl-click, Ctrl-drag box, feature-first drill-down).

Known limits are tracked in [`TASKS.md`](TASKS.md). The main ones: a slanted-edge revolve (cones) is refused, two non-parallel crossing cylinders are not supported, and the older layered "stepped" model is still used for cuts along a part's axis.

## Building

You need a recent stable Rust toolchain (1.95 or newer) on Linux, macOS or Windows.

```sh
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Running

**Desktop app**

```sh
cargo run --release -p dc-app --bin designcomputer-app -- [FILE.dcad] [--demo]
```

Useful flags: `--software` (force the software rasterizer), `--screenshot OUT.png`, `--view NAME`, `--select ITEM`.

**Headless, for scripts and agents**

```sh
# JSON-RPC over stdio
cargo run -p dc-cli -- --headless

# MCP server, with options
cargo run -p dc-cli -- --mcp --actor agent:claude --role propose --log session.log --export-dir ./out
```

Options: `--actor KIND:NAME`, `--role full|propose|read`, `--log PATH`, `--export-dir DIR`, `--open FILE.dcad`.

Security notes: the actor is bound by the host, never read from requests, and roles are set only by these flags. Agents default to propose-only, so they can read but not commit until you pass `--role full`.

## Repository layout

| Crate | Role |
|---|---|
| `dc-math` | Deterministic math (`libm`-based), vectors, one tolerance policy, exact predicates |
| `dc-sketch` | 2D sketches, constraint solver, profile detection, 2D region booleans |
| `dc-kernel` | B-rep topology and geometry, validator, tessellation, boolean, blends |
| `dc-features` | Extrude, revolve, shell, loft, sweep, and the stepped prism model |
| `dc-doc` | Commands, authority, queries, features, undo/redo, state hashes, log and replay |
| `dc-io` | `.dcad` container, EXR reader |
| `dc-protocol` | JSON-RPC and MCP server, journal |
| `dc-cli` | `designcomputer` headless and MCP binary |
| `dc-render` | Software renderer, camera, picking, PNG |
| `dc-app` | The desktop app |

Dependencies point downward only. `kernel`, `sketch`, `features`, `doc` and `authority` have no UI or GPU dependencies.

Design notes for kernel algorithms are in [`docs/kernel/`](docs/kernel/).

## Testing the UI

The app is instrumented with [eguidev](https://crates.io/crates/eguidev). Smoke scripts in `smoketest/` drive the real window:

```sh
xvfb-run -a -s "-screen 0 1600x1000x24" edev smoke
```

Add `--features devtools` to attach the runtime (`.edev.toml` does this). See the skill in `.claude/skills/eguidev`.

## Contributing

Read [`AGENTS.md`](AGENTS.md) first. The core rules:

- Every mutation is a command and every visible state is queryable. The GUI never changes the document directly.
- Kernel code is deterministic: no platform `sin`/`cos`/`pow`, no FMA, and ordered collections only. Clippy enforces this.
- Unsupported geometry is refused with a reason, never approximated.
- Add tests with every kernel change, keep `TASKS.md` current, and record decisions in the decision log.

## License

MIT. See [`LICENSE`](LICENSE).
