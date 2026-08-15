use ipnet::IpNet;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

pub const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DEFAULT_DB_PATH: &str = "/opt/xindeler-web-api/data/web-api.db";
const DEFAULT_DIGEST_STATE_PATH: &str = "/opt/xindeler-web-api/data/digest-last-sent.txt";

/// Falls back to the default DB path (instead of failing startup) when
/// `WEB_API_DB_DIR` names a path whose directory doesn't exist yet — same
/// reasoning as `xindeler-auth`'s `resolve_db_path`: an unmounted volume or a
/// typo in the unit file shouldn't crash-loop the service.
fn resolve_db_path(override_value: Option<&str>) -> PathBuf {
    if let Some(value) = override_value {
        let path = PathBuf::from(value);
        if path.exists() || path.parent().is_some_and(|parent| parent.exists()) {
            return path;
        }
        log::warn!("WEB_API_DB_DIR is an invalid path, falling back to default");
    }
    PathBuf::from(DEFAULT_DB_PATH)
}

/// SMTP is optional: unset means mail sending silently no-ops (matching the
/// Python service's `if not all([SMTP_HOST, SMTP_USER, SMTP_PASS]): return`)
/// — local dev and most tests never need real credentials.
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    password: String,
    pub from: String,
}

impl SmtpConfig {
    pub fn password(&self) -> &str {
        &self.password
    }
}

/// Which peers are trusted to set `X-Forwarded-For`. Our nginx config sets
/// this header to `$remote_addr` (the real client IP), overwriting any
/// client-supplied value — safe to trust *only* from the peer nginx itself
/// connects as, i.e. loopback by default (this service always binds to
/// `127.0.0.1` and sits behind nginx on the same host).
pub struct NetworkConfig {
    trusted_proxies: Vec<IpNet>,
}

impl NetworkConfig {
    pub fn trusts(&self, address: IpAddr) -> bool {
        self.trusted_proxies
            .iter()
            .any(|network| network.contains(&address))
    }
}

pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub http_workers: usize,
    pub database_path: PathBuf,
    pub game_server_addr: SocketAddr,
    pub rate_limit_max: usize,
    pub rate_limit_window_secs: u64,
    pub smtp: Option<SmtpConfig>,
    pub owner_email: Option<String>,
    pub network: NetworkConfig,
    pub digest_state_path: PathBuf,
    pub auth_public_url: String,
    /// Same shared secret the game server already uses against
    /// `xindeler-auth`'s `/verify` — see the "Coordinación cross-repo"
    /// section of the Fase 0 plan for why this repo doesn't mint its own.
    /// Optional so Fase 1-only local dev doesn't need a real one; session
    /// login fails closed with an internal error if it's unset.
    auth_service_token: Option<String>,
}

impl AppConfig {
    pub fn auth_service_token(&self) -> Option<&str> {
        self.auth_service_token.as_deref()
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_iter(env::vars())
    }

    pub fn from_iter<K, V, I>(values: I) -> Result<Self, String>
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        let values: std::collections::HashMap<String, String> = values
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();

        let bind_addr = values
            .get("WEB_API_BIND_ADDR")
            .map(String::as_str)
            .unwrap_or("127.0.0.1:8020")
            .parse()
            .map_err(|_| "WEB_API_BIND_ADDR must be a socket address")?;

        let http_workers = values
            .get("WEB_API_HTTP_WORKERS")
            .map(String::as_str)
            .unwrap_or("16")
            .parse::<usize>()
            .map_err(|_| "WEB_API_HTTP_WORKERS must be an integer")?;
        if !(1..=256).contains(&http_workers) {
            return Err("WEB_API_HTTP_WORKERS must be between 1 and 256".into());
        }

        let game_server_addr = values
            .get("WEB_API_GAME_SERVER_ADDR")
            .map(String::as_str)
            .unwrap_or("127.0.0.1:14004")
            .parse()
            .map_err(|_| "WEB_API_GAME_SERVER_ADDR must be a socket address")?;

        let rate_limit_max = values
            .get("WEB_API_RATE_LIMIT_MAX")
            .map(String::as_str)
            .unwrap_or("3")
            .parse::<usize>()
            .map_err(|_| "WEB_API_RATE_LIMIT_MAX must be an integer")?;
        if !(1..=10_000).contains(&rate_limit_max) {
            return Err("WEB_API_RATE_LIMIT_MAX must be between 1 and 10000".into());
        }

        let rate_limit_window_secs = values
            .get("WEB_API_RATE_LIMIT_WINDOW_SECS")
            .map(String::as_str)
            .unwrap_or("3600")
            .parse::<u64>()
            .map_err(|_| "WEB_API_RATE_LIMIT_WINDOW_SECS must be an integer")?;
        if !(1..=86_400).contains(&rate_limit_window_secs) {
            return Err("WEB_API_RATE_LIMIT_WINDOW_SECS must be between 1 and 86400".into());
        }

        let smtp_fields = (
            values.get("SMTP_HOST"),
            values.get("SMTP_USER"),
            values.get("SMTP_PASS"),
        );
        let smtp = match smtp_fields {
            (Some(host), Some(user), Some(password)) => {
                let port = values
                    .get("SMTP_PORT")
                    .map(String::as_str)
                    .unwrap_or("587")
                    .parse()
                    .map_err(|_| "SMTP_PORT must be a valid port")?;
                Some(SmtpConfig {
                    host: host.clone(),
                    port,
                    user: user.clone(),
                    password: password.clone(),
                    from: values
                        .get("MAIL_FROM")
                        .cloned()
                        .unwrap_or_else(|| format!("Xindeler <{user}>")),
                })
            }
            _ => None,
        };

        let trusted_proxies = values
            .get("WEB_API_TRUSTED_PROXIES")
            .map(String::as_str)
            .unwrap_or("127.0.0.0/8,::1/128")
            .split(',')
            .map(|value| {
                value
                    .trim()
                    .parse()
                    .map_err(|_| "WEB_API_TRUSTED_PROXIES entries must be CIDRs")
            })
            .collect::<Result<Vec<_>, _>>()?;

        let auth_service_token = match values.get("AUTH_SERVICE_TOKEN") {
            Some(token) if token.len() < 32 => {
                return Err("AUTH_SERVICE_TOKEN must contain at least 32 bytes".into())
            }
            other => other.cloned(),
        };

        Ok(Self {
            bind_addr,
            http_workers,
            database_path: resolve_db_path(values.get("WEB_API_DB_DIR").map(String::as_str)),
            game_server_addr,
            rate_limit_max,
            rate_limit_window_secs,
            smtp,
            owner_email: values.get("OWNER_EMAIL").cloned(),
            network: NetworkConfig { trusted_proxies },
            digest_state_path: values
                .get("WEB_API_DIGEST_STATE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_DIGEST_STATE_PATH)),
            auth_public_url: values
                .get("AUTH_PUBLIC_URL")
                .cloned()
                .unwrap_or_else(|| "https://auth.xindeler.com".to_owned()),
            auth_service_token,
        })
    }
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("bind_addr", &self.bind_addr)
            .field("http_workers", &self.http_workers)
            .field("database_path", &self.database_path)
            .field("game_server_addr", &self.game_server_addr)
            .field("rate_limit_max", &self.rate_limit_max)
            .field("rate_limit_window_secs", &self.rate_limit_window_secs)
            .field("smtp_configured", &self.smtp.is_some())
            .field(
                "owner_email",
                &self.owner_email.as_ref().map(|_| "[REDACTED]"),
            )
            .field("auth_public_url", &self.auth_public_url)
            .field(
                "auth_service_token",
                &self.auth_service_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

pub fn initialize() -> Result<(), String> {
    CONFIG
        .set(AppConfig::from_env()?)
        .map_err(|_| "configuration was already initialized".into())
}

pub fn get() -> &'static AppConfig {
    CONFIG
        .get()
        .expect("application configuration must be initialized")
}

#[cfg(test)]
mod tests {
    use super::{resolve_db_path, AppConfig, DEFAULT_DB_PATH};
    use std::path::PathBuf;

    #[test]
    fn defaults_are_sane() {
        let config = AppConfig::from_iter(Vec::<(&str, &str)>::new()).unwrap();
        assert_eq!(config.bind_addr.to_string(), "127.0.0.1:8020");
        assert_eq!(config.http_workers, 16);
        assert_eq!(config.game_server_addr.to_string(), "127.0.0.1:14004");
        assert_eq!(config.rate_limit_max, 3);
        assert_eq!(config.rate_limit_window_secs, 3600);
        assert!(config.smtp.is_none());
        assert_eq!(config.auth_public_url, "https://auth.xindeler.com");
        assert!(config.auth_service_token().is_none());
    }

    #[test]
    fn auth_service_token_requires_at_least_32_bytes() {
        assert!(AppConfig::from_iter(vec![("AUTH_SERVICE_TOKEN", "too-short")]).is_err());

        let config = AppConfig::from_iter(vec![(
            "AUTH_SERVICE_TOKEN",
            "0123456789abcdef0123456789abcdef",
        )])
        .unwrap();
        assert_eq!(
            config.auth_service_token(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn configuration_debug_output_redacts_auth_service_token() {
        let config = AppConfig::from_iter(vec![(
            "AUTH_SERVICE_TOKEN",
            "0123456789abcdef0123456789abcdef",
        )])
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn smtp_requires_all_three_core_fields() {
        for env in [
            vec![("SMTP_HOST", "smtp.example.com")],
            vec![("SMTP_HOST", "smtp.example.com"), ("SMTP_USER", "mailer")],
            vec![("SMTP_USER", "mailer"), ("SMTP_PASS", "secret")],
        ] {
            let config = AppConfig::from_iter(env).unwrap();
            assert!(config.smtp.is_none());
        }

        let config = AppConfig::from_iter(vec![
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_USER", "mailer"),
            ("SMTP_PASS", "secret"),
        ])
        .unwrap();
        assert!(config.smtp.is_some());
    }

    #[test]
    fn configuration_debug_output_redacts_smtp_and_owner_email() {
        let config = AppConfig::from_iter(vec![
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_USER", "mailer"),
            ("SMTP_PASS", "super-secret-password"),
            ("OWNER_EMAIL", "owner@example.com"),
        ])
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("super-secret-password"));
        assert!(!debug.contains("owner@example.com"));
    }

    #[test]
    fn configuration_rejects_malformed_values() {
        for (key, value) in [
            ("WEB_API_BIND_ADDR", "not-an-address"),
            ("WEB_API_HTTP_WORKERS", "0"),
            ("WEB_API_HTTP_WORKERS", "257"),
            ("WEB_API_GAME_SERVER_ADDR", "not-an-address"),
            ("WEB_API_RATE_LIMIT_MAX", "0"),
            ("WEB_API_RATE_LIMIT_WINDOW_SECS", "0"),
            ("WEB_API_TRUSTED_PROXIES", "not-a-cidr"),
        ] {
            assert!(
                AppConfig::from_iter(vec![(key, value)]).is_err(),
                "{key}={value}"
            );
        }
    }

    #[test]
    fn db_path_override_is_used_when_its_directory_exists() {
        assert_eq!(
            resolve_db_path(Some("/tmp/web-api.db")),
            PathBuf::from("/tmp/web-api.db")
        );
    }

    #[test]
    fn db_path_falls_back_to_default_when_directory_is_missing() {
        assert_eq!(
            resolve_db_path(Some("/no/such/directory/web-api.db")),
            PathBuf::from(DEFAULT_DB_PATH)
        );
    }

    #[test]
    fn loopback_is_trusted_by_default() {
        let config = AppConfig::from_iter(Vec::<(&str, &str)>::new()).unwrap();
        assert!(config.network.trusts("127.0.0.1".parse().unwrap()));
        assert!(!config.network.trusts("203.0.113.1".parse().unwrap()));
    }
}
