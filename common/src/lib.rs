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
    /// Derived, not read from xindeler-auth directly — see `totp.rs`. True
    /// only once a login has actually resolved a TOTP challenge, or a
    /// `2fa/confirm` succeeded through this same service.
    pub totp_enabled: bool,
}

/// `202` response body for `POST /api/session/login` when the account has
/// a confirmed TOTP enrollment — no cookie is set yet at this point.
#[derive(Debug, Serialize)]
pub struct LoginTotpChallengeResponse {
    pub challenge_id: String,
    pub expires_in: u64,
}

/// `POST /api/session/login/2fa` — no session cookie required, the
/// `challenge_id` from `LoginTotpChallengeResponse` is the credential.
#[derive(Debug, Deserialize)]
pub struct LoginTotpRequest {
    pub challenge_id: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct AvailabilityResponse {
    pub available: bool,
}

/// Requires an active session (the current username is read from it, never
/// trusted from the client). `password_prehash` is the account's *current*
/// password — xindeler-auth re-validates it on every mutating call, session
/// or not.
/// `code` is the TOTP code — only required if `GET /api/session/me` says
/// the account has 2FA enabled; a no-op on xindeler-auth's side otherwise.
#[derive(Debug, Deserialize)]
pub struct ChangeUsernameRequest {
    pub new_username: String,
    pub password_prehash: String,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password_prehash: String,
    pub new_password_prehash: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteAccountRequest {
    pub password_prehash: String,
    #[serde(default)]
    pub code: Option<String>,
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

/// The four `/api/account/2fa/*` endpoints all require an active session
/// (the `username` comes from it, never the client) and the account's
/// current password, same reauth-per-sensitive-action pattern as
/// `change-username`/`change-password`/`delete`.
#[derive(Debug, Deserialize)]
pub struct TotpEnrollRequest {
    pub password_prehash: String,
}

#[derive(Debug, Serialize)]
pub struct TotpEnrollResponse {
    pub secret_base32: String,
    pub otpauth_url: String,
    /// Pre-rendered by xindeler-auth — embed directly as
    /// `data:image/png;base64,{qr_png_base64}`, no QR library needed here.
    pub qr_png_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpCodeRequest {
    pub password_prehash: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct TotpBackupCodesResponse {
    /// Shown once — xindeler-auth doesn't let a caller retrieve them again
    /// short of `2fa/backup-codes/regenerate`, which invalidates the old set.
    pub backup_codes: Vec<String>,
}

/// Fase F (NH-79): a character's resolved location. Mirrors
/// `xindeler-new-horizon`'s `LocationDto` field-for-field — only `site`
/// resolves against real data today, `kingdom`/`continent` stay `None`
/// until a real world hierarchy exists there.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Location {
    pub site: String,
    pub kingdom: Option<String>,
    pub continent: Option<String>,
}

/// Mirrors `xindeler-new-horizon`'s `CharacterSummaryDto` field-for-field —
/// this is a straight proxy of the game server's own response shape, not a
/// re-derived one, so a mismatch here would silently corrupt every
/// character-list read.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CharacterSummary {
    pub character_id: i64,
    pub name: String,
    pub level: u32,
    pub class: String,
    pub location: Option<Location>,
}

#[derive(Debug, Serialize)]
pub struct CharactersResponse {
    pub characters: Vec<CharacterSummary>,
}

/// Requires an active session — `identity.uuid` is what actually authorizes
/// the rename against the game server (via a freshly-minted
/// `CharacterAccessToken`), never anything the client supplies.
#[derive(Debug, Deserialize)]
pub struct RenameCharacterRequest {
    pub new_alias: String,
}
