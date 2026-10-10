//! The wire protocol: JSON-RPC 2.0 and MCP over the command/query layer (AGENTS.md §4.1, §4.4).
//!
//! One [`Session`] = one connection = one **actor**. The actor is bound by the host when the
//! session is created, never read from request payloads, so a client cannot impersonate someone
//! else or promote itself. Roles are likewise host configuration, not protocol methods.
//!
//! Both transports ([`Session::handle_jsonrpc`], [`Session::handle_mcp`]) route into the same
//! [`Session::call`], so humans' tools, scripts and agents all use identical semantics.

mod export;
mod mcp;
mod rpc;
mod save;
mod schema;
mod session;
mod view;

pub use export::{ExportStlRequest, ExportStlResult, StlFormat};
pub use rpc::RpcError;
pub use save::{SaveRequest, SaveResult};
pub use session::{CommandRequest, Session, PROTOCOL_VERSION};
pub use view::{RenderRequest, RenderResult};
