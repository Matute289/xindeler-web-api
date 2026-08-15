CREATE TABLE waitlist (
    id         INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    email      TEXT    NOT NULL,
    platform   TEXT    NOT NULL,
    source     TEXT    NOT NULL
);

-- COLLATE NOCASE: dedup is case-insensitive, matching the Python service's
-- email_in_csv() (case-insensitive compare on the email column).
CREATE UNIQUE INDEX idx_waitlist_email ON waitlist(email COLLATE NOCASE);
