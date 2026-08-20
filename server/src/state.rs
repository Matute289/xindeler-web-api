use crate::authclient::AuthClient;
use crate::cache::TtlCache;
use crate::config::AppConfig;
use crate::game_server_client::GameServerClient;
use crate::ratelimit::RateLimiter;
use std::time::Duration;

const STATUS_TTL: Duration = Duration::from_secs(30);
const COUNT_TTL: Duration = Duration::from_secs(60);

pub struct AppState {
    pub status_cache: TtlCache<(bool, String)>,
    pub count_cache: TtlCache<u64>,
    pub waitlist_requests: RateLimiter,
    pub contribute_requests: RateLimiter,
    pub login_requests: RateLimiter,
    pub auth_client: AuthClient,
    pub game_server_client: GameServerClient,
}

impl AppState {
    pub fn from_config(config: &AppConfig) -> Self {
        let window = Duration::from_secs(config.rate_limit_window_secs);
        Self {
            status_cache: TtlCache::new(STATUS_TTL),
            count_cache: TtlCache::new(COUNT_TTL),
            waitlist_requests: RateLimiter::with_limits(config.rate_limit_max, window),
            contribute_requests: RateLimiter::with_limits(config.rate_limit_max, window),
            login_requests: RateLimiter::with_limits(config.rate_limit_max, window),
            auth_client: AuthClient::new(
                &config.auth_public_url,
                config.auth_service_token(),
                config.web_api_service_token(),
            ),
            game_server_client: GameServerClient::new(&config.game_server_player_api_url),
        }
    }
}
