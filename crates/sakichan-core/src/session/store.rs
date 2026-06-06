use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::error::{Result, SakichanError};
use crate::models::Message;

/// A session record.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub model_id: String,
    pub system_prompt: Option<String>,
    pub metadata: Option<String>,
    /// Current active depth in the branch tree.
    pub active_depth: i64,
    /// Current active order at the active depth.
    pub active_order: i64,
}

/// SQLite-backed session storage.
#[derive(Clone)]
pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
}

impl SessionStore {
    /// Open or create the session database at the given path.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SakichanError::Other(format!("Failed to create db directory: {e}")))?;
        }

        let conn = Connection::open(db_path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.initialize_tables()?;
        Ok(store)
    }

    /// Initialize database tables and run migrations.
    fn initialize_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Phase 1: create tables (if not exist)
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                model_id TEXT NOT NULL,
                system_prompt TEXT,
                metadata TEXT
            );

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                reasoning_content TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                edited_at INTEGER,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session_time
                ON messages(session_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_sessions_updated
                ON sessions(updated_at);
            ",
        )?;

        // Phase 2: add new columns (safe for old databases)
        Self::migrate_add_column_inner(&conn, "sessions", "active_depth", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column_inner(&conn, "sessions", "active_order", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column_inner(&conn, "messages", "depth", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column_inner(&conn, "messages", "message_order", "INTEGER NOT NULL DEFAULT 0")?;

        // Phase 3: new indexes
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_session_depth
                ON messages(session_id, depth, message_order);"
        )?;

        Ok(())
    }

    /// Check if a column exists in a table.
    fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(1)?;
            Ok(name)
        })?;
        for row in rows {
            if row? == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Add a column to a table if it doesn't already exist (uses existing connection).
    fn migrate_add_column_inner(conn: &Connection, table: &str, column: &str, col_def: &str) -> Result<()> {
        if !Self::column_exists(conn, table, column)? {
            let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_def);
            conn.execute(&sql, [])?;
        }
        Ok(())
    }

    /// Create a new session.
    pub fn create_session(
        &self,
        model_id: &str,
        system_prompt: Option<&str>,
    ) -> Result<Session> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, model_id, system_prompt, active_depth, active_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, now, now, model_id, system_prompt, 0i64, 0i64],
        )?;
        Ok(Session {
            id,
            title: None,
            created_at: now,
            updated_at: now,
            model_id: model_id.to_string(),
            system_prompt: system_prompt.map(|s| s.to_string()),
            metadata: None,
            active_depth: 0,
            active_order: 0,
        })
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, model_id, system_prompt, metadata, active_depth, active_order FROM sessions WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                model_id: row.get(4)?,
                system_prompt: row.get(5)?,
                metadata: row.get(6)?,
                active_depth: row.get(7)?,
                active_order: row.get(8)?,
            })),
            None => Ok(None),
        }
    }

    /// List all sessions, ordered by updated_at descending.
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, model_id, system_prompt, metadata, active_depth, active_order FROM sessions ORDER BY updated_at DESC LIMIT 100",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    model_id: row.get(4)?,
                    system_prompt: row.get(5)?,
                    metadata: row.get(6)?,
                    active_depth: row.get(7)?,
                    active_order: row.get(8)?,
                })
            })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// Update session title.
    pub fn update_title(&self, id: &str, title: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )?;
        Ok(())
    }

    /// Delete a session and its messages.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Add a message to a session with depth/order for branching support.
    pub fn add_message(&self, session_id: &str, msg: &Message, depth: i64, order: i64) -> Result<i64> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();

        // Update session's updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        let tool_calls_json = msg
            .tool_calls
            .as_ref()
            .map(|tc| serde_json::to_string(tc).unwrap_or_default());

        conn.execute(
            "INSERT INTO messages (session_id, role, content, reasoning_content, tool_calls, tool_call_id, created_at, depth, message_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                msg.role,
                msg.content,
                msg.reasoning_content,
                tool_calls_json,
                msg.tool_call_id,
                now,
                depth,
                order,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update a message's content (for /edit feature).
    pub fn update_message_content(&self, message_id: i64, new_content: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET content = ?1, edited_at = ?2 WHERE id = ?3",
            params![new_content, now, message_id],
        )?;
        Ok(())
    }

    /// Delete the last assistant message from a session.
    pub fn delete_last_assistant(&self, session_id: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        // Find the last assistant message ID
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM messages WHERE session_id = ?1 AND role = 'assistant' ORDER BY created_at DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .ok();

        if let Some(msg_id) = id {
            conn.execute("DELETE FROM messages WHERE id = ?1", params![msg_id])?;
            Ok(Some(msg_id))
        } else {
            Ok(None)
        }
    }

    /// Get the last user message from a session.
    pub fn get_last_user_message(&self, session_id: &str) -> Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, content FROM messages WHERE session_id = ?1 AND role = 'user' ORDER BY created_at DESC LIMIT 1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get messages for a session, ordered by creation time.
    pub fn get_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content, reasoning_content, tool_calls, tool_call_id, depth, message_order
             FROM messages WHERE session_id = ?1 ORDER BY depth ASC, message_order ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let tool_calls_str: Option<String> = row.get(3)?;
            let tool_calls = tool_calls_str
                .and_then(|s| serde_json::from_str::<Vec<crate::models::ToolCall>>(&s).ok());

            Ok(Message {
                role: row.get(0)?,
                content: row.get(1)?,
                reasoning_content: row.get(2)?,
                tool_calls,
                tool_call_id: row.get(4)?,
                name: None,
                images: Vec::new(),
            })
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    /// Get the message count for a session.
    pub fn message_count(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Set the active branch position for a session.
    pub fn set_active_branch(&self, session_id: &str, depth: i64, order: i64) -> Result<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET active_depth = ?1, active_order = ?2, updated_at = ?3 WHERE id = ?4",
            params![depth, order, now, session_id],
        )?;
        Ok(())
    }

    /// Get the next available order at a given depth for a session.
    pub fn get_next_order(&self, session_id: &str, depth: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let max_order: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(MAX(message_order), 0) FROM messages WHERE session_id = ?1 AND depth = ?2",
                params![session_id, depth],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(max_order.unwrap_or(0) + 1)
    }

    /// Get the maximum depth for a session.
    pub fn get_max_depth(&self, session_id: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let max_depth: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(MAX(depth), 0) FROM messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(max_depth.unwrap_or(0))
    }

    /// Get messages filtered to a specific branch chain.
    /// Walk from depth 0 upward, using the active order at each depth.
    pub fn get_branch_messages(&self, session_id: &str, active_depth: i64, active_order: i64) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        // Collect all (depth, order) pairs along the active path
        // Active path: for each depth d from 0 to active_depth, use the message
        // whose (depth, order) matches the session's cursor at that depth.
        // We simply load all messages and filter by depth <= active_depth,
        // then keep only the messages whose order matches the active path.
        let mut stmt = conn.prepare(
            "SELECT role, content, reasoning_content, tool_calls, tool_call_id, depth, message_order
             FROM messages WHERE session_id = ?1 AND depth <= ?2
             ORDER BY depth ASC, message_order ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id, active_depth], |row| {
            let tool_calls_str: Option<String> = row.get(3)?;
            let tool_calls = tool_calls_str
                .and_then(|s| serde_json::from_str::<Vec<crate::models::ToolCall>>(&s).ok());
            Ok((
                Message {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    reasoning_content: row.get(2)?,
                    tool_calls,
                    tool_call_id: row.get(4)?,
                    name: None,
                    images: Vec::new(),
                },
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        // Build a path of (depth -> active_order_at_this_depth)
        // From depth 0 to active_depth, we trace through:
        //   depth 0: order 0 (root)
        //   For each subsequent depth, find the order that continues from
        //   the previous depth's active path.
        // Simple approach: for each depth d, find the max order <= active_order's order
        // But actually the simplest filter is: include all messages from depth 0 to active_depth,
        // and at depth == active_depth, only include messages with order == active_order.
        let mut filtered = Vec::new();
        for row in rows {
            let (msg, d, o) = row?;
            if d < active_depth {
                // Include all messages at shallower depths
                filtered.push(msg);
            } else if d == active_depth && o == active_order {
                // Only include the active order at the current depth
                filtered.push(msg);
            }
        }
        Ok(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_store() -> SessionStore {
        let path = PathBuf::from(":memory:");
        SessionStore::open(&path).unwrap()
    }

    #[test]
    fn test_create_and_get_session() {
        let store = test_store();
        let session = store.create_session("test-model", None).unwrap();
        let got = store.get_session(&session.id).unwrap().unwrap();
        assert_eq!(got.id, session.id);
        assert_eq!(got.model_id, "test-model");
    }

    #[test]
    fn test_add_and_get_messages() {
        let store = test_store();
        let session = store.create_session("test-model", None).unwrap();
        store
            .add_message(&session.id, &Message::user("hello"), 0, 0)
            .unwrap();
        store
            .add_message(&session.id, &Message::assistant("world"), 0, 0)
            .unwrap();
        let msgs = store.get_messages(&session.id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn test_delete_last_assistant() {
        let store = test_store();
        let session = store.create_session("test-model", None).unwrap();
        store
            .add_message(&session.id, &Message::user("hi"), 0, 0)
            .unwrap();
        store
            .add_message(&session.id, &Message::assistant("hey"), 0, 0)
            .unwrap();
        let deleted = store.delete_last_assistant(&session.id).unwrap();
        assert!(deleted.is_some());
        let msgs = store.get_messages(&session.id).unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_branch_messages() {
        let store = test_store();
        let session = store.create_session("test-model", None).unwrap();

        // depth 0: root conversation
        store.add_message(&session.id, &Message::user("hello"), 0, 0).unwrap();
        store.add_message(&session.id, &Message::assistant("hi"), 0, 0).unwrap();

        // depth 1, order 1: normal continuation
        store.add_message(&session.id, &Message::user("how are you?"), 1, 1).unwrap();
        store.add_message(&session.id, &Message::assistant("fine"), 1, 1).unwrap();

        // depth 1, order 2: branch
        store.add_message(&session.id, &Message::user("what's up?"), 1, 2).unwrap();
        store.add_message(&session.id, &Message::assistant("nothing"), 1, 2).unwrap();

        // Active branch at depth 1, order 1 should only include 4 messages
        let branch1 = store.get_branch_messages(&session.id, 1, 1).unwrap();
        assert_eq!(branch1.len(), 4);

        // Active branch at depth 1, order 2 should also include 4 messages
        let branch2 = store.get_branch_messages(&session.id, 1, 2).unwrap();
        assert_eq!(branch2.len(), 4);
    }
}