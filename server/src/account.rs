//! Proxy for the mutable `xindeler-auth` endpoints the account screen
//! needs. The frontend never calls `auth.xindeler.com` directly for any of
//! these — routing through here is what lets a successful change revoke
//! every live session for the account in the same request (hallazgo 8 of
//! backlog 007), which `xindeler-auth` itself has no way to do since it
//! doesn't know this service's sessions exist.

use crate::authclient::{should_forward_verbatim, AuthClientError};
use crate::error::{self, ApiError};
use crate::game_server_client::GameServerClientError;
use crate::http::{Request, Response};
use crate::session::{clear_cookie, resolve_session, revoke_all_sessions};
use crate::state::AppState;
use crate::totp_status;
use uuid::Uuid;
use xindeler_web_api_common::{
    AvailabilityResponse, ChangePasswordRequest, ChangeUsernameRequest, CharactersResponse,
    DeleteAccountRequest, ForgotPasswordRequest, OkResponse, RenameCharacterRequest,
    ResetPasswordRequest, TotpBackupCodesResponse, TotpCodeRequest, TotpEnrollRequest,
    TotpEnrollResponse,
};

fn map_account_error(err: AuthClientError) -> ApiError {
    match err {
        AuthClientError::RejectedWithBody { status, .. } if status == 400 || status == 401 => {
            ApiError::InvalidCredentials
        }
        AuthClientError::RejectedWithBody { status: 429, .. } => ApiError::RateLimit,
        AuthClientError::RejectedWithBody { status, code, .. } => {
            log::warn!("xindeler-auth answered with unexpected status {status} (code={code})");
            ApiError::UpstreamAuthError
        }
        AuthClientError::Request(err) => {
            log::warn!("request to xindeler-auth failed: {err}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::MissingServiceToken => {
            // None of the calls in this module use the service token (see
            // authclient.rs) — reachable only if that ever changes.
            log::error!("account proxy hit MissingServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        // Only issue_character_access_token() ever produces this variant
        // (see authclient.rs) — none of the calls this maps errors for call
        // it. Unreachable in practice, kept for exhaustiveness.
        AuthClientError::MissingCharacterServiceToken => {
            log::error!("account proxy hit MissingCharacterServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        // Only sign_in()/totp_login() ever produce this variant (see
        // authclient.rs) — none of the calls this maps errors for call
        // either. Unreachable in practice, kept for exhaustiveness.
        AuthClientError::EmailVerificationRequired(_) => ApiError::InvalidCredentials,
        // Only sign_in() ever produces this variant (G-08) — same note as
        // EmailVerificationRequired above. Unreachable in practice.
        AuthClientError::AccountLoginLocked { .. } => ApiError::InvalidCredentials,
    }
}

/// Same shape as `map_account_error`, but a 400 here means a malformed
/// email or an invalid/expired reset token — never "wrong password", so it
/// maps to `InvalidRequest` instead of `InvalidCredentials`. Neither
/// forgot-password nor reset-password ever authenticates with a password.
fn map_recovery_error(err: AuthClientError) -> ApiError {
    match err {
        AuthClientError::RejectedWithBody { status: 400, .. } => {
            ApiError::InvalidRequest("xindeler-auth rejected the request".into())
        }
        AuthClientError::RejectedWithBody { status: 429, .. } => ApiError::RateLimit,
        AuthClientError::RejectedWithBody { status, code, .. } => {
            log::warn!("xindeler-auth answered with unexpected status {status} (code={code})");
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
        AuthClientError::MissingCharacterServiceToken => {
            log::error!("account proxy hit MissingCharacterServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        // forgot_password/reset_password never call sign_in()/totp_login()
        // either — see map_account_error above for the same note.
        AuthClientError::EmailVerificationRequired(_) => {
            ApiError::InvalidRequest("xindeler-auth rejected the request".into())
        }
        AuthClientError::AccountLoginLocked { .. } => {
            ApiError::InvalidRequest("xindeler-auth rejected the request".into())
        }
    }
}

/// A TOTP-specific rejection (`TOTP_INVALID_CODE`, `ACCOUNT_2FA_LOCKED`,
/// ...) is forwarded verbatim instead of collapsed through `mapper` — same
/// reasoning as `EMAIL_VERIFICATION_REQUIRED` in `session::login`: the
/// frontend needs the real code to show the right copy.
fn map_or_forward(
    err: AuthClientError,
    mapper: impl FnOnce(AuthClientError) -> ApiError,
) -> Result<Response, ApiError> {
    if let AuthClientError::RejectedWithBody {
        status,
        code,
        message,
    } = &err
    {
        if should_forward_verbatim(code) {
            return Ok(error::forwarded_response(
                *status,
                code.clone(),
                message.clone(),
            ));
        }
    }
    Err(mapper(err))
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

    if let Err(err) = state.auth_client.change_username(
        &identity.username,
        &payload.password_prehash,
        &payload.new_username,
        payload.code.as_deref(),
    ) {
        return map_or_forward(err, map_account_error);
    }

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

    if let Err(err) = state.auth_client.delete_account(
        &identity.username,
        &payload.password_prehash,
        payload.code.as_deref(),
    ) {
        return map_or_forward(err, map_account_error);
    }

    revoke_all_sessions(&identity.uuid)?;
    totp_status::mark_disabled(&identity.uuid)?;
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

// --- Fase L (2FA/TOTP), all four require an active session; the account's
// current password/code travel in the body, same reauth-per-sensitive-
// action pattern as change-username/change-password/delete above. ---

pub fn totp_enroll(body: &[u8], request: &Request, state: &AppState) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: TotpEnrollRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash is required".into(),
        ));
    }
    match state
        .auth_client
        .totp_enroll(&identity.username, &payload.password_prehash)
    {
        Ok(enrollment) => Ok(Response::json(&TotpEnrollResponse {
            secret_base32: enrollment.secret_base32,
            otpauth_url: enrollment.otpauth_url,
            qr_png_base64: enrollment.qr_png_base64,
        })),
        Err(err) => map_or_forward(err, map_account_error),
    }
}

pub fn totp_confirm(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: TotpCodeRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() || payload.code.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash and code are required".into(),
        ));
    }
    match state.auth_client.totp_confirm(
        &identity.username,
        &payload.password_prehash,
        &payload.code,
    ) {
        Ok(backup_codes) => {
            totp_status::mark_confirmed(&identity.uuid)?;
            Ok(Response::json(&TotpBackupCodesResponse { backup_codes }))
        }
        Err(err) => map_or_forward(err, map_account_error),
    }
}

pub fn totp_disable(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: TotpCodeRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() || payload.code.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash and code are required".into(),
        ));
    }
    match state.auth_client.totp_disable(
        &identity.username,
        &payload.password_prehash,
        &payload.code,
    ) {
        Ok(()) => {
            totp_status::mark_disabled(&identity.uuid)?;
            // Turning 2FA off reduces the account's security — same
            // "force a relogin" pattern as change_username/change_password
            // (hallazgo 8 of backlog 007), unlike confirming a *new*
            // enrollment below, which doesn't need it.
            revoke_all_sessions(&identity.uuid)?;
            Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
        }
        Err(err) => map_or_forward(err, map_account_error),
    }
}

pub fn totp_regenerate_backup_codes(
    body: &[u8],
    request: &Request,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: TotpCodeRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() || payload.code.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash and code are required".into(),
        ));
    }
    match state.auth_client.totp_regenerate_backup_codes(
        &identity.username,
        &payload.password_prehash,
        &payload.code,
    ) {
        Ok(backup_codes) => Ok(Response::json(&TotpBackupCodesResponse { backup_codes })),
        Err(err) => map_or_forward(err, map_account_error),
    }
}

// --- Fase F (NH-79): character-list/rename proxy. Both require an active
// session. `identity.uuid` is what actually authorizes the call against the
// game server -- via a freshly-minted, one-shot `CharacterAccessToken` --
// never anything the client supplies. ---

/// A live session's `uuid` (from `sessions.uuid`, itself set once from
/// xindeler-auth's own resolved `Uuid` in `session::create_session`) that
/// fails to parse back into a `Uuid` is a data-integrity bug, not a client
/// input problem -- never reachable through normal operation.
fn session_uuid(identity_uuid: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(identity_uuid).map_err(|_| {
        log::error!("session uuid failed to parse as a Uuid: {identity_uuid}");
        ApiError::InternalServerError
    })
}

fn map_character_token_error(err: AuthClientError) -> ApiError {
    match err {
        // xindeler-auth's UserDoesNotExist -- the session's own uuid no
        // longer names a real account (e.g. deleted moments after this
        // session was created). Force a relogin rather than a raw 500.
        AuthClientError::RejectedWithBody { code, .. } if code == "USER_NOT_FOUND" => {
            ApiError::Unauthorized
        }
        AuthClientError::RejectedWithBody { status: 429, .. } => ApiError::RateLimit,
        AuthClientError::RejectedWithBody { status, code, .. } => {
            log::warn!("xindeler-auth answered with unexpected status {status} (code={code})");
            ApiError::UpstreamAuthError
        }
        AuthClientError::Request(err) => {
            log::warn!("request to xindeler-auth failed: {err}");
            ApiError::UpstreamAuthError
        }
        AuthClientError::MissingCharacterServiceToken => {
            log::error!("WEB_API_SERVICE_TOKEN is not configured — character proxy cannot work");
            ApiError::InternalServerError
        }
        // Only issue_character_access_token() is ever mapped through this
        // function -- verify()/sign_in()/totp_login() never produce these
        // two. Unreachable in practice, kept for exhaustiveness.
        AuthClientError::MissingServiceToken => {
            log::error!("character proxy hit MissingServiceToken unexpectedly");
            ApiError::InternalServerError
        }
        AuthClientError::EmailVerificationRequired(_) => ApiError::InternalServerError,
        AuthClientError::AccountLoginLocked { .. } => ApiError::InternalServerError,
    }
}

fn map_game_server_error(err: GameServerClientError) -> Result<Response, ApiError> {
    match err {
        // "Understood but refused" (character not found/not yours, invalid
        // name, name already taken) -- forwarded verbatim, same
        // "the frontend needs the real message" reasoning `map_or_forward`
        // already applies to xindeler-auth's TOTP-specific rejections. The
        // game server's own message is already safe to show (see that
        // repo's `PersistenceError::public_message()`), just plain text
        // instead of a JSON envelope, so there's no upstream `code` to
        // forward alongside it -- a single generic one covers every 409
        // shape from this endpoint.
        GameServerClientError::Rejected {
            status: 409,
            message,
        } => Ok(error::forwarded_response(
            409,
            "CHARACTER_ACTION_REJECTED".to_owned(),
            message,
        )),
        GameServerClientError::Rejected { status, message } => {
            log::warn!("game server answered with unexpected status {status}: {message}");
            Err(ApiError::UpstreamGameServerError)
        }
        GameServerClientError::Request(err) => {
            log::warn!("request to game server failed: {err}");
            Err(ApiError::UpstreamGameServerError)
        }
    }
}

pub fn list_characters(request: &Request, state: &AppState) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let uuid = session_uuid(&identity.uuid)?;
    let token = state
        .auth_client
        .issue_character_access_token(uuid)
        .map_err(map_character_token_error)?;
    match state.game_server_client.list_characters(token) {
        Ok(characters) => Ok(Response::json(&CharactersResponse { characters })),
        Err(err) => map_game_server_error(err),
    }
}

pub fn rename_character(
    body: &[u8],
    request: &Request,
    character_id: i64,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: RenameCharacterRequest = serde_json::from_slice(body)?;
    if payload.new_alias.trim().is_empty() {
        return Err(ApiError::InvalidRequest("new_alias is required".into()));
    }

    let uuid = session_uuid(&identity.uuid)?;
    let token = state
        .auth_client
        .issue_character_access_token(uuid)
        .map_err(map_character_token_error)?;
    match state
        .game_server_client
        .rename_character(token, character_id, &payload.new_alias)
    {
        Ok(()) => Ok(Response::json(&OkResponse { ok: true })),
        Err(err) => map_game_server_error(err),
    }
}
