//! `designcomputer`: the engine without a window.
//!
//!   designcomputer --headless [--actor human:me] [--role full] [--log design.jsonl] [--export-dir out]
//!   designcomputer --mcp      [--actor agent:claude] [--role full] [--log design.jsonl] [--export-dir out]
//!
//! Both modes read one JSON-RPC message per line on stdin and answer on stdout (stderr is for
//! diagnostics only, so stdout stays clean for the protocol).

use dc_doc::{Actor, Role};
use dc_protocol::Session;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "\
designcomputer: headless Design Computer engine

USAGE:
  designcomputer (--headless | --mcp) [OPTIONS]

MODES:
  --headless        native JSON-RPC 2.0, one message per line on stdin/stdout
  --mcp             Model Context Protocol (tools) over stdio, for AI agents

OPTIONS:
  --actor KIND:NAME who this connection is: human|agent|script|remote (default: human:local,
                    or agent:mcp with --mcp). Clients cannot choose or change this.
  --role ROLE       full | propose | read. Default depends on actor kind: agents are
                    propose-only (their commits are refused) unless you grant `full`.
  --log PATH        append-only journal (history.jsonl). Replayed and hash-verified on start;
                    every commit is appended and synced.
  --open FILE       open a saved .dcad document (its history is replayed and verified).
                    Cannot be combined with --log.
  --export-dir DIR  folder where STL exports and saved .dcad files may be written (created if
                    missing). Without it, both are refused. Clients choose only a file name,
                    never a location. (--output-dir is an alias.)
  -h, --help        this text
";

enum Mode {
    Headless,
    Mcp,
}

struct Args {
    mode: Mode,
    actor: Actor,
    role: Option<Role>,
    log: Option<PathBuf>,
    export_dir: Option<PathBuf>,
    open: Option<PathBuf>,
}

fn parse_actor(s: &str) -> Result<Actor, String> {
    let (kind, name) = s
        .split_once(':')
        .ok_or("actor must look like KIND:NAME, e.g. agent:claude")?;
    if name.is_empty() {
        return Err("actor name must not be empty".into());
    }
    let name = name.to_owned();
    match kind {
        "human" => Ok(Actor::Human(name)),
        "agent" => Ok(Actor::Agent(name)),
        "script" => Ok(Actor::Script(name)),
        "remote" => Ok(Actor::Remote(name)),
        other => Err(format!(
            "unknown actor kind '{other}' (human|agent|script|remote)"
        )),
    }
}

fn parse_role(s: &str) -> Result<Role, String> {
    match s {
        "full" => Ok(Role::Full),
        "propose" => Ok(Role::ProposeOnly),
        "read" => Ok(Role::ReadOnly),
        other => Err(format!("unknown role '{other}' (full|propose|read)")),
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut mode = None;
    let mut actor = None;
    let mut role = None;
    let mut log = None;
    let mut export_dir = None;
    let mut open = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--headless" => mode = Some(Mode::Headless),
            "--mcp" => mode = Some(Mode::Mcp),
            "--actor" => actor = Some(parse_actor(&it.next().ok_or("--actor needs a value")?)?),
            "--role" => role = Some(parse_role(&it.next().ok_or("--role needs a value")?)?),
            "--log" => log = Some(PathBuf::from(it.next().ok_or("--log needs a value")?)),
            "--open" => open = Some(PathBuf::from(it.next().ok_or("--open needs a value")?)),
            "--export-dir" | "--output-dir" => {
                export_dir = Some(PathBuf::from(
                    it.next().ok_or("--export-dir needs a value")?,
                ))
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let mode = mode.ok_or("choose a mode: --headless or --mcp")?;
    let actor = actor.unwrap_or_else(|| match mode {
        Mode::Headless => Actor::Human("local".into()),
        Mode::Mcp => Actor::Agent("mcp".into()),
    });
    Ok(Some(Args {
        mode,
        actor,
        role,
        log,
        export_dir,
        open,
    }))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if args.open.is_some() && args.log.is_some() {
        eprintln!("error: --open and --log cannot be combined");
        return ExitCode::from(2);
    }
    let opened = match &args.open {
        Some(p) => Session::from_dcad(p, args.actor, args.role),
        None => Session::open(args.log.as_deref(), args.actor, args.role),
    };
    let mut session = match opened {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot open document: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(dir) = &args.export_dir {
        session = match session.with_export_dir(dir) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot use export directory {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        };
    }
    eprintln!(
        "designcomputer: actor={} role={:?} revision={}",
        session.actor(),
        session.role(),
        session.authority().revision().0
    );
    if session.role() != Role::Full {
        eprintln!("note: this actor cannot commit directly; pass --role full to allow it");
    }

    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match args.mode {
            Mode::Headless => session.handle_jsonrpc(&line),
            Mode::Mcp => session.handle_mcp(&line),
        };
        if let Some(r) = reply {
            if writeln!(out, "{r}").and_then(|()| out.flush()).is_err() {
                break; // client went away
            }
        }
    }
    ExitCode::SUCCESS
}
