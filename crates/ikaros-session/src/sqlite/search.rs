// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn search_entries(
    conn: &Connection,
    path: &Path,
    query: &SessionSearchQuery,
) -> Result<Vec<SessionSearchHit>> {
    let query_text = query.query.trim();
    if query_text.is_empty() || query.limit == 0 {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let sanitized_query = sanitize_session_search_query(query_text);
    if let Some(fts_query) = sanitized_query.fts_query.as_deref() {
        collect_index_hits(
            conn,
            path,
            &mut hits,
            &mut seen,
            ("session_entries_fts", SessionSearchIndex::Fts),
            fts_query,
            query,
        )?;
        collect_index_hits(
            conn,
            path,
            &mut hits,
            &mut seen,
            ("session_entries_trigram", SessionSearchIndex::Trigram),
            fts_query,
            query,
        )?;
    }
    collect_substring_hits(
        conn,
        path,
        &mut hits,
        &mut seen,
        query,
        &sanitized_query.substring_query,
    )?;

    hits.sort_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.entry.at.cmp(&right.entry.at))
            .then_with(|| left.entry.entry_id.cmp(&right.entry.entry_id))
    });
    hits.truncate(query.limit);
    Ok(hits)
}

pub(super) fn collect_index_hits(
    conn: &Connection,
    path: &Path,
    hits: &mut Vec<SessionSearchHit>,
    seen: &mut HashSet<String>,
    index_spec: (&str, SessionSearchIndex),
    fts_query: &str,
    query: &SessionSearchQuery,
) -> Result<()> {
    let (table, index) = index_spec;
    let sql = format!(
        r#"
        SELECT e.id, e.session_id, e.parent_entry_id, e.turn_id, e.at, e.kind, e.visible_text,
               e.payload_json, bm25({table}) AS score
        FROM {table}
        JOIN session_entries e ON e.id = {table}.entry_id
        WHERE {table} MATCH ?1
          AND (?2 IS NULL OR e.session_id = ?2)
        ORDER BY score ASC, e.at ASC
        LIMIT ?3
        "#
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(()),
    };
    let rows = match stmt.query_map(
        params![
            fts_query,
            query.session_id.as_ref().map(SessionId::as_str),
            query.limit as i64,
        ],
        |row| {
            Ok((
                (
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ),
                row.get::<_, f64>(8)?,
            ))
        },
    ) {
        Ok(rows) => rows,
        Err(_) => return Ok(()),
    };
    for row in rows {
        let (entry_row, score) = row.map_err(|source| sqlite_error(path, source))?;
        let entry = session_entry_from_parts(entry_row)?;
        if seen.insert(entry.entry_id.as_str().to_owned()) {
            hits.push(SessionSearchHit {
                snippet: entry_snippet(entry.visible_text.as_deref(), &query.query),
                entry,
                score,
                index,
            });
        }
    }
    Ok(())
}

pub(super) fn collect_substring_hits(
    conn: &Connection,
    path: &Path,
    hits: &mut Vec<SessionSearchHit>,
    seen: &mut HashSet<String>,
    query: &SessionSearchQuery,
    substring_query: &str,
) -> Result<()> {
    if substring_query.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
            FROM session_entries
            WHERE visible_text IS NOT NULL
              AND instr(visible_text, ?1) > 0
              AND (?2 IS NULL OR session_id = ?2)
            ORDER BY at ASC, rowid ASC
            LIMIT ?3
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                substring_query,
                query.session_id.as_ref().map(SessionId::as_str),
                query.limit as i64,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    for row in rows {
        let entry = session_entry_from_parts(row.map_err(|source| sqlite_error(path, source))?)?;
        if seen.insert(entry.entry_id.as_str().to_owned()) {
            hits.push(SessionSearchHit {
                snippet: entry_snippet(entry.visible_text.as_deref(), &query.query),
                score: 10_000.0 + hits.len() as f64,
                entry,
                index: SessionSearchIndex::Substring,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SanitizedSessionSearchQuery {
    fts_query: Option<String>,
    substring_query: String,
}

pub(super) fn sanitize_session_search_query(query: &str) -> SanitizedSessionSearchQuery {
    let substring_query = query
        .trim()
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let terms = substring_query
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .take(32)
        .map(quoted_fts_term)
        .collect::<Vec<_>>();
    SanitizedSessionSearchQuery {
        fts_query: if terms.is_empty() {
            None
        } else {
            Some(terms.join(" "))
        },
        substring_query,
    }
}

pub(super) fn quoted_fts_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

pub(super) fn entry_snippet(visible_text: Option<&str>, query: &str) -> String {
    let Some(text) = visible_text else {
        return String::new();
    };
    let query = query.trim();
    if query.is_empty() {
        return text.chars().take(160).collect();
    }
    let start_byte = text.find(query).unwrap_or(0);
    let prefix_start = text[..start_byte]
        .char_indices()
        .rev()
        .nth(40)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let end_byte = text[start_byte..]
        .char_indices()
        .nth(query.chars().count().saturating_add(80))
        .map(|(idx, _)| start_byte + idx)
        .unwrap_or(text.len());
    let mut snippet = String::new();
    if prefix_start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&text[prefix_start..end_byte]);
    if end_byte < text.len() {
        snippet.push_str("...");
    }
    snippet
}
