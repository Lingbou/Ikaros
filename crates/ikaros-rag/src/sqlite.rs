// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    HashEmbeddingProvider,
    embedding::EmbeddingProvider,
    files::{canonical_or_self, chunk_text, collect_files, path_to_string, system_time_to_rfc3339},
    jsonl::search_chunks,
    types::{
        IngestOptions, IngestReport, IngestSourceFile, RagChunk, RagHit, RagIndexedFile, RagQuery,
        RagStore,
    },
};
use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use rusqlite::{Connection, params};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SqliteRagIndex {
    path: PathBuf,
}

impl SqliteRagIndex {
    pub fn new(rag_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: rag_dir.into().join("index.sqlite"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn =
            Connection::open(&self.path).map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| sqlite_error(&self.path, source))?;
        self.ensure_schema(&conn)?;
        Ok(conn)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS rag_documents (
                id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                canonical_path TEXT NOT NULL,
                scope TEXT NOT NULL,
                indexed_at TEXT NOT NULL,
                modified_at TEXT
            );
            CREATE INDEX IF NOT EXISTS rag_documents_scope_idx ON rag_documents(scope);
            CREATE UNIQUE INDEX IF NOT EXISTS rag_documents_canonical_scope_idx ON rag_documents(canonical_path, scope);

            CREATE TABLE IF NOT EXISTS rag_chunks (
                id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL,
                scope TEXT NOT NULL,
                source_path TEXT NOT NULL,
                canonical_path TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding_provider TEXT,
                embedding_json TEXT,
                indexed_at TEXT NOT NULL,
                modified_at TEXT,
                FOREIGN KEY(document_id) REFERENCES rag_documents(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS rag_chunks_scope_idx ON rag_chunks(scope);
            CREATE INDEX IF NOT EXISTS rag_chunks_canonical_idx ON rag_chunks(canonical_path);
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        require_sqlite_columns(
            conn,
            &self.path,
            "rag_chunks",
            &[
                "id",
                "document_id",
                "scope",
                "source_path",
                "canonical_path",
                "line_start",
                "line_end",
                "content",
                "embedding_provider",
                "embedding_json",
                "indexed_at",
                "modified_at",
            ],
        )
    }

    pub(crate) fn read_all(&self) -> Result<Vec<RagChunk>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, document_id, scope, source_path, line_start, line_end, content, indexed_at, modified_at, embedding_provider, embedding_json FROM rag_chunks",
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                let embedding_json = row.get::<_, Option<String>>(10)?;
                Ok(RagChunk {
                    id: row.get(0)?,
                    document_id: row.get(1)?,
                    scope: row.get(2)?,
                    source_path: PathBuf::from(row.get::<_, String>(3)?),
                    line_start: row.get::<_, i64>(4)? as usize,
                    line_end: row.get::<_, i64>(5)? as usize,
                    content: row.get(6)?,
                    indexed_at: row.get(7)?,
                    modified_at: row.get(8)?,
                    embedding_provider: row.get(9)?,
                    embedding: embedding_json
                        .as_deref()
                        .and_then(|value| serde_json::from_str(value).ok()),
                })
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut chunks = Vec::new();
        for row in rows {
            chunks.push(row.map_err(|source| sqlite_error(&self.path, source))?);
        }
        Ok(chunks)
    }

    pub fn ingest_path_with_embedding(
        &self,
        path: &Path,
        options: IngestOptions,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<IngestReport> {
        let mut files = Vec::new();
        collect_files(path, &mut files)?;
        let mut sources = Vec::new();
        for file in files {
            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(_) => continue,
            };
            let metadata = fs::metadata(&file).map_err(|source| IkarosError::io(&file, source))?;
            sources.push(IngestSourceFile {
                source_path: file,
                content: text,
                modified_at: metadata.modified().ok().and_then(system_time_to_rfc3339),
            });
        }
        self.ingest_sources_with_embedding(sources, options, embedding_provider)
    }

    pub fn ingest_sources_with_embedding(
        &self,
        sources: Vec<IngestSourceFile>,
        options: IngestOptions,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<IngestReport> {
        let conn = self.open()?;
        let canonical_targets = sources
            .iter()
            .map(|source| path_to_string(&canonical_or_self(&source.source_path)))
            .collect::<BTreeSet<_>>();
        for target in &canonical_targets {
            conn.execute(
                "DELETE FROM rag_chunks WHERE canonical_path = ?1 AND scope = ?2",
                params![target, &options.scope],
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
            conn.execute(
                "DELETE FROM rag_documents WHERE canonical_path = ?1 AND scope = ?2",
                params![target, &options.scope],
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        }

        let mut indexed_files = 0;
        let mut chunks_indexed = 0;
        let files_seen = sources.len();
        for source in sources {
            let document_id = Uuid::new_v4().to_string();
            let indexed_at = now_rfc3339()?;
            let canonical_path = path_to_string(&canonical_or_self(&source.source_path));
            let source_path = path_to_string(&source.source_path);
            conn.execute(
                "INSERT INTO rag_documents (id, source_path, canonical_path, scope, indexed_at, modified_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    &document_id,
                    &source_path,
                    &canonical_path,
                    &options.scope,
                    &indexed_at,
                    &source.modified_at,
                ],
            )
            .map_err(|source| sqlite_error(&self.path, source))?;

            let mut file_chunk_count = 0;
            for (line_start, line_end, content) in
                chunk_text(&source.content, options.max_chunk_lines)
            {
                let chunk_id = Uuid::new_v4().to_string();
                let content = redact_secrets(&content);
                let embedding = serde_json::to_string(&embedding_provider.embed(&content)?)?;
                let provider = embedding_provider.name();
                conn.execute(
                    "INSERT INTO rag_chunks (id, document_id, scope, source_path, canonical_path, line_start, line_end, content, embedding_provider, embedding_json, indexed_at, modified_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        &chunk_id,
                        &document_id,
                        &options.scope,
                        &source_path,
                        &canonical_path,
                        line_start as i64,
                        line_end as i64,
                        &content,
                        provider,
                        &embedding,
                        &indexed_at,
                        &source.modified_at,
                    ],
                )
                .map_err(|source| sqlite_error(&self.path, source))?;
                file_chunk_count += 1;
            }
            if file_chunk_count > 0 {
                indexed_files += 1;
                chunks_indexed += file_chunk_count;
            }
        }

        Ok(IngestReport {
            files_seen,
            files_indexed: indexed_files,
            chunks_indexed,
        })
    }

    pub fn search_with_embedding(
        &self,
        query: RagQuery,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<Vec<RagHit>> {
        search_chunks(self.read_all()?, query, embedding_provider)
    }
}

impl RagStore for SqliteRagIndex {
    fn ingest_path(&self, path: &Path, options: IngestOptions) -> Result<IngestReport> {
        self.ingest_path_with_embedding(path, options, &HashEmbeddingProvider)
    }

    fn search(&self, query: RagQuery) -> Result<Vec<RagHit>> {
        self.search_with_embedding(query, &HashEmbeddingProvider)
    }

    fn delete_scope(&self, scope: &str) -> Result<usize> {
        let conn = self.open()?;
        let deleted = conn
            .execute("DELETE FROM rag_chunks WHERE scope = ?1", params![scope])
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.execute("DELETE FROM rag_documents WHERE scope = ?1", params![scope])
            .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted)
    }

    fn delete_path(&self, path: &Path) -> Result<usize> {
        let conn = self.open()?;
        let target = path_to_string(&canonical_or_self(path));
        let deleted = conn
            .execute(
                "DELETE FROM rag_chunks WHERE canonical_path = ?1",
                params![&target],
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.execute(
            "DELETE FROM rag_documents WHERE canonical_path = ?1",
            params![&target],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted)
    }

    fn indexed_files(&self) -> Result<Vec<RagIndexedFile>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT source_path, modified_at FROM rag_documents")
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RagIndexedFile {
                    source_path: PathBuf::from(row.get::<_, String>(0)?),
                    modified_at: row.get::<_, Option<String>>(1)?,
                })
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut files = Vec::new();
        for row in rows {
            files.push(row.map_err(|source| sqlite_error(&self.path, source))?);
        }
        files.sort_by(|left, right| {
            left.source_path
                .cmp(&right.source_path)
                .then_with(|| left.modified_at.cmp(&right.modified_at))
        });
        files.dedup();
        Ok(files)
    }

    fn stale_files(&self) -> Result<Vec<PathBuf>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT source_path, modified_at FROM rag_documents")
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut stale = Vec::new();
        for row in rows {
            let (path, indexed_modified) =
                row.map_err(|source| sqlite_error(&self.path, source))?;
            let current = fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_to_rfc3339);
            if current != indexed_modified {
                stale.push(path);
            }
        }
        stale.sort();
        stale.dedup();
        Ok(stale)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn backend_name(&self) -> &'static str {
        "sqlite"
    }
}

fn sqlite_error(path: &Path, source: rusqlite::Error) -> IkarosError {
    IkarosError::Message(format!("sqlite error at {}: {source}", path.display()))
}

fn require_sqlite_columns(
    conn: &Connection,
    path: &Path,
    table: &str,
    required: &[&str],
) -> Result<()> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| sqlite_error(path, source))?;
    let mut existing = BTreeSet::new();
    for row in rows {
        existing.insert(row.map_err(|source| sqlite_error(path, source))?);
    }
    for column in required {
        if !existing.contains(*column) {
            return Err(IkarosError::Message(format!(
                "RAG SQLite index at {} is missing required column {table}.{column}; delete the index and run rag reindex",
                path.display()
            )));
        }
    }
    Ok(())
}
