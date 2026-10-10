use crate::{Command, Diagnostic, Hash128, Revision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// One line of `history.jsonl`: a committed command, the revision it produced, and the state hash
/// after it. Replicas and replays verify the hash to detect divergence immediately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogEntry {
    pub revision: Revision,
    pub command: Command,
    pub state_hash: Hash128,
}

#[derive(Debug)]
pub enum ReplayError {
    Parse {
        line: usize,
        message: String,
    },
    Revision {
        line: usize,
        expected: Revision,
        found: Revision,
    },
    Apply {
        line: usize,
        diagnostic: Diagnostic,
    },
    HashMismatch {
        revision: Revision,
        expected: Hash128,
        actual: Hash128,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Parse { line, message } => {
                write!(f, "line {line}: parse error: {message}")
            }
            ReplayError::Revision {
                line,
                expected,
                found,
            } => {
                write!(
                    f,
                    "line {line}: expected revision {}, found {}",
                    expected.0, found.0
                )
            }
            ReplayError::Apply { line, diagnostic } => write!(f, "line {line}: {diagnostic}"),
            ReplayError::HashMismatch {
                revision,
                expected,
                actual,
            } => write!(
                f,
                "state hash mismatch at revision {}: log says {expected}, replay produced {actual}",
                revision.0
            ),
        }
    }
}

impl std::error::Error for ReplayError {}
