//! Proxy for the mutable `xindeler-auth` endpoints the account screen
//! needs. The frontend never calls `auth.xindeler.com` directly for any of
//! these — routing through here is what lets a successful change revoke
//! every live session for the account in the same request (hallazgo 8 of
//! backlog 007), which `xindeler-auth` itself has no way to do since it
//! doesn't know this service's sessions exist.

use crate::authclient::{
    should_forward_recovery_verbatim, should_forward_verbatim, AuthClientError,
};
use crate::error::{self, ApiError};
use crate::game_server_client::GameServerClientError;
use crate::http::{Request, Response};
use crate::session::{clear_cookie, resolve_session, revoke_all_sessions};
use crate::state::AppState;
use crate::totp_status;
use std::net::IpAddr;
use uuid::Uuid;
use xindeler_web_api_common::{
    AccountEmailRequest, AvailabilityResponse, ChangePasswordRequest, ChangeUsernameRequest,
    CharactersResponse, DeleteAccountRequest, ForgotPasswordRequest, OkResponse, RegisterRequest,
    RenameCharacterRequest, ResendVerificationRequest, ResetPasswordRequest,
    TotpBackupCodesResponse, TotpCodeRequest, TotpEnrollRequest, TotpEnrollResponse,
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
/// reasoning as `EMAIL_VERIFICATION_REQUIRED` in `session::login`. `verbatim`
/// is which allowlist applies: `should_forward_verbatim` for
/// change_username/delete_account/2fa/*, `should_forward_recovery_verbatim`
/// for the register/recovery proxies below — never hardcoded here, so a
/// caller can't accidentally forward a code from the wrong family (see
/// `should_forward_verbatim`'s doc comment for why that distinction exists).
fn map_or_forward(
    err: AuthClientError,
    verbatim: impl Fn(&str) -> bool,
    mapper: impl FnOnce(AuthClientError) -> ApiError,
) -> Result<Response, ApiError> {
    if let AuthClientError::RejectedWithBody {
        status,
        code,
        message,
    } = &err
    {
        if verbatim(code) {
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
pub fn check_username(
    request: &Request,
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let username = request
        .get_param("username")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::InvalidRequest("username is required".into()))?;
    let available = state
        .auth_client
        .check_username(remote_ip, &username)
        .map_err(map_account_error)?;
    Ok(Response::json(&AvailabilityResponse { available }))
}

pub fn change_username(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
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
        remote_ip,
        &identity.username,
        &payload.password_prehash,
        &payload.new_username,
        payload.code.as_deref(),
    ) {
        return map_or_forward(err, should_forward_verbatim, map_account_error);
    }

    // The session's cached username is now stale; revoke rather than patch
    // it in place — same "force a relogin" pattern as change_password.
    revoke_all_sessions(&identity.uuid)?;
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

pub fn change_password(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
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
            remote_ip,
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
    remote_ip: IpAddr,
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
        remote_ip,
        &identity.username,
        &payload.password_prehash,
        payload.code.as_deref(),
    ) {
        return map_or_forward(err, should_forward_verbatim, map_account_error);
    }

    revoke_all_sessions(&identity.uuid)?;
    totp_status::mark_disabled(&identity.uuid)?;
    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

/// No session required — this is the "forgot password, can't log in" flow.
/// Always 200 regardless of whether the email is registered, matching
/// xindeler-auth's own anti-enumeration behavior on `/forgot-password`.
pub fn forgot_password(
    body: &[u8],
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: ForgotPasswordRequest = serde_json::from_slice(body)?;
    if payload.email.trim().is_empty() {
        return Err(ApiError::InvalidRequest("email is required".into()));
    }
    state
        .auth_client
        .forgot_password(remote_ip, &payload.email)
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
    remote_ip: IpAddr,
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
        .reset_password(remote_ip, &payload.token, &payload.new_password_prehash)
        .map_err(map_recovery_error)?;

    if let Ok(identity) = resolve_session(request) {
        revoke_all_sessions(&identity.uuid)?;
    }

    Ok(clear_cookie(Response::json(&OkResponse { ok: true })))
}

/// No session required — same "prove you can receive email" flow the
/// frontend used to run directly against `auth.xindeler.com`. A 200 here
/// covers both a genuinely new registration and xindeler-auth's own
/// anti-enumeration response for an email already in use — same response
/// shape either way, nothing for the client to distinguish.
pub fn register(body: &[u8], remote_ip: IpAddr, state: &AppState) -> Result<Response, ApiError> {
    let payload: RegisterRequest = serde_json::from_slice(body)?;
    if payload.username.trim().is_empty() || payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "username and password_prehash are required".into(),
        ));
    }

    if let Err(err) = state.auth_client.register(
        remote_ip,
        &payload.username,
        &payload.password_prehash,
        payload.email.as_deref(),
    ) {
        return map_or_forward(err, should_forward_recovery_verbatim, map_recovery_error);
    }

    Ok(Response::json(&OkResponse { ok: true }))
}

/// No session required — reached only via the emailed verification link,
/// same as the direct call this replaces.
pub fn verify_email(
    request: &Request,
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let token = request
        .get_param("token")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::InvalidRequest("token is required".into()))?;

    if let Err(err) = state.auth_client.verify_email(remote_ip, &token) {
        return map_or_forward(err, should_forward_recovery_verbatim, map_recovery_error);
    }

    Ok(Response::json(&OkResponse { ok: true }))
}

/// No session required — the `completion_token` in the body is what proves
/// identity here, same legacy flow as `resend_verification` below. Never a
/// session cookie: this fires before login ever succeeds.
pub fn account_email(
    body: &[u8],
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: AccountEmailRequest = serde_json::from_slice(body)?;
    if payload.completion_token.trim().is_empty() || payload.email.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "completion_token and email are required".into(),
        ));
    }

    if let Err(err) =
        state
            .auth_client
            .set_account_email(remote_ip, &payload.completion_token, &payload.email)
    {
        return map_or_forward(err, should_forward_recovery_verbatim, map_recovery_error);
    }

    Ok(Response::json(&OkResponse { ok: true }))
}

/// Same `completion_token` credential as `account_email` above.
pub fn resend_verification(
    body: &[u8],
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let payload: ResendVerificationRequest = serde_json::from_slice(body)?;
    if payload.completion_token.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "completion_token is required".into(),
        ));
    }

    if let Err(err) = state
        .auth_client
        .resend_verification(remote_ip, &payload.completion_token)
    {
        return map_or_forward(err, should_forward_recovery_verbatim, map_recovery_error);
    }

    Ok(Response::json(&OkResponse { ok: true }))
}

// --- Fase L (2FA/TOTP), all four require an active session; the account's
// current password/code travel in the body, same reauth-per-sensitive-
// action pattern as change-username/change-password/delete above. ---

pub fn totp_enroll(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let payload: TotpEnrollRequest = serde_json::from_slice(body)?;
    if payload.password_prehash.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "password_prehash is required".into(),
        ));
    }
    match state
        .auth_client
        .totp_enroll(remote_ip, &identity.username, &payload.password_prehash)
    {
        Ok(enrollment) => Ok(Response::json(&TotpEnrollResponse {
            secret_base32: enrollment.secret_base32,
            otpauth_url: enrollment.otpauth_url,
            qr_png_base64: enrollment.qr_png_base64,
        })),
        Err(err) => map_or_forward(err, should_forward_verbatim, map_account_error),
    }
}

pub fn totp_confirm(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
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
        remote_ip,
        &identity.username,
        &payload.password_prehash,
        &payload.code,
    ) {
        Ok(backup_codes) => {
            totp_status::mark_confirmed(&identity.uuid)?;
            Ok(Response::json(&TotpBackupCodesResponse { backup_codes }))
        }
        Err(err) => map_or_forward(err, should_forward_verbatim, map_account_error),
    }
}

pub fn totp_disable(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
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
        remote_ip,
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
        Err(err) => map_or_forward(err, should_forward_verbatim, map_account_error),
    }
}

pub fn totp_regenerate_backup_codes(
    body: &[u8],
    request: &Request,
    remote_ip: IpAddr,
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
        remote_ip,
        &identity.username,
        &payload.password_prehash,
        &payload.code,
    ) {
        Ok(backup_codes) => Ok(Response::json(&TotpBackupCodesResponse { backup_codes })),
        Err(err) => map_or_forward(err, should_forward_verbatim, map_account_error),
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

pub fn list_characters(
    request: &Request,
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let uuid = session_uuid(&identity.uuid)?;
    let token = state
        .auth_client
        .issue_character_access_token(remote_ip, uuid)
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
    remote_ip: IpAddr,
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
        .issue_character_access_token(remote_ip, uuid)
        .map_err(map_character_token_error)?;
    match state
        .game_server_client
        .rename_character(token, character_id, &payload.new_alias)
    {
        Ok(()) => Ok(Response::json(&OkResponse { ok: true })),
        Err(err) => map_game_server_error(err),
    }
}

/// A buggy or compromised game server forwarding e.g. `text/html` or
/// `image/svg+xml` (script-capable) verbatim would have those bytes served
/// on this service's own origin, right next to the session cookie -- so only
/// an actual image format is ever forwarded as-is.
const ALLOWED_PORTRAIT_CONTENT_TYPES: [&str; 3] = ["image/webp", "image/png", "image/jpeg"];

/// Matches the endpoint's documented cache contract: private (never a shared
/// cache) for 5 minutes. Set explicitly here rather than left to
/// `web::finalize()`'s default `no-store` -- without it the browser would
/// never store the response or send `If-None-Match` back, making the 304
/// path below unreachable in practice.
const PORTRAIT_CACHE_CONTROL: &str = "private, max-age=300";

pub fn character_portrait(
    request: &Request,
    character_id: i64,
    remote_ip: IpAddr,
    state: &AppState,
) -> Result<Response, ApiError> {
    let identity = resolve_session(request)?;
    let uuid = session_uuid(&identity.uuid)?;
    let token = state
        .auth_client
        .issue_character_access_token(remote_ip, uuid)
        .map_err(map_character_token_error)?;
    let if_none_match = request.header("If-None-Match");
    match state
        .game_server_client
        .get_character_portrait(token, character_id, if_none_match)
    {
        // 304/404/503 are documented, valid outcomes (see
        // `PortraitResponse`'s doc comment) -- forwarded with only the
        // headers that outcome actually carries, never the upstream's body.
        // Anything else (500 included) is the same "understood but not a
        // sanctioned shape" case `map_game_server_error` treats as a leak
        // risk for every other game-server call in this file.
        Ok(portrait) => match portrait.status {
            200 => {
                let content_type = portrait
                    .content_type
                    .filter(|content_type| {
                        ALLOWED_PORTRAIT_CONTENT_TYPES.contains(&content_type.as_str())
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_owned());
                let mut response = Response::bytes(content_type, portrait.data)
                    .with_unique_header("Cache-Control", PORTRAIT_CACHE_CONTROL);
                if let Some(etag) = portrait.etag {
                    response = response.with_unique_header("ETag", etag);
                }
                Ok(response)
            }
            304 => {
                // `empty_204()` is reused purely for its no-body/no-header
                // shape, not because this is actually a 204 -- overwritten
                // right below.
                let mut response = Response::empty_204()
                    .with_status_code(304)
                    .with_unique_header("Cache-Control", PORTRAIT_CACHE_CONTROL);
                if let Some(etag) = portrait.etag {
                    response = response.with_unique_header("ETag", etag);
                }
                Ok(response)
            }
            404 => Ok(Response::empty_404()),
            503 => {
                let mut response = Response::empty_204().with_status_code(503);
                if let Some(retry_after) = portrait.retry_after {
                    response = response.with_unique_header("Retry-After", retry_after);
                }
                Ok(response)
            }
            status => {
                log::warn!("game server portrait answered with unexpected status {status}");
                Err(ApiError::UpstreamGameServerError)
            }
        },
        Err(err) => map_game_server_error(err),
    }
}
