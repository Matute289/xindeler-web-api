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
