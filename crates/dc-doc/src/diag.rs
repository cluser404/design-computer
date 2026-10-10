use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagCode {
    PermissionDenied,
    ProposalRequired,
    DuplicateCommand,
    FutureRevision,
    InvalidValue,
    InvalidName,
    NotFound,
    StaleExpectation,
    NestedTransaction,
    EmptyTransaction,
    AlreadyExists,
    WrongKind,
    InUse,
    /// The edit would leave a sketch with contradictory or unsolvable constraints.
    Unsolvable,
    /// A feature cannot be built, or an edit would break a feature that depends on it.
    FeatureFailed,
    /// There is no command of yours to undo or redo (or it was already undone).
    /// The request is valid but this version cannot do it yet (reported, never approximated).
    Unsupported,
    NothingToUndo,
    /// Later commands changed the same things, so undoing/redoing would overwrite them.
    UndoConflict,
    /// The rollback bar is not at the end, so nothing can be added: roll forward first.
    RolledBack,
}

/// A structured error: identical for the GUI and for agents (AGENTS.md §4.4, "errors are data").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    pub code: DiagCode,
    pub message: String,
    /// IDs of the entities the problem refers to.
    pub entities: Vec<String>,
    pub suggestion: Option<String>,
}

impl Diagnostic {
    pub fn new(code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            entities: Vec::new(),
            suggestion: None,
        }
    }
    pub fn entity(mut self, id: impl Into<String>) -> Self {
        self.entities.push(id.into());
        self
    }
    pub fn suggest(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Same spelling as the serialized code, so messages and structured data agree.
        let code = serde_json::to_value(self.code)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        write!(f, "{code}: {}", self.message)
    }
}

impl std::error::Error for Diagnostic {}
