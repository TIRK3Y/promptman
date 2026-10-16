//! SQLite storage: runs and their results, the prompt library, and settings.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::llm::{Message, Params, Role};
use super::providers::ModelRef;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResultRecord {
    pub id: Option<i64>,
    /// The user stopped this request. Kept in memory only; the saved note says so.
    #[serde(default)]
    pub stopped: bool,
    pub slot: usize,
    pub provider: String,
    pub model: String,
    pub output: String,
    pub error: Option<String>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    /// Token counts were estimated locally because the provider didn't report them.
    pub tokens_estimated: bool,
    pub latency_ms: f64,
    pub ttft_ms: Option<f64>,
    pub tokens_per_sec: f64,
    /// `None` when the model has no entry in the pricing file.
    pub cost_usd: Option<f64>,
    pub is_local: bool,
    pub note: Option<String>,
    pub cpu_peak: Option<f32>,
    pub ram_peak_mb: Option<f64>,
    pub gpu_peak: Option<f32>,
    pub rating: Option<u8>,
    pub judge_score: Option<f32>,
    pub judge_reason: Option<String>,
}

impl ResultRecord {
    pub fn model_ref(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: Option<i64>,
    pub created_at: String,
    /// `compare`, `batch`, `cli` or `imported`.
    pub kind: String,
    pub prompt: String,
    pub system: String,
    pub params: Params,
    /// Earlier turns when this run continued a conversation.
    pub history: Vec<Message>,
}

/// One row of the history list.
#[derive(Clone, Debug)]
pub struct HistoryRow {
    pub run_id: i64,
    pub kind: String,
    pub created_at: String,
    pub prompt: String,
    pub model: String,
    pub cost_usd: Option<f64>,
    pub latency_ms: f64,
    pub tokens: usize,
    pub is_local: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ModelStats {
    pub model: String,
    pub runs: usize,
    pub errors: usize,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
    pub avg_ttft_ms: Option<f64>,
    pub avg_tokens_per_sec: f64,
    pub avg_rating: Option<f64>,
    pub rated: usize,
    /// Runs where this model got the single highest rating / runs it was rated against others.
    pub wins: usize,
    pub contests: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptRecord {
    pub id: i64,
    pub name: String,
    pub version: i64,
    pub tags: String,
    pub system: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct ConversationRow {
    pub id: i64,
    pub title: String,
    pub model: ModelRef,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct ChatMessage {
    pub id: Option<i64>,
    pub role: Role,
    pub content: String,
    /// Model that wrote an assistant message.
    pub model: Option<ModelRef>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cost_usd: Option<f64>,
    pub latency_ms: f64,
    pub error: Option<String>,
    pub note: Option<String>,
}

pub struct Db {
    conn: Mutex<Connection>,
}

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    prompt TEXT NOT NULL,
    system TEXT NOT NULL DEFAULT '',
    params TEXT NOT NULL DEFAULT '{}',
    history TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS results (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    slot INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    output TEXT NOT NULL DEFAULT '',
    error TEXT,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    tokens_estimated INTEGER NOT NULL DEFAULT 0,
    latency_ms REAL NOT NULL DEFAULT 0,
    ttft_ms REAL,
    tokens_per_sec REAL NOT NULL DEFAULT 0,
    cost_usd REAL,
    is_local INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    cpu_peak REAL,
    ram_peak_mb REAL,
    gpu_peak REAL,
    rating INTEGER,
    judge_score REAL,
    judge_reason TEXT
);
CREATE INDEX IF NOT EXISTS results_run ON results(run_id);
CREATE INDEX IF NOT EXISTS runs_created ON runs(created_at);
CREATE TABLE IF NOT EXISTS prompts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    tags TEXT NOT NULL DEFAULT '',
    system TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(name, version)
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS conversations (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    provider TEXT,
    model TEXT,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    latency_ms REAL NOT NULL DEFAULT 0,
    error TEXT,
    note TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id);
";

const RESULT_COLS: &str = "id, slot, provider, model, output, error, prompt_tokens, completion_tokens, \
    tokens_estimated, latency_ms, ttft_ms, tokens_per_sec, cost_usd, is_local, note, cpu_peak, \
    ram_peak_mb, gpu_peak, rating, judge_score, judge_reason";

fn result_from_row(r: &Row) -> rusqlite::Result<ResultRecord> {
    Ok(ResultRecord {
        id: Some(r.get(0)?),
        stopped: false,
        slot: r.get::<_, i64>(1)? as usize,
        provider: r.get(2)?,
        model: r.get(3)?,
        output: r.get(4)?,
        error: r.get(5)?,
        prompt_tokens: r.get::<_, i64>(6)? as usize,
        completion_tokens: r.get::<_, i64>(7)? as usize,
        tokens_estimated: r.get(8)?,
        latency_ms: r.get(9)?,
        ttft_ms: r.get(10)?,
        tokens_per_sec: r.get(11)?,
        cost_usd: r.get(12)?,
        is_local: r.get(13)?,
        note: r.get(14)?,
        cpu_peak: r.get(15)?,
        ram_peak_mb: r.get(16)?,
        gpu_peak: r.get(17)?,
        rating: r.get(18)?,
        judge_score: r.get(19)?,
        judge_reason: r.get(20)?,
    })
}

fn prompt_from_row(r: &Row) -> rusqlite::Result<PromptRecord> {
    Ok(PromptRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        tags: r.get(3)?,
        system: r.get(4)?,
        body: r.get(5)?,
        created_at: r.get(6)?,
    })
}

type DbResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Db {
    pub fn open(path: &Path) -> DbResult<Self> {
        Self::init(Connection::open(path).map_err(err)?)
    }

    pub fn open_in_memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---- runs ----------------------------------------------------------

    /// Saves a run and its results; returns (run id, result ids in order).
    pub fn save_run(&self, run: &RunRecord, results: &[ResultRecord]) -> DbResult<(i64, Vec<i64>)> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(err)?;
        tx.execute(
            "INSERT INTO runs (created_at, kind, prompt, system, params, history) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                if run.created_at.is_empty() { now() } else { run.created_at.clone() },
                run.kind,
                run.prompt,
                run.system,
                serde_json::to_string(&run.params).map_err(err)?,
                serde_json::to_string(&run.history).map_err(err)?,
            ],
        )
        .map_err(err)?;
        let run_id = tx.last_insert_rowid();
        let mut ids = Vec::with_capacity(results.len());
        for r in results {
            tx.execute(
                "INSERT INTO results (run_id, slot, provider, model, output, error, prompt_tokens, \
                 completion_tokens, tokens_estimated, latency_ms, ttft_ms, tokens_per_sec, cost_usd, \
                 is_local, note, cpu_peak, ram_peak_mb, gpu_peak, rating, judge_score, judge_reason) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                params![
                    run_id,
                    r.slot as i64,
                    r.provider,
                    r.model,
                    r.output,
                    r.error,
                    r.prompt_tokens as i64,
                    r.completion_tokens as i64,
                    r.tokens_estimated,
                    r.latency_ms,
                    r.ttft_ms,
                    r.tokens_per_sec,
                    r.cost_usd,
                    r.is_local,
                    r.note,
                    r.cpu_peak,
                    r.ram_peak_mb,
                    r.gpu_peak,
                    r.rating,
                    r.judge_score,
                    r.judge_reason,
                ],
            )
            .map_err(err)?;
            ids.push(tx.last_insert_rowid());
        }
        tx.commit().map_err(err)?;
        Ok((run_id, ids))
    }

    pub fn load_run(&self, run_id: i64) -> DbResult<(RunRecord, Vec<ResultRecord>)> {
        let conn = self.conn();
        let run = conn
            .query_row(
                "SELECT id, created_at, kind, prompt, system, params, history FROM runs WHERE id = ?1",
                [run_id],
                |r| {
                    Ok(RunRecord {
                        id: Some(r.get(0)?),
                        created_at: r.get(1)?,
                        kind: r.get(2)?,
                        prompt: r.get(3)?,
                        system: r.get(4)?,
                        params: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                        history: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                    })
                },
            )
            .map_err(err)?;
        let mut stmt =
            conn.prepare(&format!("SELECT {RESULT_COLS} FROM results WHERE run_id = ?1 ORDER BY slot")).map_err(err)?;
        let results = stmt.query_map([run_id], result_from_row).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
        Ok((run, results))
    }

    pub fn history(&self, limit: usize) -> DbResult<Vec<HistoryRow>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.id, r.created_at, substr(r.prompt, 1, 120), r.kind, x.provider || '/' || x.model, \
                 x.cost_usd, x.latency_ms, x.prompt_tokens + x.completion_tokens, x.is_local, x.error IS NOT NULL \
                 FROM results x JOIN runs r ON r.id = x.run_id \
                 ORDER BY r.id DESC, x.slot LIMIT ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok(HistoryRow {
                    run_id: r.get(0)?,
                    created_at: r.get(1)?,
                    prompt: r.get(2)?,
                    kind: r.get(3)?,
                    model: r.get(4)?,
                    cost_usd: r.get(5)?,
                    latency_ms: r.get(6)?,
                    tokens: r.get::<_, i64>(7)? as usize,
                    is_local: r.get(8)?,
                    failed: r.get(9)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(rows)
    }

    pub fn set_rating(&self, result_id: i64, rating: Option<u8>) -> DbResult<()> {
        self.conn()
            .execute("UPDATE results SET rating = ?1 WHERE id = ?2", params![rating, result_id])
            .map(|_| ())
            .map_err(err)
    }

    pub fn set_judge(&self, result_id: i64, score: f32, reason: &str) -> DbResult<()> {
        self.conn()
            .execute(
                "UPDATE results SET judge_score = ?1, judge_reason = ?2 WHERE id = ?3",
                params![score, reason, result_id],
            )
            .map(|_| ())
            .map_err(err)
    }

    /// Total spend on runs created at or after `since` (`YYYY-MM-DD…`).
    pub fn spend_since(&self, since: &str) -> DbResult<f64> {
        self.conn()
            .query_row(
                "SELECT COALESCE(SUM(x.cost_usd), 0) FROM results x JOIN runs r ON r.id = x.run_id \
                 WHERE r.created_at >= ?1",
                [since],
                |r| r.get(0),
            )
            .map_err(err)
    }

    // ---- analytics ------------------------------------------------------

    /// Spend per day (`YYYY-MM-DD`) since a date, oldest first.
    pub fn spend_by_day(&self, since: &str) -> DbResult<Vec<(String, f64)>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT substr(r.created_at, 1, 10) AS day, COALESCE(SUM(x.cost_usd), 0) \
                 FROM results x JOIN runs r ON r.id = x.run_id WHERE r.created_at >= ?1 \
                 GROUP BY day ORDER BY day",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(rows)
    }

    /// (model, created_at, latency_ms) for successful results since a date.
    pub fn latency_points(&self, since: &str) -> DbResult<Vec<(String, String, f64)>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT x.provider || '/' || x.model, r.created_at, x.latency_ms \
                 FROM results x JOIN runs r ON r.id = x.run_id \
                 WHERE r.created_at >= ?1 AND x.error IS NULL AND r.kind != 'imported' ORDER BY r.created_at",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(rows)
    }

    pub fn model_stats(&self, since: &str) -> DbResult<Vec<ModelStats>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT x.run_id, x.provider || '/' || x.model, x.error IS NOT NULL, x.cost_usd, \
                 x.latency_ms, x.ttft_ms, x.tokens_per_sec, x.rating \
                 FROM results x JOIN runs r ON r.id = x.run_id WHERE r.created_at >= ?1",
            )
            .map_err(err)?;
        let rows: Vec<StatRow> = stmt
            .query_map([since], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(aggregate_stats(&rows))
    }

    pub fn export_csv(&self, path: &Path) -> DbResult<usize> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.created_at, r.kind, x.provider, x.model, x.prompt_tokens, x.completion_tokens, \
                 x.cost_usd, x.latency_ms, x.ttft_ms, x.tokens_per_sec, x.rating, x.judge_score, \
                 COALESCE(x.error, ''), r.prompt, x.output \
                 FROM results x JOIN runs r ON r.id = x.run_id ORDER BY r.id, x.slot",
            )
            .map_err(err)?;
        let mut w = csv::Writer::from_path(path).map_err(err)?;
        w.write_record([
            "created_at",
            "kind",
            "provider",
            "model",
            "prompt_tokens",
            "completion_tokens",
            "cost_usd",
            "latency_ms",
            "ttft_ms",
            "tokens_per_sec",
            "rating",
            "judge_score",
            "error",
            "prompt",
            "output",
        ])
        .map_err(err)?;
        let mut rows = stmt.query([]).map_err(err)?;
        let mut n = 0;
        while let Some(r) = rows.next().map_err(err)? {
            let cell = |i: usize| -> String {
                match r.get_ref(i) {
                    Ok(rusqlite::types::ValueRef::Null) | Err(_) => String::new(),
                    Ok(rusqlite::types::ValueRef::Integer(v)) => v.to_string(),
                    Ok(rusqlite::types::ValueRef::Real(v)) => v.to_string(),
                    Ok(rusqlite::types::ValueRef::Text(t)) => String::from_utf8_lossy(t).into_owned(),
                    Ok(rusqlite::types::ValueRef::Blob(_)) => String::new(),
                }
            };
            w.write_record((0..15).map(cell)).map_err(err)?;
            n += 1;
        }
        w.flush().map_err(err)?;
        Ok(n)
    }

    // ---- prompt library -------------------------------------------------

    /// Saves a new version of a named prompt; returns the version number.
    pub fn save_prompt(&self, name: &str, tags: &str, system: &str, body: &str) -> DbResult<i64> {
        let conn = self.conn();
        let version: i64 = conn
            .query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM prompts WHERE name = ?1", [name], |r| r.get(0))
            .map_err(err)?;
        conn.execute(
            "INSERT INTO prompts (name, version, tags, system, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name, version, tags, system, body, now()],
        )
        .map_err(err)?;
        Ok(version)
    }

    /// Latest version of every prompt, by name.
    pub fn list_prompts(&self) -> DbResult<Vec<PromptRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, version, tags, system, body, created_at FROM prompts p \
                 WHERE version = (SELECT MAX(version) FROM prompts WHERE name = p.name) ORDER BY name COLLATE NOCASE",
            )
            .map_err(err)?;
        let rows = stmt.query_map([], prompt_from_row).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
        Ok(rows)
    }

    pub fn prompt_versions(&self, name: &str) -> DbResult<Vec<PromptRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, version, tags, system, body, created_at FROM prompts WHERE name = ?1 ORDER BY version DESC",
            )
            .map_err(err)?;
        let rows = stmt.query_map([name], prompt_from_row).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
        Ok(rows)
    }

    pub fn delete_prompt(&self, name: &str) -> DbResult<()> {
        self.conn().execute("DELETE FROM prompts WHERE name = ?1", [name]).map(|_| ()).map_err(err)
    }

    // ---- chat -----------------------------------------------------------

    pub fn create_conversation(&self, title: &str, model: &ModelRef) -> DbResult<i64> {
        let conn = self.conn();
        let t = now();
        conn.execute(
            "INSERT INTO conversations (title, provider, model, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![title, model.provider, model.model, t],
        )
        .map_err(err)?;
        Ok(conn.last_insert_rowid())
    }

    /// Most recently active first.
    pub fn list_conversations(&self) -> DbResult<Vec<ConversationRow>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, title, provider, model, updated_at FROM conversations ORDER BY updated_at DESC, id DESC",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ConversationRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    model: ModelRef { provider: r.get(2)?, model: r.get(3)? },
                    updated_at: r.get(4)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(rows)
    }

    pub fn conversation_messages(&self, conversation_id: i64) -> DbResult<Vec<ChatMessage>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, role, content, provider, model, prompt_tokens, completion_tokens, cost_usd, \
                 latency_ms, error, note FROM messages WHERE conversation_id = ?1 ORDER BY id",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([conversation_id], |r| {
                let provider: Option<String> = r.get(3)?;
                let model: Option<String> = r.get(4)?;
                Ok(ChatMessage {
                    id: Some(r.get(0)?),
                    role: if r.get::<_, String>(1)? == "assistant" { Role::Assistant } else { Role::User },
                    content: r.get(2)?,
                    model: provider.zip(model).map(|(p, m)| ModelRef { provider: p, model: m }),
                    prompt_tokens: r.get::<_, i64>(5)? as usize,
                    completion_tokens: r.get::<_, i64>(6)? as usize,
                    cost_usd: r.get(7)?,
                    latency_ms: r.get(8)?,
                    error: r.get(9)?,
                    note: r.get(10)?,
                })
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(rows)
    }

    /// Appends a message and marks the conversation as active (and on this model).
    pub fn add_message(&self, conversation_id: i64, m: &ChatMessage) -> DbResult<i64> {
        let conn = self.conn();
        let role = if m.role == Role::Assistant { "assistant" } else { "user" };
        conn.execute(
            "INSERT INTO messages (conversation_id, role, content, provider, model, prompt_tokens, completion_tokens, \
             cost_usd, latency_ms, error, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                conversation_id,
                role,
                m.content,
                m.model.as_ref().map(|x| x.provider.clone()),
                m.model.as_ref().map(|x| x.model.clone()),
                m.prompt_tokens as i64,
                m.completion_tokens as i64,
                m.cost_usd,
                m.latency_ms,
                m.error,
                m.note,
                now(),
            ],
        )
        .map_err(err)?;
        let id = conn.last_insert_rowid();
        match &m.model {
            Some(model) => conn.execute(
                "UPDATE conversations SET updated_at = ?1, provider = ?2, model = ?3 WHERE id = ?4",
                params![now(), model.provider, model.model, conversation_id],
            ),
            None => {
                conn.execute("UPDATE conversations SET updated_at = ?1 WHERE id = ?2", params![now(), conversation_id])
            }
        }
        .map_err(err)?;
        Ok(id)
    }

    pub fn delete_message(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM messages WHERE id = ?1", [id]).map(|_| ()).map_err(err)
    }

    /// Replaces a message's text (used when the user edits a sent message).
    pub fn update_message(&self, id: i64, content: &str) -> DbResult<()> {
        self.conn()
            .execute("UPDATE messages SET content = ?1 WHERE id = ?2", params![content, id])
            .map(|_| ())
            .map_err(err)
    }

    /// Deletes every message after `id` in its conversation (an edit restarts from there).
    pub fn delete_messages_after(&self, conversation_id: i64, id: i64) -> DbResult<()> {
        self.conn()
            .execute("DELETE FROM messages WHERE conversation_id = ?1 AND id > ?2", params![conversation_id, id])
            .map(|_| ())
            .map_err(err)
    }

    pub fn rename_conversation(&self, id: i64, title: &str) -> DbResult<()> {
        self.conn()
            .execute("UPDATE conversations SET title = ?1 WHERE id = ?2", params![title, id])
            .map(|_| ())
            .map_err(err)
    }

    pub fn delete_conversation(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM conversations WHERE id = ?1", [id]).map(|_| ()).map_err(err)
    }

    // ---- settings -------------------------------------------------------

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let value: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        value.and_then(|v| serde_json::from_str(&v).ok())
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> DbResult<()> {
        let json = serde_json::to_string(value).map_err(err)?;
        self.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )
            .map(|_| ())
            .map_err(err)
    }

    // ---- migration ------------------------------------------------------

    /// Imports the pre-1.0 `audit_ledger.json` once, then renames it.
    pub fn import_legacy_ledger(&self, data_dir: &Path) -> DbResult<usize> {
        #[derive(Deserialize)]
        struct Legacy {
            timestamp: String,
            prompt_preview: String,
            model: String,
            prompt_tokens: usize,
            completion_tokens: usize,
            latency_ms: f64,
            cost_usd: f64,
            is_local: bool,
        }
        let path = data_dir.join("audit_ledger.json");
        if !path.exists() {
            return Ok(0);
        }
        let records: Vec<Legacy> =
            serde_json::from_str(&std::fs::read_to_string(&path).map_err(err)?).unwrap_or_default();
        for rec in records.iter().rev() {
            let created_at = chrono::DateTime::parse_from_rfc3339(&rec.timestamp)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|_| now());
            let run = RunRecord {
                created_at,
                kind: "imported".into(),
                prompt: rec.prompt_preview.clone(),
                ..Default::default()
            };
            let result = ResultRecord {
                provider: if rec.is_local { "ollama".into() } else { "openai".into() },
                model: rec.model.clone(),
                prompt_tokens: rec.prompt_tokens,
                completion_tokens: rec.completion_tokens,
                latency_ms: rec.latency_ms,
                cost_usd: Some(rec.cost_usd),
                is_local: rec.is_local,
                note: Some("Imported from the old ledger (output not stored)".into()),
                ..Default::default()
            };
            self.save_run(&run, &[result])?;
        }
        std::fs::rename(&path, data_dir.join("audit_ledger.imported.json")).map_err(err)?;
        Ok(records.len())
    }
}

type StatRow = (i64, String, bool, Option<f64>, f64, Option<f64>, f64, Option<u8>);

/// Stats plus the latency, first-token, speed and rating samples behind them.
type Acc = (ModelStats, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

fn aggregate_stats(rows: &[StatRow]) -> Vec<ModelStats> {
    let mut by_model: HashMap<String, Acc> = HashMap::new();
    for (_, model, failed, cost, latency, ttft, tps, rating) in rows {
        let (s, lat, tt, tp, rt) = by_model.entry(model.clone()).or_insert_with(|| {
            (ModelStats { model: model.clone(), ..Default::default() }, vec![], vec![], vec![], vec![])
        });
        s.runs += 1;
        s.total_cost += cost.unwrap_or(0.0);
        if *failed {
            s.errors += 1;
            continue;
        }
        lat.push(*latency);
        tp.push(*tps);
        if let Some(t) = ttft {
            tt.push(*t);
        }
        if let Some(r) = rating {
            rt.push(*r as f64);
        }
    }

    // Wins: in each run with 2+ rated results, a unique top rating wins.
    let mut runs: HashMap<i64, Vec<(&str, u8)>> = HashMap::new();
    for (run, model, _, _, _, _, _, rating) in rows {
        if let Some(r) = rating {
            runs.entry(*run).or_default().push((model, *r));
        }
    }
    let mut wins: HashMap<&str, (usize, usize)> = HashMap::new();
    for rated in runs.values().filter(|v| v.len() >= 2) {
        let top = rated.iter().map(|(_, r)| *r).max().unwrap_or(0);
        let unique = rated.iter().filter(|(_, r)| *r == top).count() == 1;
        for (model, r) in rated {
            let w = wins.entry(model).or_default();
            w.1 += 1;
            if unique && *r == top {
                w.0 += 1;
            }
        }
    }

    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let mut out: Vec<ModelStats> = by_model
        .into_values()
        .map(|(mut s, lat, tt, tp, rt)| {
            s.avg_latency_ms = mean(&lat).unwrap_or(0.0);
            s.avg_ttft_ms = mean(&tt);
            s.avg_tokens_per_sec = mean(&tp).unwrap_or(0.0);
            s.avg_rating = mean(&rt);
            s.rated = rt.len();
            (s.wins, s.contests) = wins.get(s.model.as_str()).copied().unwrap_or((0, 0));
            s
        })
        .collect();
    out.sort_by(|a, b| b.runs.cmp(&a.runs).then(a.model.cmp(&b.model)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(model: &str, cost: f64, rating: Option<u8>) -> ResultRecord {
        ResultRecord {
            provider: "openai".into(),
            model: model.into(),
            output: "out".into(),
            cost_usd: Some(cost),
            latency_ms: 100.0,
            rating,
            ..Default::default()
        }
    }

    #[test]
    fn run_roundtrip_and_stats() {
        let db = Db::open_in_memory().unwrap();
        let run = RunRecord { kind: "compare".into(), prompt: "hi".into(), ..Default::default() };
        let (run_id, ids) = db.save_run(&run, &[result("a", 0.5, Some(5)), result("b", 0.25, Some(3))]).unwrap();
        assert_eq!(ids.len(), 2);

        let (loaded, results) = db.load_run(run_id).unwrap();
        assert_eq!(loaded.prompt, "hi");
        assert_eq!(results[1].model, "b");

        db.set_rating(ids[1], Some(4)).unwrap();
        db.set_judge(ids[0], 8.5, "good").unwrap();
        let (_, results) = db.load_run(run_id).unwrap();
        assert_eq!(results[1].rating, Some(4));
        assert_eq!(results[0].judge_score, Some(8.5));

        assert!((db.spend_since("2000-01-01").unwrap() - 0.75).abs() < 1e-9);
        assert_eq!(db.history(10).unwrap().len(), 2);

        let stats = db.model_stats("2000-01-01").unwrap();
        let a = stats.iter().find(|s| s.model == "openai/a").unwrap();
        assert_eq!((a.wins, a.contests), (1, 1));
        assert_eq!(a.avg_rating, Some(5.0));
    }

    #[test]
    fn prompt_versions() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.save_prompt("p", "t", "", "v1").unwrap(), 1);
        assert_eq!(db.save_prompt("p", "t", "", "v2").unwrap(), 2);
        let latest = db.list_prompts().unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].body, "v2");
        assert_eq!(db.prompt_versions("p").unwrap().len(), 2);
        db.delete_prompt("p").unwrap();
        assert!(db.list_prompts().unwrap().is_empty());
    }

    #[test]
    fn chat_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let model = ModelRef::new("ollama", "llama3");
        let conv = db.create_conversation("Hello", &model).unwrap();
        db.add_message(conv, &ChatMessage { content: "hi".into(), ..Default::default() }).unwrap();
        let reply = ChatMessage {
            role: Role::Assistant,
            content: "yo".into(),
            model: Some(model.clone()),
            ..Default::default()
        };
        let reply_id = db.add_message(conv, &reply).unwrap();
        let msgs = db.conversation_messages(conv).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, Role::Assistant);
        assert_eq!(msgs[1].model.as_ref(), Some(&model));
        db.delete_message(reply_id).unwrap();
        assert_eq!(db.conversation_messages(conv).unwrap().len(), 1);

        // Editing a message drops everything after it.
        let first = db.conversation_messages(conv).unwrap()[0].id.unwrap();
        db.add_message(conv, &reply).unwrap();
        db.add_message(conv, &ChatMessage { content: "more".into(), ..Default::default() }).unwrap();
        db.update_message(first, "hi, edited").unwrap();
        db.delete_messages_after(conv, first).unwrap();
        let msgs = db.conversation_messages(conv).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi, edited");
        db.rename_conversation(conv, "Renamed").unwrap();
        assert_eq!(db.list_conversations().unwrap()[0].title, "Renamed");
        db.delete_conversation(conv).unwrap();
        assert!(db.list_conversations().unwrap().is_empty());
        assert!(db.conversation_messages(conv).unwrap().is_empty());
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("promptman-db-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn csv_export_escapes_awkward_text() {
        let db = Db::open_in_memory().unwrap();
        let tricky = "He said \"hi\", then\nleft — ✓ 日本";
        let run = RunRecord { kind: "compare".into(), prompt: tricky.into(), ..Default::default() };
        db.save_run(&run, &[ResultRecord { output: tricky.into(), ..result("m", 0.1, None) }]).unwrap();
        let dir = temp_dir("csv");
        let path = dir.join("out.csv");
        assert_eq!(db.export_csv(&path).unwrap(), 1);
        let mut reader = csv::Reader::from_path(&path).unwrap();
        let headers = reader.headers().unwrap().clone();
        let row = reader.records().next().unwrap().unwrap();
        let col = |name: &str| row.get(headers.iter().position(|h| h == name).unwrap()).unwrap().to_string();
        assert_eq!(col("prompt"), tricky);
        assert_eq!(col("output"), tricky);
        assert_eq!(col("model"), "m");
        assert_eq!(col("error"), "");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_ledger_is_imported_once() {
        let dir = temp_dir("ledger");
        let ledger = r#"[
          {"id":"a","timestamp":"2026-01-02T03:04:05Z","prompt_preview":"p1","provider":"OpenAI","model":"gpt-4o",
           "prompt_tokens":10,"completion_tokens":20,"latency_us":1,"latency_ms":450.0,"cost_usd":0.002,"cost_micro_usd":2000,"is_local":false},
          {"id":"b","timestamp":"bad date","prompt_preview":"p2","provider":"Ollama","model":"llama3",
           "prompt_tokens":5,"completion_tokens":5,"latency_us":1,"latency_ms":900.0,"cost_usd":0.0,"cost_micro_usd":0,"is_local":true}
        ]"#;
        std::fs::write(dir.join("audit_ledger.json"), ledger).unwrap();
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.import_legacy_ledger(&dir).unwrap(), 2);
        assert!(!dir.join("audit_ledger.json").exists());
        assert!(dir.join("audit_ledger.imported.json").exists());
        assert_eq!(db.import_legacy_ledger(&dir).unwrap(), 0, "second launch imports nothing");
        let history = db.history(10).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|h| h.kind == "imported"));
        assert!(history.iter().any(|h| h.model == "ollama/llama3" && h.is_local));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_legacy_ledger_does_not_panic() {
        let dir = temp_dir("badledger");
        std::fs::write(dir.join("audit_ledger.json"), "{ not json").unwrap();
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.import_legacy_ledger(&dir).unwrap(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn analytics_queries() {
        let db = Db::open_in_memory().unwrap();
        let at = |day: &str| RunRecord {
            kind: "compare".into(),
            created_at: format!("{day} 10:00:00"),
            ..Default::default()
        };
        db.save_run(&at("2026-03-01"), &[result("a", 1.0, Some(4)), result("b", 2.0, Some(2))]).unwrap();
        db.save_run(&at("2026-03-02"), &[result("a", 0.5, None)]).unwrap();
        let failed = ResultRecord { error: Some("boom".into()), ..result("b", 0.0, None) };
        db.save_run(&at("2026-03-02"), &[failed]).unwrap();
        let imported = RunRecord { kind: "imported".into(), ..at("2026-03-02") };
        db.save_run(&imported, &[result("c", 0.0, None)]).unwrap();

        let days = db.spend_by_day("2026-03-01").unwrap();
        assert_eq!(days, vec![("2026-03-01".to_string(), 3.0), ("2026-03-02".to_string(), 0.5)]);
        assert_eq!(db.spend_by_day("2026-03-02").unwrap().len(), 1);

        let points = db.latency_points("2026-01-01").unwrap();
        assert_eq!(points.len(), 3, "failed and imported answers are left out");

        let stats = db.model_stats("2026-01-01").unwrap();
        let b = stats.iter().find(|s| s.model == "openai/b").unwrap();
        assert_eq!((b.runs, b.errors), (2, 1));
        assert_eq!((b.wins, b.contests), (0, 1));
        let a = stats.iter().find(|s| s.model == "openai/a").unwrap();
        assert_eq!(a.rated, 1);
        assert_eq!(stats[0].model, "openai/a", "most used first");
    }

    #[test]
    fn history_is_newest_first_and_limited() {
        let db = Db::open_in_memory().unwrap();
        for i in 0..5 {
            let run = RunRecord { kind: "compare".into(), prompt: format!("p{i}"), ..Default::default() };
            db.save_run(&run, &[result("m", 0.0, None)]).unwrap();
        }
        let rows = db.history(3).unwrap();
        assert_eq!(rows.iter().map(|r| r.prompt.as_str()).collect::<Vec<_>>(), vec!["p4", "p3", "p2"]);
    }

    #[test]
    fn unknown_setting_types_read_as_none() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("x", &"text").unwrap();
        assert_eq!(db.get_setting::<f64>("x"), None);
        assert_eq!(db.get_setting::<String>("x").as_deref(), Some("text"));
    }

    #[test]
    fn settings_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get_setting::<f64>("cap"), None);
        db.set_setting("cap", &25.0).unwrap();
        db.set_setting("cap", &30.0).unwrap();
        assert_eq!(db.get_setting::<f64>("cap"), Some(30.0));
    }
}
