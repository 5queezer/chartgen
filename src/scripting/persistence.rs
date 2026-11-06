//! SQLite store for user indicators and registered OAuth clients (ADR-0025).
//!
//! One file, `CHARTGEN_DB` (default `~/.chartgen/chartgen.db`), migrated on
//! open. OAuth clients are stored so a client keeps its `client_id` (the
//! script owner's identity) across server restarts, with `CHARTGEN_SCRIPTING`
//! or without (ADR-0036). With sign-in on, users, their browser sessions and
//! the clients they approved live here too (ADR-0044).

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Schema migrations; `PRAGMA user_version` is the number applied.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE user_indicators (
        user_id      TEXT    NOT NULL,
        name         TEXT    NOT NULL,
        source       TEXT    NOT NULL,
        meta_json    TEXT    NOT NULL,
        source_hash  TEXT    NOT NULL,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL,
        PRIMARY KEY (user_id, name)
    );
    CREATE TABLE oauth_clients (
        client_id      TEXT    PRIMARY KEY,
        redirect_uris  TEXT    NOT NULL,
        created_at     INTEGER NOT NULL
    );
",
    "
    ALTER TABLE oauth_clients ADD COLUMN last_token_at INTEGER;
",
    "
    ALTER TABLE oauth_clients ADD COLUMN client_name TEXT;
    CREATE TABLE users (
        id             TEXT    PRIMARY KEY,
        name           TEXT    NOT NULL,
        created_at     INTEGER NOT NULL,
        last_login_at  INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        token_hash  TEXT    PRIMARY KEY,
        user_id     TEXT    NOT NULL,
        created_at  INTEGER NOT NULL,
        expires_at  INTEGER NOT NULL
    );
    CREATE TABLE client_grants (
        user_id     TEXT    NOT NULL,
        client_id   TEXT    NOT NULL,
        created_at  INTEGER NOT NULL,
        PRIMARY KEY (user_id, client_id)
    );
",
];

/// A stored script. `meta` is the `ScriptInfo` snapshot taken at register.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredScript {
    pub name: String,
    pub source: String,
    pub meta: Value,
    pub source_hash: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A stored OAuth client registration.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredClient {
    pub client_id: String,
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
    /// When it last received a token; `None` if it never did.
    pub last_token_at: Option<i64>,
    /// As it registered: chosen by the client, so never trusted.
    pub client_name: Option<String>,
}

/// A signed-in user (ADR-0044). `id` is `<provider>:<the provider's immutable
/// user id>`; `name` is for display only.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredUser {
    pub id: String,
    pub name: String,
}

/// A browser session; only the hash of its cookie value is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSession {
    pub token_hash: String,
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

fn row_to_script(r: &rusqlite::Row) -> rusqlite::Result<StoredScript> {
    let meta: String = r.get(2)?;
    Ok(StoredScript {
        name: r.get(0)?,
        source: r.get(1)?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        source_hash: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

const COLUMNS: &str = "name, source, meta_json, source_hash, created_at, updated_at";

impl Store {
    /// Open (creating the file and its directory) and migrate. A missing
    /// directory is created owner-only (0700) and the file is 0600: it holds
    /// users' scripts and the OAuth client ids that own them.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        restrict(path);
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Insert or replace `(user, name)`; keeps the original `created_at`.
    pub fn upsert(
        &self,
        user: &str,
        name: &str,
        source: &str,
        meta: &Value,
        source_hash: &str,
    ) -> rusqlite::Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn().execute(
            "INSERT INTO user_indicators
                 (user_id, name, source, meta_json, source_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT (user_id, name) DO UPDATE SET
                 source = excluded.source, meta_json = excluded.meta_json,
                 source_hash = excluded.source_hash, updated_at = excluded.updated_at",
            params![user, name, source, meta.to_string(), source_hash, now],
        )?;
        Ok(())
    }

    pub fn get(&self, user: &str, name: &str) -> rusqlite::Result<Option<StoredScript>> {
        self.conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM user_indicators WHERE user_id = ?1 AND name = ?2"),
                params![user, name],
                row_to_script,
            )
            .optional()
    }

    /// The user's scripts, by name.
    pub fn list(&self, user: &str) -> rusqlite::Result<Vec<StoredScript>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM user_indicators WHERE user_id = ?1 ORDER BY name"
        ))?;
        let rows = stmt.query_map(params![user], row_to_script)?;
        rows.collect()
    }

    /// True when a row was removed.
    pub fn delete(&self, user: &str, name: &str) -> rusqlite::Result<bool> {
        let n = self.conn().execute(
            "DELETE FROM user_indicators WHERE user_id = ?1 AND name = ?2",
            params![user, name],
        )?;
        Ok(n > 0)
    }

    pub fn count(&self, user: &str) -> rusqlite::Result<usize> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM user_indicators WHERE user_id = ?1",
            params![user],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn save_oauth_client(
        &self,
        client_id: &str,
        redirect_uris: &[String],
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO oauth_clients (client_id, redirect_uris, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                client_id,
                serde_json::to_string(redirect_uris).unwrap_or_default(),
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub fn set_oauth_client_name(&self, client_id: &str, name: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE oauth_clients SET client_name = ?2 WHERE client_id = ?1",
            params![client_id, name],
        )?;
        Ok(())
    }

    /// Every stored OAuth client.
    pub fn oauth_clients(&self) -> rusqlite::Result<Vec<StoredClient>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT client_id, redirect_uris, created_at, last_token_at, client_name
             FROM oauth_clients",
        )?;
        let rows = stmt.query_map([], |r| {
            let uris: String = r.get(1)?;
            Ok(StoredClient {
                client_id: r.get(0)?,
                redirect_uris: serde_json::from_str(&uris).unwrap_or_default(),
                created_at: r.get(2)?,
                last_token_at: r.get(3)?,
                client_name: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Record that `client_id` received a token now.
    pub fn mark_oauth_client_used(&self, client_id: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE oauth_clients SET last_token_at = ?2 WHERE client_id = ?1",
            params![client_id, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn delete_oauth_client(&self, client_id: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM oauth_clients WHERE client_id = ?1",
            params![client_id],
        )?;
        Ok(())
    }

    /// The client ids that own at least one stored script.
    pub fn script_owners(&self) -> rusqlite::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT user_id FROM user_indicators")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Delete clients registered over `max_age_secs` ago that never received
    /// a token and own no scripts; returns how many.
    pub fn gc_oauth_clients(&self, max_age_secs: i64) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM oauth_clients WHERE last_token_at IS NULL AND created_at < ?1
                 AND client_id NOT IN (SELECT user_id FROM user_indicators)",
            params![chrono::Utc::now().timestamp() - max_age_secs],
        )
    }

    /// Insert the user, or refresh the display name and login time of a known one.
    pub fn save_user(&self, id: &str, name: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT INTO users (id, name, created_at, last_login_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 name = excluded.name, last_login_at = excluded.last_login_at",
            params![id, name, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn users(&self) -> rusqlite::Result<Vec<StoredUser>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name FROM users")?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredUser {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        rows.collect()
    }

    pub fn save_session(&self, session: &StoredSession) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO sessions (token_hash, user_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session.token_hash,
                session.user_id,
                session.created_at,
                session.expires_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, token_hash: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    /// The sessions still valid at `now`; expired ones are deleted.
    pub fn live_sessions(&self, now: i64) -> rusqlite::Result<Vec<StoredSession>> {
        let conn = self.conn();
        conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
        let mut stmt =
            conn.prepare("SELECT token_hash, user_id, created_at, expires_at FROM sessions")?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredSession {
                token_hash: r.get(0)?,
                user_id: r.get(1)?,
                created_at: r.get(2)?,
                expires_at: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn save_grant(&self, user_id: &str, client_id: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO client_grants (user_id, client_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![user_id, client_id, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn delete_grant(&self, user_id: &str, client_id: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM client_grants WHERE user_id = ?1 AND client_id = ?2",
            params![user_id, client_id],
        )?;
        Ok(())
    }

    /// `(user, client)` approvals, oldest first. Those of clients no longer
    /// registered are deleted: a client id is never issued twice.
    pub fn grants(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM client_grants WHERE client_id NOT IN (SELECT client_id FROM oauth_clients)",
            [],
        )?;
        let mut stmt = conn.prepare(
            "SELECT user_id, client_id FROM client_grants ORDER BY created_at, client_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Move scripts owned by `client_id` to `user_id`, newest first, up to `max`
    /// scripts in all for that user; returns how many moved. A name the user
    /// already has stays with the client, and so does what exceeds `max`, so
    /// nothing is overwritten or deleted (ADR-0044).
    pub fn claim_scripts(
        &self,
        client_id: &str,
        user_id: &str,
        max: usize,
    ) -> rusqlite::Result<usize> {
        self.conn().execute(
            "UPDATE OR IGNORE user_indicators SET user_id = ?2
             WHERE user_id = ?1 AND name IN (
                 SELECT name FROM user_indicators
                 WHERE user_id = ?1 AND name NOT IN
                     (SELECT name FROM user_indicators WHERE user_id = ?2)
                 ORDER BY updated_at DESC, name
                 LIMIT max(0, ?3 - (SELECT COUNT(*) FROM user_indicators WHERE user_id = ?2))
             )",
            params![client_id, user_id, max as i64],
        )
    }
}

/// Create `path`'s directory if missing (0700) and the file (0600) before
/// SQLite opens it, and tighten the file if it already exists. An existing
/// directory is not ours to change: `CHARTGEN_DB` may sit directly in `/tmp`.
fn restrict(path: &Path) {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    #[cfg(not(unix))]
    if let Some(dir) = dir {
        let _ = std::fs::create_dir_all(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        if let Some(dir) = dir {
            let _ = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir);
        }
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path);
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

/// `CHARTGEN_DB`, else `~/.chartgen/chartgen.db`.
pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("CHARTGEN_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".chartgen").join("chartgen.db")
}

static STORE: OnceLock<Option<Store>> = OnceLock::new();

/// The process-wide store at `path`, opened (and migrated) on first use;
/// later calls return that same store whatever `path` they pass. `None`
/// (logged once) when it cannot be opened.
pub fn open_global(path: &Path) -> Option<&'static Store> {
    STORE
        .get_or_init(|| match Store::open(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("[store] cannot open {}: {e}", path.display());
                None
            }
        })
        .as_ref()
}

/// The process-wide store at `db_path()`.
pub fn global() -> Option<&'static Store> {
    open_global(&db_path())
}
