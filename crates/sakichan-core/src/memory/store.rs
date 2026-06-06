use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::error::{Result, SakichanError};

/// A long-term memory record.
#[derive(Clone, Debug)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub memory_type: String,
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub accessed_at: Option<i64>,
    pub source_session_id: Option<String>,
}

/// SQLite-backed long-term memory store.
#[derive(Clone)]
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryStore {
    /// Open or create the memory database (uses the same SQLite db as sessions).
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

    /// Share an existing connection.
    pub fn from_connection(existing: Arc<Mutex<Connection>>) -> Self {
        Self { conn: existing }
    }

    fn initialize_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS long_term_memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL DEFAULT 'custom',
                tags TEXT,
                embedding BLOB,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                accessed_at INTEGER,
                source_session_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_memories_content
                ON long_term_memories(content);

            CREATE INDEX IF NOT EXISTS idx_memories_type
                ON long_term_memories(memory_type);

            CREATE INDEX IF NOT EXISTS idx_memories_accessed
                ON long_term_memories(accessed_at);
            ",
        )?;
        Ok(())
    }

    /// Add a new memory.
    pub fn add(
        &self,
        content: &str,
        memory_type: &str,
        tags: Option<&[String]>,
        source_session_id: Option<&str>,
    ) -> Result<i64> {
        let now = Utc::now().timestamp();
        let tags_json = tags.map(|t| serde_json::to_string(t).unwrap_or_default());
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO long_term_memories (content, memory_type, tags, created_at, source_session_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![content, memory_type, tags_json, now, source_session_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Delete a memory by ID.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM long_term_memories WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Get a memory by ID.
    pub fn get(&self, id: i64) -> Result<Option<Memory>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content, memory_type, tags, created_at, accessed_at, source_session_id
             FROM long_term_memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            let tags_str: Option<String> = row.get(3)?;
            let tags = tags_str.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
            Ok(Memory {
                id: row.get(0)?,
                content: row.get(1)?,
                memory_type: row.get(2)?,
                tags,
                created_at: row.get(4)?,
                accessed_at: row.get(5)?,
                source_session_id: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(Ok(mem)) => Ok(Some(mem)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// List all memories.
    pub fn list(&self, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content, memory_type, tags, created_at, accessed_at, source_session_id
             FROM long_term_memories ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let tags_str: Option<String> = row.get(3)?;
            let tags = tags_str.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
            Ok(Memory {
                id: row.get(0)?,
                content: row.get(1)?,
                memory_type: row.get(2)?,
                tags,
                created_at: row.get(4)?,
                accessed_at: row.get(5)?,
                source_session_id: row.get(6)?,
            })
        })?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row?);
        }
        Ok(memories)
    }

    /// Search memories by keyword (LIKE-based).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = conn.prepare(
            "SELECT id, content, memory_type, tags, created_at, accessed_at, source_session_id
             FROM long_term_memories
             WHERE content LIKE ?1 ESCAPE '\\'
             ORDER BY CASE WHEN accessed_at IS NOT NULL THEN 0 ELSE 1 END, accessed_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let tags_str: Option<String> = row.get(3)?;
            let tags = tags_str.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
            Ok(Memory {
                id: row.get(0)?,
                content: row.get(1)?,
                memory_type: row.get(2)?,
                tags,
                created_at: row.get(4)?,
                accessed_at: row.get(5)?,
                source_session_id: row.get(6)?,
            })
        })?;
        let mut memories = Vec::new();
        for row in rows {
            let mem = row?;
            // Update accessed_at
            conn.execute(
                "UPDATE long_term_memories SET accessed_at = ?1 WHERE id = ?2",
                params![now, mem.id],
            )?;
            memories.push(mem);
        }
        Ok(memories)
    }

    /// Count total memories.
    pub fn count(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM long_term_memories",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_store() -> MemoryStore {
        MemoryStore::open(&PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_add_and_list() {
        let store = test_store();
        store.add("我的名字是张三", "user_fact", None, None).unwrap();
        store.add("我喜欢吃苹果", "custom", None, None).unwrap();
        let list = store.list(10).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_search() {
        let store = test_store();
        store.add("我的名字是张三", "user_fact", None, None).unwrap();
        store.add("我喜欢吃苹果", "custom", None, None).unwrap();
        let results = store.search("张三", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "我的名字是张三");
    }

    #[test]
    fn test_delete() {
        let store = test_store();
        let id = store.add("test", "custom", None, None).unwrap();
        assert!(store.delete(id).unwrap());
        assert!(store.get(id).unwrap().is_none());
    }
}