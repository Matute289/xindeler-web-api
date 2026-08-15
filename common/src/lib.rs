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

#[derive(Debug, Serialize)]
pub struct AvailabilityResponse {
    pub available: bool,
}

/// Requires an active session (the current username is read from it, never
/// trusted from the client). `password_prehash` is the account's *current*
/// password — xindeler-auth re-validates it on every mutating call, session
/// or not.
#[derive(Debug, Deserialize)]
pub struct ChangeUsernameRequest {
    pub new_username: String,
    pub password_prehash: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password_prehash: String,
    pub new_password_prehash: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteAccountRequest {
    pub password_prehash: String,
}

/// No session required — this is the *forgot* your password flow, called
/// from a logged-out browser by definition.
#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password_prehash: String,
}
