-- Sakichan 数据库初始化脚本
-- 执行: sqlite3 ~/.local/share/sakichan/sakichan.db < scripts/setup_db.sql

PRAGMA foreign_keys = ON;

-- 会话表
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    title TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    model_id TEXT NOT NULL,
    system_prompt TEXT,
    metadata TEXT -- JSON string
);

-- 消息表
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    reasoning_content TEXT,
    tool_calls TEXT, -- JSON string
    tool_call_id TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    edited_at INTEGER,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- 长期记忆表
CREATE TABLE IF NOT EXISTS long_term_memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    memory_type TEXT NOT NULL DEFAULT 'custom',
    tags TEXT, -- JSON array
    embedding BLOB,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    accessed_at INTEGER,
    source_session_id TEXT,
    FOREIGN KEY (source_session_id) REFERENCES sessions(id) ON DELETE SET NULL
);

-- 索引优化
CREATE INDEX IF NOT EXISTS idx_messages_session_time ON messages(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_memories_content ON long_term_memories(content);
CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at);
