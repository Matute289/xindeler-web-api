//! Derived TOTP enrollment state, keyed by `uuid` — see `migrations/V4__totp_status.sql`
//! for why this exists instead of asking xindeler-auth for it: there's no
//! "is TOTP confirmed" endpoint in Fase L, only mutation endpoints. A row's
//! presence here is inferred entirely from what this service already
//! observes, and kept in sync from every place that observation happens.

use crate::db::db;
use crate::error::ApiError;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

pub fn mark_confirmed(uuid: &str) -> Result<(), ApiError> {
    let now = Utc::now().timestamp();
    db()?.execute(
        "INSERT INTO totp_status (uuid, confirmed_at) VALUES (?1, ?2) \
         ON CONFLICT(uuid) DO UPDATE SET confirmed_at = excluded.confirmed_at",
        params![uuid, now],
    )?;
    Ok(())
}

pub fn mark_disabled(uuid: &str) -> Result<(), ApiError> {
    db()?.execute("DELETE FROM totp_status WHERE uuid = ?1", params![uuid])?;
    Ok(())
}

pub fn is_enabled(uuid: &str) -> Result<bool, ApiError> {
    Ok(db()?
        .query_row(
            "SELECT 1 FROM totp_status WHERE uuid = ?1",
            params![uuid],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each `db()` call in test mode opens a fresh in-memory database — these
    // only exercise that the schema/queries are well-formed in isolation.
    // The actual round trip (confirm → is_enabled → disable → is_enabled,
    // sharing one real database across calls) is covered by the HTTP
    // integration tests exercising the `/api/account/2fa/*` handlers
    // against the real compiled binary instead.

    #[test]
    fn mark_confirmed_does_not_error() {
        assert!(mark_confirmed("uuid-a").is_ok());
    }

    #[test]
    fn mark_disabled_does_not_error_for_an_unknown_uuid() {
        assert!(mark_disabled("never-seen").is_ok());
    }

    #[test]
    fn is_enabled_defaults_to_false_for_an_unknown_uuid() {
        assert!(!is_enabled("never-seen").unwrap());
    }
}
