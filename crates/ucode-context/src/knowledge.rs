use std::path::Path;

use rusqlite::Connection;

use crate::embedder::Embedder;
use crate::error::ContextError;

pub struct KnowledgeBase {
    conn: Connection,
    embedder: Option<Box<dyn Embedder>>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeEntry {
    pub id: i64,
    pub source: String,
    pub content: String,
    pub metadata: Option<String>,
    /// ISO 8601 timestamp string from SQLite.
    pub created_at: String,
    pub score: f64,
}

impl KnowledgeBase {
    pub fn open(path: &Path, embedder: Option<Box<dyn Embedder>>) -> Result<Self, ContextError> {
        let conn = Connection::open(path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                source     TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts USING fts5(
                content,
                source,
                content=knowledge,
                content_rowid=id,
                tokenize='porter unicode61'
            );",
        )?;

        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS knowledge_ai AFTER INSERT ON knowledge BEGIN
                INSERT INTO knowledge_fts(rowid, content, source)
                    VALUES (new.id, new.content, new.source);
            END;
            CREATE TRIGGER IF NOT EXISTS knowledge_ad AFTER DELETE ON knowledge BEGIN
                INSERT INTO knowledge_fts(knowledge_fts, rowid, content, source)
                    VALUES ('delete', old.id, old.content, old.source);
            END;
            CREATE TRIGGER IF NOT EXISTS knowledge_au AFTER UPDATE ON knowledge BEGIN
                INSERT INTO knowledge_fts(knowledge_fts, rowid, content, source)
                    VALUES ('delete', old.id, old.content, old.source);
                INSERT INTO knowledge_fts(rowid, content, source)
                    VALUES (new.id, new.content, new.source);
            END;",
        )?;

        // TODO: sqlite-vec integration for vector search.
        // When sqlite-vec stabilises, load the extension here and create a
        // `knowledge_vec` virtual table keyed on `knowledge.id`.  The
        // `search_vector` stub below already returns an empty slice so the
        // public `search` API degrades gracefully until then.

        Ok(Self { conn, embedder })
    }

    pub fn store(
        &self,
        source: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64, ContextError> {
        self.conn.execute(
            "INSERT INTO knowledge (source, content, metadata) VALUES (?1, ?2, ?3)",
            rusqlite::params![source, content, metadata],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Full-text search using FTS5 BM25 ranking.
    ///
    /// FTS5's built-in `rank` column returns negative BM25 scores; more
    /// negative means more relevant.  We negate it so callers receive a
    /// positive score where higher is better.
    pub fn search_keyword(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, ContextError> {
        let mut stmt = self.conn.prepare(
            "SELECT k.id, k.source, k.content, k.metadata, k.created_at,
                    -rank AS score
             FROM knowledge_fts fts
             JOIN knowledge k ON k.id = fts.rowid
             WHERE knowledge_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let entries = stmt
            .query_map(rusqlite::params![query, limit as i64], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    content: row.get(2)?,
                    metadata: row.get(3)?,
                    created_at: row.get(4)?,
                    score: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Vector similarity search stub.
    ///
    /// Returns empty results until sqlite-vec integration is complete.
    /// When `embedder` is `None` this is a no-op regardless.
    pub fn search_vector(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, ContextError> {
        // TODO: generate embedding via self.embedder, query knowledge_vec table,
        // join with knowledge, return ranked results.
        Ok(Vec::new())
    }

    /// Search the knowledge base.
    ///
    /// Currently delegates to keyword search only.  When sqlite-vec
    /// integration is complete this will perform Reciprocal Rank Fusion (RRF)
    /// over both FTS5 and vector results.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeEntry>, ContextError> {
        self.search_keyword(query, limit)
    }

    /// Whether an embedder is configured (vector search capable).
    pub fn has_embedder(&self) -> bool {
        self.embedder.is_some()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn open_kb(dir: &TempDir) -> KnowledgeBase {
        KnowledgeBase::open(&dir.path().join("kb.db"), None).expect("open failed")
    }

    #[test]
    fn kb_open_creates_database() {
        let dir = TempDir::new().unwrap();
        let kb = KnowledgeBase::open(&dir.path().join("kb.db"), None);
        assert!(kb.is_ok());
    }

    #[test]
    fn kb_store_and_search_keyword() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        kb.store("test_source", "Rust ownership and borrowing rules", None)
            .unwrap();

        let results = kb.search_keyword("ownership", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "test_source");
        assert!(results[0].content.contains("ownership"));
    }

    #[test]
    fn kb_search_returns_ranked_results() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        // High relevance: mentions "memory" multiple times
        kb.store(
            "high",
            "memory management memory safety memory allocation in Rust",
            None,
        )
        .unwrap();
        // Low relevance: mentions "memory" once
        kb.store("low", "Rust has memory safety guarantees", None)
            .unwrap();

        let results = kb.search_keyword("memory", 10).unwrap();
        assert_eq!(results.len(), 2);
        // Higher score (more relevant) should come first
        assert!(results[0].score >= results[1].score);
        assert_eq!(results[0].source, "high");
    }

    #[test]
    fn kb_search_no_results() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        kb.store("src", "Rust ownership and borrowing", None)
            .unwrap();

        let results = kb.search_keyword("python", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn kb_store_with_metadata() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        kb.store(
            "docs",
            "async await in Rust with tokio",
            Some(r#"{"tag":"async"}"#),
        )
        .unwrap();

        let results = kb.search_keyword("async", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.as_deref(), Some(r#"{"tag":"async"}"#));
    }

    #[test]
    fn kb_porter_stemming() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        // Store content with base form "run"
        kb.store("src", "The program will run the tests", None)
            .unwrap();

        // Search with inflected form -- Porter stemmer should match
        let results = kb.search_keyword("running", 10).unwrap();
        assert_eq!(
            results.len(),
            1,
            "Porter stemmer should match 'running' against 'run'"
        );
    }
}
