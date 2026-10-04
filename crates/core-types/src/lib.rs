//! Domain types shared by every layer of Browse.
//!
//! These types are the source of truth for IPC (Shell ↔ Core), UniFFI (Swift ↔ Rust)
//! and the SQLite schema. Everything is `serde`-serializable.

pub mod entity;
pub mod observation;
pub mod origin;
pub mod scope;
pub mod tool;

pub use entity::*;
pub use observation::*;
pub use origin::*;
pub use scope::*;
pub use tool::*;

use serde::{Deserialize, Serialize};

/// Identifier type. UUID v4 strings in practice; kept as a plain string for
/// cross-language simplicity.
pub type Id = String;

/// Unix time in milliseconds, UTC.
pub type UnixMs = i64;

/// Sensitivity class of a piece of data. Computed deterministically from the
/// data's *source*, never by a model (ADR-007).
///
/// Ordering matters: `Public < Personal < Private < Secret`, and the class of a
/// composite request is the maximum of its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Personal,
    Private,
    /// Never persisted, never shown to any model. Redacted before Model Gateway.
    Secret,
}

impl Sensitivity {
    pub fn max_of<I: IntoIterator<Item = Sensitivity>>(items: I) -> Sensitivity {
        items.into_iter().max().unwrap_or(Sensitivity::Public)
    }

    /// Whether data of this class may leave the device at all.
    pub fn cloud_eligible(self, allowed: &[Sensitivity]) -> bool {
        // `Private` and `Secret` are never cloud-eligible regardless of config.
        matches!(self, Sensitivity::Public | Sensitivity::Personal) && allowed.contains(&self)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sensitivity::Public => "public",
            Sensitivity::Personal => "personal",
            Sensitivity::Private => "private",
            Sensitivity::Secret => "secret",
        }
    }
}

/// Where a value came from. Attached to every argument the model passes to a
/// tool so PolicyEngine can restore the same-origin policy for the agent
/// (ADR-005 §3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Typed or spoken by the user in this session.
    User,
    /// A confirmed user memory (`memories` table).
    Memory { id: Id },
    /// Free text authored by the model.
    Model,
    /// Read from a page on the given origin (observation or tool output).
    Origin { origin: Origin },
    /// Output of a tool that is not tied to a web origin (e.g. a plugin).
    Tool { name: String },
}

/// A JSON value carrying provenance and sensitivity labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabeledValue {
    pub value: serde_json::Value,
    pub provenance: Provenance,
    pub sensitivity: Sensitivity,
}

impl LabeledValue {
    pub fn user(value: impl Into<serde_json::Value>) -> Self {
        Self { value: value.into(), provenance: Provenance::User, sensitivity: Sensitivity::Personal }
    }

    pub fn model(value: impl Into<serde_json::Value>) -> Self {
        Self { value: value.into(), provenance: Provenance::Model, sensitivity: Sensitivity::Public }
    }

    pub fn from_origin(value: impl Into<serde_json::Value>, origin: Origin, sensitivity: Sensitivity) -> Self {
        Self { value: value.into(), provenance: Provenance::Origin { origin }, sensitivity }
    }
}

/// Decision of PolicyEngine or of the critic for a single tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Confirm { reason: String },
    Deny { reason: String },
}

impl Verdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Verdict::Allow)
    }

    /// Combine two verdicts: the stricter one wins.
    pub fn stricter(self, other: Verdict) -> Verdict {
        match (self, other) {
            (Verdict::Deny { reason }, _) | (_, Verdict::Deny { reason }) => Verdict::Deny { reason },
            (Verdict::Confirm { reason }, _) | (_, Verdict::Confirm { reason }) => Verdict::Confirm { reason },
            _ => Verdict::Allow,
        }
    }
}

/// Model role tiers used by Model Gateway and TaskScope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Fast,
    Smart,
    Vision,
    Embed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    Local,
    Cloud,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitivity_is_ordered_and_max_composes() {
        assert!(Sensitivity::Public < Sensitivity::Personal);
        assert!(Sensitivity::Private < Sensitivity::Secret);
        assert_eq!(
            Sensitivity::max_of([Sensitivity::Public, Sensitivity::Private, Sensitivity::Personal]),
            Sensitivity::Private
        );
    }

    #[test]
    fn private_is_never_cloud_eligible() {
        let allow_all = [Sensitivity::Public, Sensitivity::Personal, Sensitivity::Private, Sensitivity::Secret];
        assert!(!Sensitivity::Private.cloud_eligible(&allow_all));
        assert!(!Sensitivity::Secret.cloud_eligible(&allow_all));
        assert!(Sensitivity::Public.cloud_eligible(&[Sensitivity::Public]));
        assert!(!Sensitivity::Personal.cloud_eligible(&[Sensitivity::Public]));
    }

    #[test]
    fn stricter_verdict_wins() {
        let a = Verdict::Confirm { reason: "x".into() };
        let d = Verdict::Deny { reason: "y".into() };
        assert_eq!(a.clone().stricter(Verdict::Allow), a);
        assert_eq!(Verdict::Allow.stricter(d.clone()), d);
        assert_eq!(a.stricter(d.clone()), d);
    }
}
