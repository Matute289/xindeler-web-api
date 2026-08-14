use crate::config::AppConfig;

/// Empty in Fase 0. Fase 1 adds rate limiters (same pattern as
/// `xindeler-auth`'s `AppState`); Fase 2 adds the session store.
pub struct AppState {}

impl AppState {
    pub fn from_config(_config: &AppConfig) -> Self {
        Self {}
    }
}
