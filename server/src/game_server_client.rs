//! Thin, hand-rolled HTTP client for `xindeler-new-horizon`'s loopback-only
//! `/player_api/v1` router (NH-79). Deliberately **not** `AuthClient`: a
//! different service, with a different contract -- its error responses are
//! plain text (`axum`'s `impl IntoResponse for (StatusCode, String)`), never
//! xindeler-auth's `{code, message, request_id}` JSON envelope, so this
//! client's own error type reflects that instead of pretending otherwise.

use serde::Serialize;
use std::time::Duration;
use xindeler_auth_common::CharacterAccessToken;
use xindeler_web_api_common::CharacterSummary;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum GameServerClientError {
    Request(reqwest::Error),
    /// The game server answered outside 200-299. `message` is the
    /// best-effort plain-text response body (empty if there wasn't one, or
    /// it couldn't be read) -- unlike `AuthClientError::RejectedWithBody`,
    /// there's no `code` here: `player_api`'s rejections are plain text, not
    /// a JSON envelope (confirmed reading that repo's
    /// `server-cli/src/web/player_api.rs`, not assumed).
    Rejected {
        status: u16,
        message: String,
    },
}

impl From<reqwest::Error> for GameServerClientError {
    fn from(err: reqwest::Error) -> Self {
        GameServerClientError::Request(err)
    }
}

fn rejected(status: u16, response: reqwest::blocking::Response) -> GameServerClientError {
    GameServerClientError::Rejected {
        status,
        message: response.text().unwrap_or_default(),
    }
}

#[derive(Serialize)]
struct RenameCharacterBody<'a> {
    new_alias: &'a str,
}

/// Every field the portrait proxy (Task 3) needs to forward. Unlike
/// `list_characters`/`rename_character`, a non-2xx status here is *not* an
/// error to react to — 304/404/503 are all documented, valid outcomes the
/// caller forwards as-is (see `docs/superpowers/specs/2026-08-31-campfire-character-scene-design.md`).
/// Only a genuine network/connection failure becomes a `GameServerClientError`.
pub struct PortraitResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub retry_after: Option<String>,
    pub data: Vec<u8>,
}

pub struct GameServerClient {
    client: reqwest::blocking::Client,
    base_url: String,
}

impl GameServerClient {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client config is always valid");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// `token` is a `CharacterAccessToken` freshly minted by
    /// `AuthClient::issue_character_access_token` -- never cached or reused
    /// across calls (60s TTL, one-shot on the game server's side).
    pub fn list_characters(
        &self,
        token: CharacterAccessToken,
    ) -> Result<Vec<CharacterSummary>, GameServerClientError> {
        let response = self
            .client
            .get(format!("{}/player_api/v1/characters", self.base_url))
            .bearer_auth(token.serialize())
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejected(status.as_u16(), response));
        }
        Ok(response.json()?)
    }

    pub fn rename_character(
        &self,
        token: CharacterAccessToken,
        character_id: i64,
        new_alias: &str,
    ) -> Result<(), GameServerClientError> {
        let response = self
            .client
            .post(format!(
                "{}/player_api/v1/characters/{character_id}/rename",
                self.base_url
            ))
            .bearer_auth(token.serialize())
            .json(&RenameCharacterBody { new_alias })
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejected(status.as_u16(), response));
        }
        Ok(())
    }

    /// `if_none_match` is the caller's own `If-None-Match` request header,
    /// forwarded upstream so the game server can answer 304 instead of
    /// re-sending unchanged image bytes.
    pub fn get_character_portrait(
        &self,
        token: CharacterAccessToken,
        character_id: i64,
        if_none_match: Option<&str>,
    ) -> Result<PortraitResponse, GameServerClientError> {
        let mut request = self
            .client
            .get(format!(
                "{}/player_api/v1/characters/{character_id}/portrait",
                self.base_url
            ))
            .bearer_auth(token.serialize());
        if let Some(etag) = if_none_match {
            request = request.header("If-None-Match", etag);
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let data = response.bytes()?.to_vec();
        Ok(PortraitResponse {
            status,
            content_type,
            etag,
            retry_after,
            data,
        })
    }
}
