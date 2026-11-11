//! SQLite store for user indicators and registered OAuth clients (ADR-0025).
//!
//! One file, `CHARTGEN_DB` (default `~/.chartgen/chartgen.db`), migrated on
//! open. OAuth clients are stored so a client keeps its `client_id` (the
//! script owner's identity) across server restarts, with `CHARTGEN_SCRIPTING`
//! or without (ADR-0036). With sign-in on, users, their browser sessions and
//! the clients they approved live here too (ADR-0044).

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Schema migrations; `PRAGMA user_version` is the number applied.
pub(super) const MIGRATIONS: &[&str] = &[
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
    CREATE TABLE server_alerts (
        id           TEXT PRIMARY KEY,
        owner        TEXT NOT NULL,
        record_json  TEXT NOT NULL
    );
    CREATE INDEX server_alerts_owner ON server_alerts (owner);
    CREATE TABLE server_alert_events (
        id          TEXT    PRIMARY KEY,
        alert_id    TEXT    NOT NULL,
        body        TEXT    NOT NULL,
        created_at  INTEGER NOT NULL
    );
",
    "
    ALTER TABLE server_alert_events ADD COLUMN device TEXT;
    CREATE TABLE push_devices (
        id          TEXT    PRIMARY KEY,
        owner       TEXT    NOT NULL,
        endpoint    TEXT    NOT NULL,
        p256dh      TEXT    NOT NULL,
        auth        TEXT    NOT NULL,
        created_at  INTEGER NOT NULL,
        UNIQUE (owner, endpoint)
    );
    CREATE TABLE server_settings (
        name   TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    );
",
    "
    ALTER TABLE oauth_clients ADD COLUMN client_name TEXT;
    CREATE TABLE users (
        id             TEXT    PRIMARY KEY,
        name           TEXT    NOT NULL,
        created_at     INTEGER NOT NULL,
        last_login_at  INTEGER NOT NULL,
        last_token_at  INTEGER
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
    "
    CREATE TABLE workspace_docs (
        user_id     TEXT    NOT NULL,
        key         TEXT    NOT NULL,
        value       TEXT,
        version     INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        PRIMARY KEY (user_id, key)
    );
    CREATE INDEX workspace_docs_version ON workspace_docs (user_id, version);
    CREATE TABLE workspace_users (
        user_id  TEXT    PRIMARY KEY,
        seq      INTEGER NOT NULL,
        bytes    INTEGER NOT NULL
    );
",
    "
    CREATE TABLE refresh_tokens (
        token_hash  TEXT    PRIMARY KEY,
        family      TEXT    NOT NULL,
        seq         INTEGER NOT NULL,
        client_id   TEXT    NOT NULL,
        user_id     TEXT,
        created_at  INTEGER NOT NULL,
        spent_at    INTEGER
    );
    CREATE INDEX refresh_tokens_family ON refresh_tokens (family);
",
    "
    ALTER TABLE refresh_tokens ADD COLUMN access_hash TEXT;
",
    "
    CREATE TABLE setups (
        id      TEXT PRIMARY KEY,
        owner   TEXT NOT NULL,
        record  TEXT NOT NULL
    );
    CREATE INDEX setups_owner ON setups (owner);
    CREATE TABLE setup_revisions (
        setup_id  TEXT    NOT NULL,
        seq       INTEGER NOT NULL,
        body      TEXT    NOT NULL,
        PRIMARY KEY (setup_id, seq)
    );
    CREATE TRIGGER setup_revisions_no_update BEFORE UPDATE ON setup_revisions
    BEGIN
        SELECT RAISE(ABORT, 'a setup''s log is append-only');
    END;
    CREATE TRIGGER setup_revisions_no_delete BEFORE DELETE ON setup_revisions
    BEGIN
        SELECT RAISE(ABORT, 'a setup''s log is append-only');
    END;
    CREATE TRIGGER setup_revisions_no_replace BEFORE INSERT ON setup_revisions
    WHEN EXISTS (SELECT 1 FROM setup_revisions
                 WHERE setup_id = NEW.setup_id AND seq = NEW.seq)
    BEGIN
        SELECT RAISE(ABORT, 'a setup''s log is append-only');
    END;
",
];

/// Whether an owner is a signed-in account (ADR-0044).
pub fn is_account(owner: &str) -> bool {
    owner.starts_with("github:") || owner.starts_with("google:")
}

/// Whether an owner has the shape `/register` gives a client id, a hyphenated UUID, and
/// so is what `CHARTGEN_CLAIM_OWNER` collects. A positive test: an owner of any other
/// kind, now or later, is not swept into someone's account (ADR-0044).
pub fn is_client_id(owner: &str) -> bool {
    owner.len() == 36 && uuid::Uuid::try_parse(owner).is_ok()
}

/// A setup to write: its JSON and the log entries it gained, as `(seq, JSON)`.
pub struct StoredSetup {
    pub id: String,
    pub owner: String,
    pub record: String,
    pub revisions: Vec<(u32, String)>,
}

/// What `Store::waiting_rows` counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaitingRows {
    pub scripts: usize,
    pub alerts: usize,
    pub devices: usize,
    /// Of `scripts`, those whose name the named owner already has, so they cannot move.
    pub name_collisions: usize,
}

/// What one account may keep in its workspace (ADR-0050).
#[derive(Clone, Copy, Debug)]
pub struct WorkspaceLimits {
    pub doc_bytes: usize,
    pub user_bytes: i64,
    pub user_docs: i64,
    /// Deleted documents remembered per user, the newest. They are outside `user_docs`.
    pub user_tombstones: i64,
    pub total_bytes: i64,
}

/// A write to one document: `value` `None` deletes it, leaving a tombstone so that
/// other devices learn of it. `base_version` is the version it was made against.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceWrite {
    pub key: String,
    pub value: Option<String>,
    pub base_version: i64,
}

/// What became of one write.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceOutcome {
    Stored {
        version: i64,
    },
    /// The document moved on: here is what it holds now, to merge with.
    Conflict {
        version: i64,
        value: Option<String>,
    },
    TooLarge,
    OverQuota,
}

/// A document as it stands.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WorkspaceDoc {
    pub key: String,
    pub value: Option<String>,
    pub version: i64,
}

/// The answer to a sync.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSynced {
    pub outcomes: Vec<WorkspaceOutcome>,
    pub changes: Vec<WorkspaceDoc>,
    pub cursor: i64,
    /// The caller's cursor is past this account's versions: the file was restored or the
    /// account emptied. `changes` then holds all there is, deletions included.
    pub restored: bool,
}

/// How long a deletion is remembered. A device away for longer can bring it back.
const WORKSPACE_TOMBSTONE_SECS: i64 = 90 * 86_400;

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

/// An account as the operator page shows it (ADR-0063).
#[derive(Clone, Debug, PartialEq)]
pub struct UserSummary {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_login_at: i64,
    pub last_token_at: Option<i64>,
    pub workspace_docs: i64,
    pub workspace_bytes: i64,
    pub setups: i64,
    pub server_alerts: i64,
    /// Refresh token families: applications that keep working without the browser.
    pub authorisations: i64,
    pub push_devices: i64,
}

/// A registered client as the operator page shows it (ADR-0063).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSummary {
    pub id: String,
    pub name: Option<String>,
    pub created_at: i64,
    pub last_token_at: Option<i64>,
    pub has_family: bool,
    pub owns_setups: bool,
}

/// The store as the operator page shows it (ADR-0063).
#[derive(Clone, Debug, PartialEq)]
pub struct StoreSummary {
    pub users_total: i64,
    pub users: Vec<UserSummary>,
    pub clients_total: i64,
    pub clients: Vec<ClientSummary>,
    pub setups_total: i64,
    pub pending_deliveries: i64,
    pub schema_version: i64,
}

/// A browser session; only the hash of its cookie value is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSession {
    pub token_hash: String,
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// A refresh token (ADR-0058); only the hash of its value is stored. A family is one
/// authorization: its tokens follow each other by `seq`, and at most one is unspent.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredRefreshToken {
    pub token_hash: String,
    pub family: String,
    pub seq: i64,
    pub client_id: String,
    /// `None` when issued with sign-in off.
    pub user_id: Option<String>,
    pub created_at: i64,
    /// When it was exchanged, or 0 if a retry replaced it before it was exchanged;
    /// kept so that using it again is recognised.
    pub spent_at: Option<i64>,
    /// The hash of the access token issued with it, while it is unspent: what lets
    /// that access token outlive a restart.
    pub access_hash: Option<String>,
}

/// A delivery not finished yet: written with the fire it reports, so a restart
/// resumes it and never fires again to get it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredEvent {
    pub id: String,
    pub alert_id: String,
    pub body: String,
    pub created_at: i64,
    /// The push device it goes to (`StoredDevice::id`); `None` is the alert's webhook.
    pub device: Option<String>,
}

/// A browser's push subscription (ADR-0045). Not `Debug`: the endpoint and keys are
/// what it takes to notify that device.
#[derive(Clone, PartialEq)]
pub struct StoredDevice {
    pub id: String,
    pub owner: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    /// Unix milliseconds.
    pub created_at: i64,
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
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::migrated(conn)
    }

    /// A store that lives in memory and is gone with the process: a replay's setups
    /// (ADR-0059). No file is touched.
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::migrated(Connection::open_in_memory()?)
    }

    fn migrated(mut conn: Connection) -> rusqlite::Result<Self> {
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

    /// Insert `(user, name)` only if it does not exist; false when it does.
    /// The check and the write are one statement, so two creators cannot both win.
    pub fn insert_new(
        &self,
        user: &str,
        name: &str,
        source: &str,
        meta: &Value,
        source_hash: &str,
    ) -> rusqlite::Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let inserted = self.conn().execute(
            "INSERT INTO user_indicators
                 (user_id, name, source, meta_json, source_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT (user_id, name) DO NOTHING",
            params![user, name, source, meta.to_string(), source_hash, now],
        )?;
        Ok(inserted == 1)
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

    /// Delete a client, and its refresh tokens with it (ADR-0058).
    pub fn delete_oauth_client(&self, client_id: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM refresh_tokens WHERE client_id = ?1",
            params![client_id],
        )?;
        tx.execute(
            "DELETE FROM oauth_clients WHERE client_id = ?1",
            params![client_id],
        )?;
        tx.commit()
    }

    /// Store `new` and mark `spent` as used at `spent_at`, in one transaction: a
    /// rotation is on disk whole or not at all. `spent_at` is the time of the exchange,
    /// or 0 for a token that was replaced before it was ever exchanged. Spent tokens of
    /// the family below `forget_below_seq` are dropped, and with `spent_at` 0 the ones
    /// replaced earlier. An error if a token of `spent`
    /// is not a usable one here: the server spends only what it holds as usable, so
    /// the file was changed from outside, and a successor would revive what was
    /// deleted.
    pub fn save_refresh_token(
        &self,
        new: &StoredRefreshToken,
        spent: &[String],
        spent_at: i64,
        forget_below_seq: i64,
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        // A retry replaces a successor, and only the newest replaced one is kept:
        // before this one is marked, so that the count below is of this one alone.
        if spent_at == 0 {
            tx.execute(
                "DELETE FROM refresh_tokens WHERE family = ?1 AND spent_at = 0",
                params![new.family],
            )?;
        }
        for hash in spent {
            let usable = tx.execute(
                "UPDATE refresh_tokens SET spent_at = ?2, access_hash = NULL
                 WHERE token_hash = ?1 AND spent_at IS NULL",
                params![hash, spent_at],
            )?;
            if usable != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }
        tx.execute(
            "DELETE FROM refresh_tokens WHERE family = ?1 AND spent_at IS NOT NULL AND seq < ?2",
            params![new.family, forget_below_seq],
        )?;
        tx.execute(
            "INSERT INTO refresh_tokens
                 (token_hash, family, seq, client_id, user_id, created_at, spent_at, access_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                new.token_hash,
                new.family,
                new.seq,
                new.client_id,
                new.user_id,
                new.created_at,
                new.spent_at,
                new.access_hash
            ],
        )?;
        tx.commit()
    }

    pub fn delete_refresh_families(&self, families: &[String]) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for family in families {
            tx.execute(
                "DELETE FROM refresh_tokens WHERE family = ?1",
                params![family],
            )?;
        }
        tx.commit()
    }

    /// Every stored refresh token, in the order they were stored; families whose
    /// newest token is older than `cutoff` are deleted first, and of several unspent
    /// tokens in one family all but the one stored last are marked replaced.
    pub fn refresh_tokens(&self, cutoff: i64) -> rusqlite::Result<Vec<StoredRefreshToken>> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM refresh_tokens WHERE family IN
                 (SELECT family FROM refresh_tokens GROUP BY family HAVING MAX(created_at) < ?1)",
            params![cutoff],
        )?;
        // A family has one unspent token. A build before the one that finishes a
        // request whose client hung up could store a successor twice. The client
        // holds the one stored last, so the others are marked replaced, here in the
        // file: left as they are, every start would have to tell them apart again.
        conn.execute(
            "UPDATE refresh_tokens SET spent_at = 0, access_hash = NULL
             WHERE spent_at IS NULL AND rowid <
                 (SELECT MAX(rowid) FROM refresh_tokens AS newer
                  WHERE newer.family = refresh_tokens.family AND newer.spent_at IS NULL)",
            [],
        )?;
        let mut stmt = conn.prepare(
            "SELECT token_hash, family, seq, client_id, user_id, created_at, spent_at, access_hash
             FROM refresh_tokens ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredRefreshToken {
                token_hash: r.get(0)?,
                family: r.get(1)?,
                seq: r.get(2)?,
                client_id: r.get(3)?,
                user_id: r.get(4)?,
                created_at: r.get(5)?,
                spent_at: r.get(6)?,
                access_hash: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// The client ids that own at least one stored script.
    pub fn script_owners(&self) -> rusqlite::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT user_id FROM user_indicators")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// The client ids that own at least one server alert.
    pub fn server_alert_owners(&self) -> rusqlite::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT owner FROM server_alerts")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// The owners of at least one setup, closed ones included (ADR-0059).
    pub fn setup_owners(&self) -> rusqlite::Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT owner FROM setups")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Delete clients registered over `max_age_secs` ago that never received
    /// a token and own no scripts; returns how many. An owner of a server alert
    /// has received one.
    pub fn gc_oauth_clients(&self, max_age_secs: i64) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM oauth_clients WHERE last_token_at IS NULL AND created_at < ?1
                 AND client_id NOT IN (SELECT user_id FROM user_indicators)",
            params![chrono::Utc::now().timestamp() - max_age_secs],
        )
    }

    /// Insert or replace a server alert; `record` is its JSON.
    pub fn save_server_alert(&self, id: &str, owner: &str, record: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO server_alerts (id, owner, record_json) VALUES (?1, ?2, ?3)",
            params![id, owner, record],
        )?;
        Ok(())
    }

    /// The JSON of every server alert.
    pub fn server_alerts(&self) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT record_json FROM server_alerts")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// Delete a server alert and whatever it still had to deliver.
    pub fn delete_server_alert(&self, id: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM server_alerts WHERE id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM server_alert_events WHERE alert_id = ?1",
            params![id],
        )?;
        tx.commit()
    }

    /// Write a fired alert's new state and, in one transaction, its deliveries.
    pub fn record_server_alert_fire(
        &self,
        id: &str,
        owner: &str,
        record: &str,
        events: &[StoredEvent],
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO server_alerts (id, owner, record_json) VALUES (?1, ?2, ?3)",
            params![id, owner, record],
        )?;
        for e in events {
            tx.execute(
                "INSERT INTO server_alert_events (id, alert_id, body, created_at, device)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.id, e.alert_id, e.body, e.created_at, e.device],
            )?;
        }
        tx.commit()
    }

    /// Write setups and the entries their logs gained, in one transaction (ADR-0059).
    /// A log entry is only ever inserted: one that exists already fails the write.
    pub fn save_setups(&self, setups: &[StoredSetup]) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for s in setups {
            tx.execute(
                "INSERT OR REPLACE INTO setups (id, owner, record) VALUES (?1, ?2, ?3)",
                params![s.id, s.owner, s.record],
            )?;
            for (seq, body) in &s.revisions {
                tx.execute(
                    "INSERT INTO setup_revisions (setup_id, seq, body) VALUES (?1, ?2, ?3)",
                    params![s.id, seq, body],
                )?;
            }
        }
        tx.commit()
    }

    /// Every setup's JSON, with the bytes of its log.
    pub fn setups(&self) -> rusqlite::Result<Vec<(String, usize)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT record, (SELECT COALESCE(SUM(LENGTH(CAST(body AS BLOB))), 0)
                             FROM setup_revisions WHERE setup_id = setups.id)
             FROM setups ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?;
        rows.collect()
    }

    /// The log of one setup, oldest first.
    pub fn setup_revisions(&self, id: &str) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT body FROM setup_revisions WHERE setup_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map(params![id], |r| r.get(0))?;
        rows.collect()
    }

    /// Deliveries not finished, oldest first.
    pub fn server_alert_events(&self) -> rusqlite::Result<Vec<StoredEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, alert_id, body, created_at, device FROM server_alert_events
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredEvent {
                id: r.get(0)?,
                alert_id: r.get(1)?,
                body: r.get(2)?,
                created_at: r.get(3)?,
                device: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_server_alert_event(&self, id: &str) -> rusqlite::Result<()> {
        self.conn()
            .execute("DELETE FROM server_alert_events WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Insert or replace a push device.
    pub fn save_push_device(&self, d: &StoredDevice) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO push_devices (id, owner, endpoint, p256dh, auth, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![d.id, d.owner, d.endpoint, d.p256dh, d.auth, d.created_at],
        )?;
        Ok(())
    }

    pub fn push_devices(&self) -> rusqlite::Result<Vec<StoredDevice>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, owner, endpoint, p256dh, auth, created_at FROM push_devices")?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredDevice {
                id: r.get(0)?,
                owner: r.get(1)?,
                endpoint: r.get(2)?,
                p256dh: r.get(3)?,
                auth: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Delete a push device and whatever it still had to receive.
    pub fn delete_push_device(&self, id: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM push_devices WHERE id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM server_alert_events WHERE device = ?1",
            params![id],
        )?;
        tx.commit()
    }

    /// The setting `name`, set to `value` first if it has none. One statement decides,
    /// so two starts cannot each keep a different key.
    pub fn setting_or_insert(&self, name: &str, value: &str) -> rusqlite::Result<String> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO server_settings (name, value) VALUES (?1, ?2)",
            params![name, value],
        )?;
        conn.query_row(
            "SELECT value FROM server_settings WHERE name = ?1",
            params![name],
            |r| r.get(0),
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

    /// Record that `id` received a token now: what keeps their server alerts alive,
    /// as `last_token_at` does for a client's (ADR-0044).
    pub fn mark_user_used(&self, id: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE users SET last_token_at = ?2 WHERE id = ?1",
            params![id, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// When each user last signed in or received a token, in unix seconds.
    pub fn user_activity(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, max(last_login_at, COALESCE(last_token_at, 0)) FROM users")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// What the operator page shows of the store (ADR-0063): the `users` accounts that
    /// signed in last and the `clients` registered last, with counts of what each has,
    /// and totals. A fixed number of statements whatever the store holds, none per row
    /// of another table that is not found by an index. **Counts and times only:** no
    /// token, hash, record, document, endpoint or redirect is read.
    pub fn operator_summary(&self, users: usize, clients: usize) -> rusqlite::Result<StoreSummary> {
        let conn = self.conn();
        let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
        let mut families: HashMap<String, i64> = HashMap::new();
        let mut with_family: HashSet<String> = HashSet::new();
        {
            let mut stmt = conn.prepare(
                "SELECT user_id, client_id, COUNT(DISTINCT family) FROM refresh_tokens
                 GROUP BY user_id, client_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (user, client, n) = row?;
                if let Some(user) = user {
                    *families.entry(user).or_default() += n;
                }
                with_family.insert(client);
            }
        }
        let mut stmt = conn.prepare(
            "SELECT u.id, u.name, u.created_at, u.last_login_at, u.last_token_at,
                (SELECT COUNT(*) FROM workspace_docs d
                  WHERE d.user_id = u.id AND d.value IS NOT NULL),
                COALESCE((SELECT bytes FROM workspace_users w WHERE w.user_id = u.id), 0),
                (SELECT COUNT(*) FROM setups s WHERE s.owner = u.id),
                (SELECT COUNT(*) FROM server_alerts a WHERE a.owner = u.id),
                (SELECT COUNT(*) FROM push_devices p WHERE p.owner = u.id)
             FROM users u ORDER BY u.last_login_at DESC, u.id LIMIT ?1",
        )?;
        let shown_users = stmt
            .query_map([users as i64], |r| {
                let id: String = r.get(0)?;
                Ok(UserSummary {
                    authorisations: families.get(&id).copied().unwrap_or(0),
                    id,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    last_login_at: r.get(3)?,
                    last_token_at: r.get(4)?,
                    workspace_docs: r.get(5)?,
                    workspace_bytes: r.get(6)?,
                    setups: r.get(7)?,
                    server_alerts: r.get(8)?,
                    push_devices: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(
            "SELECT c.client_id, c.client_name, c.created_at, c.last_token_at,
                EXISTS (SELECT 1 FROM setups s WHERE s.owner = c.client_id)
             FROM oauth_clients c ORDER BY c.created_at DESC, c.client_id LIMIT ?1",
        )?;
        let shown_clients = stmt
            .query_map([clients as i64], |r| {
                let id: String = r.get(0)?;
                Ok(ClientSummary {
                    has_family: with_family.contains(&id),
                    id,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    last_token_at: r.get(3)?,
                    owns_setups: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(StoreSummary {
            users_total: count("SELECT COUNT(*) FROM users")?,
            users: shown_users,
            clients_total: count("SELECT COUNT(*) FROM oauth_clients")?,
            clients: shown_clients,
            setups_total: count("SELECT COUNT(*) FROM setups")?,
            pending_deliveries: count("SELECT COUNT(*) FROM server_alert_events")?,
            schema_version: count("PRAGMA user_version")?,
        })
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

    /// Move every script owned by an OAuth client to `user_id`; returns how many moved.
    /// A name the user already has stays with its client, as does the second of two
    /// clients' scripts with one name, so nothing is overwritten or deleted (ADR-0044).
    pub fn claim_waiting_scripts(&self, user_id: &str) -> rusqlite::Result<usize> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT user_id FROM user_indicators")?;
        let owners: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut moved = 0;
        for owner in owners.iter().filter(|o| is_client_id(o)) {
            moved += conn.execute(
                "UPDATE OR IGNORE user_indicators SET user_id = ?1 WHERE user_id = ?2",
                params![user_id, owner],
            )?;
        }
        Ok(moved)
    }

    /// What OAuth clients still own: from before sign-in, and out of everyone's reach
    /// until `CHARTGEN_CLAIM_OWNER` collects it. `owner` is that account, if one is named.
    pub fn waiting_rows(&self, owner: Option<&str>) -> rusqlite::Result<WaitingRows> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT user_id, name FROM user_indicators")?;
        let scripts: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let taken: HashSet<&str> = scripts
            .iter()
            .filter(|(o, _)| Some(o.as_str()) == owner)
            .map(|(_, name)| name.as_str())
            .collect();
        let waiting = || scripts.iter().filter(|(o, _)| is_client_id(o));
        let clients_in = |table: &str| -> rusqlite::Result<usize> {
            let mut stmt = conn.prepare(&format!("SELECT owner FROM {table}"))?;
            let owners: Vec<String> = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(owners.iter().filter(|o| is_client_id(o)).count())
        };
        Ok(WaitingRows {
            scripts: waiting().count(),
            alerts: clients_in("server_alerts")?,
            devices: clients_in("push_devices")?,
            name_collisions: waiting()
                .filter(|(_, name)| taken.contains(name.as_str()))
                .count(),
        })
    }

    /// Apply `writes` for `user` and answer with what became of each, the documents
    /// changed since `since` other than those just stored, and the new cursor. Every
    /// statement names the user: this is the isolation between accounts (ADR-0050). One
    /// transaction, so a failure leaves nothing half done.
    pub fn workspace_sync(
        &self,
        user: &str,
        since: i64,
        writes: &[WorkspaceWrite],
        limits: &WorkspaceLimits,
    ) -> rusqlite::Result<WorkspaceSynced> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp();
        let (mut seq, mut bytes): (i64, i64) = tx
            .query_row(
                "SELECT seq, bytes FROM workspace_users WHERE user_id = ?1",
                params![user],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        // A cursor from before this file was restored or the account emptied.
        let restored = since > seq;
        // A device with no cursor has nothing to delete, so it is told of no deletions.
        let with_tombstones = since > 0;
        let since = if restored { 0 } else { since };
        if !writes.is_empty() {
            tx.execute(
                "DELETE FROM workspace_docs
                 WHERE user_id = ?1 AND value IS NULL AND updated_at < ?2",
                params![user, now - WORKSPACE_TOMBSTONE_SECS],
            )?;
        }
        let mut docs: i64 = tx.query_row(
            "SELECT COUNT(*) FROM workspace_docs WHERE user_id = ?1 AND value IS NOT NULL",
            params![user],
            |r| r.get(0),
        )?;
        let mut total: i64 = tx.query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM workspace_users",
            [],
            |r| r.get(0),
        )?;
        let mut outcomes = Vec::with_capacity(writes.len());
        let mut stored = HashSet::new();
        for write in writes {
            let current: Option<(Option<String>, i64)> = tx
                .query_row(
                    "SELECT value, version FROM workspace_docs WHERE user_id = ?1 AND key = ?2",
                    params![user, write.key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (value, version) = current.unwrap_or((None, 0));
            // Already so, as a retry after a lost answer or another tab's same write is.
            if value == write.value {
                stored.insert(write.key.as_str());
                outcomes.push(WorkspaceOutcome::Stored { version });
                continue;
            }
            if write.base_version != version {
                outcomes.push(WorkspaceOutcome::Conflict { version, value });
                continue;
            }
            let new_len = write.value.as_ref().map_or(0, String::len);
            if new_len > limits.doc_bytes {
                outcomes.push(WorkspaceOutcome::TooLarge);
                continue;
            }
            let delta = new_len as i64 - value.as_ref().map_or(0, String::len) as i64;
            let docs_delta = i64::from(write.value.is_some()) - i64::from(value.is_some());
            // The last tenth of the total is for accounts already here, so that a file
            // filling up with new accounts leaves those working for a while yet.
            let room = if docs == 0 {
                limits.total_bytes - limits.total_bytes / 10
            } else {
                limits.total_bytes
            };
            if (delta > 0 && (bytes + delta > limits.user_bytes || total + delta > room))
                || (docs_delta > 0 && docs >= limits.user_docs)
            {
                outcomes.push(WorkspaceOutcome::OverQuota);
                continue;
            }
            seq += 1;
            bytes += delta;
            total += delta;
            docs += docs_delta;
            tx.execute(
                "INSERT INTO workspace_docs (user_id, key, value, version, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (user_id, key) DO UPDATE SET
                     value = excluded.value, version = excluded.version,
                     updated_at = excluded.updated_at",
                params![user, write.key, write.value, seq, now],
            )?;
            stored.insert(write.key.as_str());
            outcomes.push(WorkspaceOutcome::Stored { version: seq });
        }
        tx.execute(
            "INSERT INTO workspace_users (user_id, seq, bytes) VALUES (?1, ?2, ?3)
             ON CONFLICT (user_id) DO UPDATE SET seq = excluded.seq, bytes = excluded.bytes",
            params![user, seq, bytes],
        )?;
        if !writes.is_empty() {
            tx.execute(
                "DELETE FROM workspace_docs
                 WHERE user_id = ?1 AND value IS NULL AND version NOT IN (
                     SELECT version FROM workspace_docs
                     WHERE user_id = ?1 AND value IS NULL
                     ORDER BY version DESC LIMIT ?2)",
                params![user, limits.user_tombstones],
            )?;
        }
        let changes = {
            let mut stmt = tx.prepare(
                "SELECT key, value, version FROM workspace_docs
                 WHERE user_id = ?1 AND version > ?2 AND (?3 OR value IS NOT NULL)
                 ORDER BY version",
            )?;
            let rows = stmt.query_map(params![user, since, with_tombstones], |r| {
                Ok(WorkspaceDoc {
                    key: r.get(0)?,
                    value: r.get(1)?,
                    version: r.get(2)?,
                })
            })?;
            rows.filter(|d| {
                d.as_ref()
                    .map_or(true, |d| !stored.contains(d.key.as_str()))
            })
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(WorkspaceSynced {
            outcomes,
            changes,
            cursor: seq,
            restored,
        })
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

/// Whether asking for the store may open it. Only `main` says so, and no test runs
/// `main`: a test that reaches code asking for the store must not open, and migrate,
/// the developer's own `~/.chartgen/chartgen.db`.
static OPENS_ON_DEMAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Let `global()` open the store at `db_path()` when it is first asked for. For the
/// start of a real process.
pub fn open_on_demand() {
    OPENS_ON_DEMAND.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Make the process-wide store one that lives in memory and touches no file, for a
/// replay server (ADR-0059). At startup, before anything asks for the store; false if
/// a store was opened already, or this one could not be.
pub fn open_global_in_memory() -> bool {
    let mut ours = false;
    STORE.get_or_init(|| {
        ours = true;
        Store::in_memory().ok()
    });
    ours && STORE.get().is_some_and(Option::is_some)
}

/// The process-wide store: the one at `db_path()`, opened on first use, in a process
/// that called `open_on_demand()`. Anywhere else, only a store that `open_global`
/// already opened, and never a file of its own choosing.
pub fn global() -> Option<&'static Store> {
    if OPENS_ON_DEMAND.load(std::sync::atomic::Ordering::Relaxed) {
        return open_global(&db_path());
    }
    STORE.get().and_then(Option::as_ref)
}

#[cfg(test)]
mod operator_summary_tests {
    use super::*;

    /// A store with two accounts, three clients and something of every kind the page
    /// counts, each row carrying a value that must never be read.
    fn planted() -> Store {
        let store = Store::in_memory().unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO users VALUES ('github:1000001', 'Ann', 10, 50, 60);
                 INSERT INTO users VALUES ('google:2000002', 'Bob', 20, 40, NULL);
                 INSERT INTO oauth_clients (client_id, redirect_uris, created_at, last_token_at, client_name)
                   VALUES ('client-old', '[\"https://secret-redirect.example/cb\"]', 1, NULL, NULL),
                          ('client-mid', '[]', 2, 30, 'An agent'),
                          ('client-new', '[]', 3, NULL, 'Newest');
                 INSERT INTO workspace_docs VALUES ('github:1000001', 'k1', 'SECRET-BODY', 1, 1),
                                                   ('github:1000001', 'k2', NULL, 2, 2);
                 INSERT INTO workspace_users VALUES ('github:1000001', 2, 11);
                 INSERT INTO setups VALUES ('s1', 'github:1000001', '{\"thesis\":\"SECRET-THESIS\"}'),
                                           ('s2', 'github:1000001', '{}'),
                                           ('s3', 'client-mid', '{}');
                 INSERT INTO server_alerts VALUES ('a1', 'google:2000002', '{\"webhook_url\":\"https://secret-hook.example\"}');
                 INSERT INTO server_alert_events (id, alert_id, body, created_at) VALUES ('e1', 'a1', 'SECRET-EVENT', 1);
                 INSERT INTO push_devices VALUES ('d1', 'github:1000001', 'https://secret-push.example', 'k', 'a', 1);
                 INSERT INTO refresh_tokens (token_hash, family, seq, client_id, user_id, created_at)
                   VALUES ('SECRET-HASH-1', 'f1', 1, 'client-mid', 'github:1000001', 1),
                          ('SECRET-HASH-2', 'f1', 2, 'client-mid', 'github:1000001', 2),
                          ('SECRET-HASH-3', 'f2', 1, 'client-new', 'github:1000001', 3);",
            )
            .unwrap();
        store
    }

    #[test]
    fn the_operator_summary_counts_what_each_account_and_client_has() {
        let summary = planted().operator_summary(200, 50).unwrap();
        assert_eq!(summary.users_total, 2);
        assert_eq!(
            summary.users,
            [
                UserSummary {
                    id: "github:1000001".into(),
                    name: "Ann".into(),
                    created_at: 10,
                    last_login_at: 50,
                    last_token_at: Some(60),
                    workspace_docs: 1,
                    workspace_bytes: 11,
                    setups: 2,
                    server_alerts: 0,
                    authorisations: 2,
                    push_devices: 1,
                },
                UserSummary {
                    id: "google:2000002".into(),
                    name: "Bob".into(),
                    created_at: 20,
                    last_login_at: 40,
                    last_token_at: None,
                    workspace_docs: 0,
                    workspace_bytes: 0,
                    setups: 0,
                    server_alerts: 1,
                    authorisations: 0,
                    push_devices: 0,
                },
            ]
        );
        assert_eq!(summary.clients_total, 3);
        let clients: Vec<_> = summary
            .clients
            .iter()
            .map(|c| {
                (
                    c.id.as_str(),
                    c.name.as_deref(),
                    c.has_family,
                    c.owns_setups,
                )
            })
            .collect();
        assert_eq!(
            clients,
            [
                ("client-new", Some("Newest"), true, false),
                ("client-mid", Some("An agent"), true, true),
                ("client-old", None, false, false),
            ]
        );
        assert_eq!(summary.setups_total, 3);
        assert_eq!(summary.pending_deliveries, 1);
        assert_eq!(summary.schema_version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn the_operator_summary_shows_the_newest_only_and_still_counts_all() {
        let summary = planted().operator_summary(1, 2).unwrap();
        assert_eq!(summary.users_total, 2);
        assert_eq!(summary.users.len(), 1);
        assert_eq!(summary.users[0].id, "github:1000001");
        assert_eq!(summary.clients_total, 3);
        assert_eq!(summary.clients.len(), 2);
    }

    #[test]
    fn the_operator_summary_carries_nothing_that_is_secret_or_content() {
        let text = format!("{:?}", planted().operator_summary(200, 50).unwrap());
        for planted in ["SECRET", "secret-", "thesis", "webhook"] {
            assert!(!text.contains(planted), "{planted} in {text}");
        }
    }
}
