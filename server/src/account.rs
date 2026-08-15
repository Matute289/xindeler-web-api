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
    OkResponse,
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
