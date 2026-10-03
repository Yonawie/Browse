//! Time-limited permissions granted by the user (mirrors the `grants` table).

use core_types::{Id, UnixMs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    /// Copy the user's session cookies for one origin into the agent profile.
    SessionImport,
    /// Allow data flow `originA->originB` for this session.
    CrossOriginFlow,
    /// Add an origin to the task scope.
    OriginScope,
    /// Enable a tool (e.g. an MCP plugin) for this session.
    Tool,
    /// Observe a cross-origin iframe.
    IframeObserve,
    /// Read-only access to the user's profile.
    ProfileReadonly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: Id,
    pub kind: GrantKind,
    /// Who receives the grant (`agent`, a tool name, ...).
    pub subject: String,
    /// What it applies to (origin, `a->b`, capability, ...).
    pub object: String,
    pub granted_at: UnixMs,
    pub expires_at: UnixMs,
    pub revoked_at: Option<UnixMs>,
}

impl Grant {
    pub fn new(kind: GrantKind, subject: &str, object: &str, granted_at: UnixMs, expires_at: UnixMs) -> Self {
        Self {
            id: format!("grant-{kind:?}-{object}-{granted_at}"),
            kind,
            subject: subject.into(),
            object: object.into(),
            granted_at,
            expires_at,
            revoked_at: None,
        }
    }

    pub fn active_at(&self, now: UnixMs) -> bool {
        self.revoked_at.is_none() && self.granted_at <= now && now < self.expires_at
    }
}

#[derive(Debug, Clone, Default)]
pub struct GrantStore {
    grants: Vec<Grant>,
}

impl GrantStore {
    pub fn add(&mut self, grant: Grant) {
        self.grants.push(grant);
    }

    pub fn has(&self, kind: GrantKind, object: &str, now: UnixMs) -> bool {
        self.grants.iter().any(|g| g.kind == kind && g.object == object && g.active_at(now))
    }

    pub fn revoke_all(&mut self, now: UnixMs) {
        for g in &mut self.grants {
            if g.revoked_at.is_none() {
                g.revoked_at = Some(now);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter()
    }
}
