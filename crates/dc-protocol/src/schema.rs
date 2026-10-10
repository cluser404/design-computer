//! JSON Schemas generated from the Rust types: the single source of truth for the protocol.

use dc_doc::{Diagnostic, Outcome, Query, QueryResult};
use schemars::JsonSchema;
use serde_json::{json, Value};

use crate::session::CommandRequest;

/// Schema for `T`, guaranteed to be `"type": "object"` at the top level (MCP requires it).
pub(crate) fn of<T: JsonSchema>() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(T)).expect("schemas always serialize");
    if let Some(o) = v.as_object_mut() {
        o.entry("type")
            .or_insert_with(|| Value::String("object".into()));
    }
    v
}

pub(crate) fn all() -> Value {
    json!({
        "command_request": of::<CommandRequest>(),
        "query": of::<Query>(),
        "query_result": of::<QueryResult>(),
        "outcome": of::<Outcome>(),
        "diagnostic": of::<Diagnostic>(),
        "render_request": of::<crate::view::RenderRequest>(),
        "render_result": of::<crate::view::RenderResult>(),
        "save_request": of::<crate::save::SaveRequest>(),
        "save_result": of::<crate::save::SaveResult>(),
        "export_stl_request": of::<crate::export::ExportStlRequest>(),
        "export_stl_result": of::<crate::export::ExportStlResult>(),
    })
}
