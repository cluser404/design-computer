use dc_doc::Diagnostic;
use serde_json::{json, Value};

/// JSON-RPC error. Command failures carry the full [`Diagnostic`] in `data`.
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// Application error: the command was understood but refused. `data` is a `Diagnostic`.
pub const COMMAND_REJECTED: i64 = -32000;

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }
    pub fn from_diagnostic(d: &Diagnostic) -> Self {
        Self {
            code: COMMAND_REJECTED,
            message: d.to_string(),
            data: Some(serde_json::to_value(d).expect("diagnostics always serialize")),
        }
    }
    pub(crate) fn to_value(&self) -> Value {
        match &self.data {
            Some(d) => json!({"code": self.code, "message": self.message, "data": d}),
            None => json!({"code": self.code, "message": self.message}),
        }
    }
}

pub(crate) fn response_ok(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

pub(crate) fn response_err(id: Value, e: &RpcError) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": e.to_value()}).to_string()
}
