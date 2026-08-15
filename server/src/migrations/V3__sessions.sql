-- session_id stores SHA-256(raw cookie value) — never the raw token, same
-- policy as xindeler-auth's password_reset_tokens/email_completion_tokens.
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    uuid       TEXT    NOT NULL,
    username   TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);

-- Revoking every session for a uuid (a password change, a 2FA disable) is
-- the operation hallazgo 8 of backlog 007 needs to do in the same request
-- that confirms the change.
CREATE INDEX idx_sessions_uuid ON sessions(uuid);
