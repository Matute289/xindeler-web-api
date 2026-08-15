//! Proxy for the four mutable `xindeler-auth` endpoints the account screen
//! needs. The frontend never calls `auth.xindeler.com` directly for any of
//! these — routing through here is what lets a successful change revoke
//! every live session for the account in the same request (hallazgo 8 of
//! backlog 007), which `xindeler-auth` itself has no way to do since it
//! doesn't know this service's sessions exist.

use crate::authclient::AuthClientError;
use crate::error::ApiError;
use crate::http::{Request, Response};
use crate::session::{clear_cookie, resolve_session, revoke_all_sessions};
use crate::state::AppState;
use xindeler_web_api_common::{
    AvailabilityResponse, ChangePasswordRequest, ChangeUsernameRequest, DeleteAccountRequest,
    ForgotPasswordRequest, OkResponse, ResetPasswordRequest,
};

fn map_account_error(err: AuthClientError) -> ApiError {
    match err {
        AuthClientError::Rejected(400) | AuthClientError::Rejected(401) => {
            ApiError::InvalidCredentials
        }
        AuthClientError::Rejected(429) => ApiError::RateLimit,
        AuthClientError::Rejected(status) => {
            log::warn!("xindeler-auth answered with unexpected status {status}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::Request(err) => {
            log::warn!("request to xindeler-auth failed: {err}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::MissingServiceToken => {
            // None of the four calls in this module use the service token
            // (see authclient.rs) — reachable only if that ever changes.
            log::error!("account proxy hit MissingServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        // Only sign_in() ever produces this variant (see authclient.rs) —
        // none of the four calls this maps errors for call sign_in.
        // Unreachable in practice, kept for exhaustiveness.
        AuthClientError::EmailVerificationRequired(_) => ApiError::InvalidCredentials,
    }
}

/// Same shape as `map_account_error`, but a 400 here means a malformed
/// email or an invalid/expired reset token — never "wrong password", so it
/// maps to `InvalidRequest` instead of `InvalidCredentials`. Neither
/// forgot-password nor reset-password ever authenticates with a password.
fn map_recovery_error(err: AuthClientError) -> ApiError {
    match err {
        AuthClientError::Rejected(400) => {
            ApiError::InvalidRequest("xindeler-auth rejected the request".into())
        }
        AuthClientError::Rejected(429) => ApiError::RateLimit,
        AuthClientError::Rejected(status) => {
            log::warn!("xindeler-auth answered with unexpected status {status}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::Request(err) => {
            log::warn!("request to xindeler-auth failed: {err}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::MissingServiceToken => {
            log::error!("account proxy hit MissingServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        // forgot_password/reset_password never call sign_in() either — see
        // map_account_error above for the same note.
        AuthClientError::EmailVerificationRequired(_) => {
            ApiError::InvalidRequest("xindeler-auth rejected the request".into())
        }
    }
}

/// No session required — same as today's direct call from `AuthModal`
/// during registration. Exists here too so the account screen (005) can
/// check a *new* username's availability without a second, differently
/// authenticated code path.
pub fn check_username(request: &Request, state: &AppState) -> Result<Response, ApiError> {
    let username = request
        .get_param("username")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::InvalidRequest("username is required".into()))?;
    let available = state
        .auth_client
        .check_username(&username)
        .map_err(map_account_error)?;
    Ok(Response::json(&AvailabilityResponse { available }))
}

pub fn change_username(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: ChangeUsernameRequest = serde_json::from_slice(body)?;
    if payload.new_username.trim().is_empty() || payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "new_username and password_prehash are required".into(),
        ));
    }

    state
        .auth_client
        .change_username(
            &identity.username,
            &payload.password_prehash,
            &payload.new_username,
        )
        .map_err(map_account_error)?;

    // The session's cached username is now stale; revoke rather than patch
    // it in place — same "force a relogin" pattern as change_password.
    revoke_all_sessions(&identity.uuid)?;
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

pub fn change_password(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: ChangePasswordRequest = serde_json::from_slice(body)?;
    if payload.current_password_prehash.trim().is_empty()
        || payload.new_password_prehash.trim().is_empty()
    {
        return Err(ApiError::InvalidRequest(
            "current_password_prehash and new_password_prehash are required".into(),
        ));
    }

    state
        .auth_client
        .change_password(
            &identity.username,
            &payload.current_password_prehash,
            &payload.new_password_prehash,
        )
        .map_err(map_account_error)?;

    revoke_all_sessions(&identity.uuid)?;
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

pub fn delete_account(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: DeleteAccountRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash is required".into(),
        ));
    }

    state
        .auth_client
        .delete_account(&identity.username, &payload.password_prehash)
        .map_err(map_account_error)?;

    revoke_all_sessions(&identity.uuid)?;
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

/// No session required — this is the "forgot password, can't log in" flow.
/// Always 200 regardless of whether the email is registered, matching
/// xindeler-auth's own anti-enumeration behavior on `/forgot-password`.
pub fn forgot_password(body: &[u8], state: &AppState) -> Result<Response, ApiError> {
    let payload: ForgotPasswordRequest = serde_json::from_slice(body)?;
    if payload.email.trim().is_empty() {
        return Err(ApiError::InvalidRequest("email is required".into()));
    }
    state
        .auth_client
        .forgot_password(&payload.email)
        .map_err(map_recovery_error)?;
    Ok(Response::json(&OkResponse { ok: true }))
}

/// No session required — the whole point of a reset token is proving
/// identity *without* one. Known limitation, not fixed here: xindeler-auth's
/// `reset_password` doesn't return the uuid it just updated, so this can't
/// revoke every session for the account the way `change_password` does —
/// forcing that contract change on xindeler-auth is out of scope (see
/// `.backlog/SPEC.md`, "no le pide a xindeler-auth que cambie su contrato").
/// The 7-day session TTL is what actually bounds this gap (hallazgo 8 of
/// backlog 007). As a same-request bonus that costs nothing: if the caller
/// happens to still be carrying a session cookie for the account being
/// reset (e.g. resetting from a tab that was already logged in), that one
/// session is revoked too.
pub fn reset_password(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: ResetPasswordRequest = serde_json::from_slice(body)?;
    if payload.token.trim().is_empty() || payload.new_password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "token and new_password_prehash are required".into(),
        ));
    }

    state
        .auth_client
        .reset_password(&payload.token, &payload.new_password_prehash)
        .map_err(map_recovery_error)?;

    if let Ok(identity) = resolve_session(request) {
        revoke_all_sessions(&identity.uuid)?;
    }

    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}
