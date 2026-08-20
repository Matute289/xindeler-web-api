//! CLI subcommand to honor a data-subject deletion request against the
//! `waitlist`/`contributors` tables (Ley 25.326 art. 16 — supresión). No
//! HTTP endpoint exists for this on purpose: these rows are never deleted
//! by an unattended process, only by an operator running this command by
//! hand over SSH after a real request comes in.
//!
//! `xindeler-web-api-server delete-request <email>` — dry run, never
//! writes anything. Prints exactly what a real run would delete.
//!
//! `xindeler-web-api-server delete-request <email> --confirm <email>` —
//! deletes, but only if the confirmation email is a byte-for-byte match of
//! the target email. This is deliberately not a bare `--yes` flag: a typo'd
//! or copy-pasted-from-the-wrong-request confirmation email fails closed
//! instead of silently deleting the wrong row.

use crate::db::db;
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};
use std::fs::OpenOptions;
use std::io::Write;

struct MatchedRow {
    table: &'static str,
    id: i64,
    name: String,
    email: String,
    created_at: i64,
}

fn find_in(conn: &Connection, table: &'static str, email: &str) -> Vec<MatchedRow> {
    let sql =
        format!("SELECT id, name, email, created_at FROM {table} WHERE email = ?1 COLLATE NOCASE");
    let mut stmt = conn.prepare(&sql).unwrap_or_else(|err| {
        panic!("[delete-request] failed to prepare lookup on {table}: {err}")
    });
    let rows = stmt
        .query_map(params![email], |row| {
            Ok(MatchedRow {
                table,
                id: row.get(0)?,
                name: row.get(1)?,
                email: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .unwrap_or_else(|err| panic!("[delete-request] failed to query {table}: {err}"));
    rows.filter_map(Result::ok).collect()
}

fn print_row(row: &MatchedRow) {
    let when = chrono::DateTime::from_timestamp(row.created_at, 0)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, false))
        .unwrap_or_else(|| row.created_at.to_string());
    println!(
        "  [{table}#{id}] name={name:?} email={email:?} created_at={when}",
        table = row.table,
        id = row.id,
        name = row.name,
        email = row.email,
    );
}

/// Appends one line per deletion to a plain-text log next to the database
/// file — the paper trail that a request was honored, and when. Best
/// effort: a failure here is loud but never blocks the deletion itself,
/// since the whole point of this command is to actually remove the data.
fn append_audit_log(email: &str, deleted: &[MatchedRow]) {
    let mut log_path = crate::config::get().database_path.clone();
    log_path.set_file_name("privacy-deletions.log");
    let tables: Vec<&str> = deleted.iter().map(|r| r.table).collect();
    let line = format!(
        "{ts} email={email:?} tables={tables:?} rows={n}\n",
        ts = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false),
        n = deleted.len(),
    );
    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(mut file) => {
            if let Err(err) = file.write_all(line.as_bytes()) {
                eprintln!(
                    "[delete-request] WARNING: failed to write audit log {log_path:?}: {err}"
                );
            }
        }
        Err(err) => {
            eprintln!("[delete-request] WARNING: failed to open audit log {log_path:?}: {err}");
        }
    }
    print!("[delete-request] audit log line: {line}");
}

pub fn run(email: &str, confirm: Option<&str>) -> ! {
    env_logger::init();
    crate::config::initialize().expect("Invalid web-api server configuration");

    let conn = db().expect("Failed to open database");
    let matches: Vec<MatchedRow> = [
        find_in(&conn, "waitlist", email),
        find_in(&conn, "contributors", email),
    ]
    .into_iter()
    .flatten()
    .collect();

    if matches.is_empty() {
        println!("[delete-request] no rows match email {email:?} in waitlist or contributors — nothing to do");
        std::process::exit(0);
    }

    println!(
        "[delete-request] found {} row(s) matching {email:?}:",
        matches.len()
    );
    for row in &matches {
        print_row(row);
    }

    let Some(confirm) = confirm else {
        println!(
            "\n[delete-request] DRY RUN — nothing was deleted. To actually delete these rows, re-run:\n  xindeler-web-api-server delete-request {email:?} --confirm {email:?}"
        );
        std::process::exit(0);
    };

    // Byte-for-byte, not case-insensitive: this only guards against
    // shell-history and copy/paste mistakes, so it should be exactly as
    // strict as what the operator actually typed.
    if confirm != email {
        eprintln!(
            "[delete-request] ABORTED — the --confirm value ({confirm:?}) doesn't match the target email ({email:?}). Nothing was deleted."
        );
        std::process::exit(1);
    }

    conn.execute_batch("BEGIN")
        .expect("Failed to start transaction");
    for row in &matches {
        let sql = format!("DELETE FROM {} WHERE id = ?1", row.table);
        if let Err(err) = conn.execute(&sql, params![row.id]) {
            let _ = conn.execute_batch("ROLLBACK");
            panic!(
                "[delete-request] delete failed on {}#{}: {err}",
                row.table, row.id
            );
        }
    }
    conn.execute_batch("COMMIT")
        .expect("Failed to commit transaction");

    println!(
        "\n[delete-request] deleted {} row(s) for {email:?}.",
        matches.len()
    );
    append_audit_log(email, &matches);
    std::process::exit(0);
}
