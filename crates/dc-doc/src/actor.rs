use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Who is submitting a command. Humans, agents, scripts and remote users are the same kind of
/// participant (AGENTS.md §1); only their [`Role`] differs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum Actor {
    Human(String),
    Agent(String),
    Script(String),
    Remote(String),
}

impl Actor {
    /// Stable key used for role lookup, e.g. `agent:claude`.
    pub fn key(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::Human(n) => write!(f, "human:{n}"),
            Actor::Agent(n) => write!(f, "agent:{n}"),
            Actor::Script(n) => write!(f, "script:{n}"),
            Actor::Remote(n) => write!(f, "remote:{n}"),
        }
    }
}

/// Capability level of an actor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// May query but not submit.
    ReadOnly,
    /// May only submit proposals (pending changes). Direct commits are refused.
    ProposeOnly,
    /// May commit directly.
    Full,
}
