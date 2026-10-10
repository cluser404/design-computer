//! Document, command, query and authority skeleton (AGENTS.md §4.1, §4.2, §4.5, §4.7).
//!
//! This is the spine every client uses: GUI, scripts, agents and (later) remote collaborators all
//! submit [`Command`]s to an [`Authority`] and read state through [`Query`]s. The document content
//! here is deliberately tiny (named numeric parameters). It exists to make the *contracts*
//! real and tested before geometry arrives: actors, capabilities, base revisions, preconditions,
//! atomic transactions, dry-run, structured diagnostics, bounded queries, a deterministic log and
//! incremental state hashes.

pub mod actor;
pub mod authority;
pub mod command;
pub mod diag;
pub mod feature;
pub mod hash;
pub mod log;
pub mod query;
pub mod state;

pub use actor::{Actor, Role};
pub use authority::{Authority, Outcome, Policy};
pub use command::{Change, Command, CommandKind, Expected, Revision};
pub use diag::{DiagCode, Diagnostic};
pub use feature::{
    shell_end_faces, EndMode, Extent, FaceInfo, FeaturePattern, FeatureSlot, FeatureSpec,
    FeatureView, SolidSummary,
};
pub use hash::Hash128;
pub use log::{LogEntry, ReplayError};
pub use query::{PlaneFrame, Query, QueryResult, SketchInfo};
pub use state::{SketchSlot, State};
