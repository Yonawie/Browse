//! Runtime view of `schemas/policy.schema.json`. Defaults are the secure ones.

use core_types::Sensitivity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Disables all non-local providers and outbound AI-layer connections.
    pub offline_mode: bool,
    /// Sensitivity classes that may be sent to cloud providers. `Private` and
    /// `Secret` are rejected even if listed (see `Sensitivity::cloud_eligible`).
    pub cloud_allowed_classes: Vec<Sensitivity>,
    /// Domains always classified `private`.
    pub private_domains: Vec<String>,
    /// Domains the user explicitly downgraded to `public` despite a session.
    pub public_override_domains: Vec<String>,
    /// Domains whose content is never stored or indexed.
    pub never_remember_domains: Vec<String>,
    /// Agentic actions are opt-in.
    pub agent_enabled: bool,
    pub dev_mode_enabled: bool,
    /// Exact `https://localhost:port` origins allowed in dev mode.
    pub dev_localhost_origins: Vec<String>,
    /// Whether any task may declare a money limit > 0.
    pub allow_money: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            offline_mode: false,
            cloud_allowed_classes: vec![Sensitivity::Public],
            private_domains: DEFAULT_PRIVATE_DOMAINS.iter().map(|s| s.to_string()).collect(),
            public_override_domains: vec![],
            never_remember_domains: DEFAULT_NEVER_REMEMBER.iter().map(|s| s.to_string()).collect(),
            agent_enabled: false,
            dev_mode_enabled: false,
            dev_localhost_origins: vec![],
            allow_money: false,
        }
    }
}

/// Prepopulated list; users extend it. Matching is by registrable domain.
pub const DEFAULT_PRIVATE_DOMAINS: &[&str] = &[
    "mail.google.com",
    "outlook.live.com",
    "outlook.office.com",
    "proton.me",
    "web.whatsapp.com",
    "web.telegram.org",
    "slack.com",
    "discord.com",
    "paypal.com",
    "stripe.com",
    "wise.com",
    "revolut.com",
    "sberbank.ru",
    "tinkoff.ru",
    "chase.com",
    "bankofamerica.com",
    "myhealth.example",
];

pub const DEFAULT_NEVER_REMEMBER: &[&str] = &["paypal.com", "stripe.com", "wise.com", "revolut.com", "chase.com", "bankofamerica.com"];
