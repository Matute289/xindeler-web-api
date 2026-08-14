use std::env;
use std::net::SocketAddr;
use std::sync::OnceLock;

/// Fase 0: only what `/ping` needs to bind and serve. Fase 1 adds SMTP and
/// database config; Fase 2 adds `AUTH_SERVICE_TOKEN` for calling
/// `xindeler-auth`. Both will need the same redacted-`Debug` treatment
/// `xindeler-auth`'s `config.rs` uses once there's a secret to redact.
#[derive(Debug)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub http_workers: usize,
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

        Ok(Self {
            bind_addr,
            http_workers,
        })
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
    use super::AppConfig;

    #[test]
    fn bind_addr_defaults_to_a_loopback_port() {
        let config = AppConfig::from_iter(Vec::<(&str, &str)>::new()).unwrap();
        assert_eq!(config.bind_addr.to_string(), "127.0.0.1:8020");
        assert_eq!(config.http_workers, 16);
    }

    #[test]
    fn configuration_rejects_malformed_values() {
        for (key, value) in [
            ("WEB_API_BIND_ADDR", "not-an-address"),
            ("WEB_API_HTTP_WORKERS", "0"),
            ("WEB_API_HTTP_WORKERS", "257"),
            ("WEB_API_HTTP_WORKERS", "not-a-number"),
        ] {
            assert!(
                AppConfig::from_iter(vec![(key, value)]).is_err(),
                "{key}={value}"
            );
        }
    }
}
