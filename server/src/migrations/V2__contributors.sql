CREATE TABLE contributors (
    id         INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    email      TEXT    NOT NULL,
    skills     TEXT    NOT NULL,
    portfolio  TEXT    NOT NULL DEFAULT ''
);

-- Same dedup semantics as waitlist — see V1__waitlist.sql.
CREATE UNIQUE INDEX idx_contributors_email ON contributors(email COLLATE NOCASE);
