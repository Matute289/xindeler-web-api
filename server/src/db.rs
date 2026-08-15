use crate::config::SQLITE_BUSY_TIMEOUT;
use crate::error::ApiError;
use rusqlite::Connection;
use std::path::Path;

/// Get a connection to the database. In tests, an in-memory database is
/// used and migrated eagerly so any caller gets a queryable schema —
/// same pattern as `xindeler-auth`'s `db()`.
pub fn db() -> Result<Connection, ApiError> {
    if cfg!(test) {
        let mut connection = Connection::open_in_memory()?;
        configure_connection(&connection, false)?;
        init_db(&mut connection)?;
        Ok(connection)
    } else {
        open_database(&crate::config::get().database_path)
    }
}

fn open_database(path: impl AsRef<Path>) -> Result<Connection, ApiError> {
    let connection = Connection::open(path)?;
    configure_connection(&connection, true)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection, persistent: bool) -> Result<(), ApiError> {
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    if persistent {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub fn init_db(db: &mut Connection) -> Result<(), ApiError> {
    mod embedded {
        use refinery::embed_migrations;
        embed_migrations!("./src/migrations");
    }

    let report = embedded::migrations::runner()
        .set_abort_divergent(false)
        .run(db)?;
    log::info!(
        "Applied {} database migrations",
        report.applied_migrations().len()
    );

    Ok(())
}
