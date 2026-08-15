#![forbid(unsafe_code)]

//! Wire types shared between crates in this workspace.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct WaitlistPayload {
    pub name: String,
    pub email: String,
    pub platform: String,
    pub source: String,
    #[serde(default)]
    pub honeypot: String,
}

#[derive(Debug, Deserialize)]
pub struct ContributePayload {
    pub name: String,
    pub email: String,
    pub skills: String,
    #[serde(default)]
    pub portfolio: String,
    #[serde(default)]
    pub honeypot: String,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct CountResponse {
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub online: bool,
    /// RFC 3339 / ISO 8601, matching the Python service's
    /// `datetime.now(timezone.utc).isoformat()`.
    pub checked_at: String,
}

/// `password_prehash` is always the client-side prehash
/// (`netPrehash()`/`net_prehash()`) — this service never sees a raw
/// password and never recalculates this value, only forwards it.
#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password_prehash: String,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub username: String,
}
