use super::*;

impl SqliteStore {
    // ─── Episodic Memory (Scoped FTS5 + SimHash/Jaccard Dedup + RRF Hybrid) ────

    pub fn add_memory(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
    ) -> Result<MemoryEntry> {
        let res = self.upsert_memory_dedup(scope, kind, title, content, salience, None, None, true)?;
        Ok(res.entry)
    }

    /// Inserts a sanitized, SimHash-indexed memory entry — or merges into an existing near-duplicate
    /// in the same scope when `dedup_enabled` and `shingle_jaccard >= MEMORY_DEDUP_JACCARD_THRESHOLD`.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_memory_dedup(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
        source_session_id: Option<&str>,
        source_event_id: Option<&str>,
        dedup_enabled: bool,
    ) -> Result<MemoryUpsertResult> {
        self.upsert_memory_dedup_with_secrets(
            scope,
            kind,
            title,
            content,
            salience,
            source_session_id,
            source_event_id,
            dedup_enabled,
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_memory_dedup_with_secrets(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
        source_session_id: Option<&str>,
        source_event_id: Option<&str>,
        dedup_enabled: bool,
        known_secrets: &[String],
    ) -> Result<MemoryUpsertResult> {
        let clean_title = sanitize_memory_text_with_secrets(title.trim(), known_secrets);
        let clean_content = sanitize_memory_text_with_secrets(content.trim(), known_secrets);
        let norm_scope = match scope.trim().is_empty() {
            true => MEMORY_SCOPE_GLOBAL.to_string(),
            false => scope.trim().to_string(),
        };
        let norm_kind = MemoryKind::from_str_loose(kind).as_str().to_string();
        let clamped_salience = salience.clamp(0.1, 3.0);
        let combined = format!("{clean_title} {clean_content}");
        let simhash = compute_simhash64(&combined);
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        if dedup_enabled {
            let existing: Vec<MemoryEntry> = self.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                     FROM memory_entries
                     WHERE scope = ?1 AND archived = 0
                     ORDER BY created_at DESC LIMIT 200",
                )?;
                let rows = stmt.query_map(params![norm_scope], row_to_memory)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })?;

            let mut best_match: Option<(MemoryEntry, f64)> = None;
            for cand in existing {
                let cand_text = format!("{} {}", cand.title, cand.content);
                let cand_hash = match cand.content_simhash {
                    0 => compute_simhash64(&cand_text),
                    h => h,
                };
                if hamming_distance(simhash, cand_hash) > MEMORY_DEDUP_SIMHASH_MAX_HAMMING
                    && !cand.title.eq_ignore_ascii_case(&clean_title)
                {
                    continue;
                }
                let sim = shingle_jaccard(&combined, &cand_text);
                if sim >= MEMORY_DEDUP_JACCARD_THRESHOLD
                    && best_match.as_ref().is_none_or(|(_, s)| sim > *s)
                {
                    best_match = Some((cand, sim));
                }
            }

            if let Some((mut prev, sim)) = best_match {
                let merged_salience = prev.salience.max(clamped_salience).min(3.0) + 0.05;
                let merged_salience = merged_salience.min(3.0);
                let merged_content = match clean_content.len() >= prev.content.len() {
                    true => clean_content,
                    false => prev.content.clone(),
                };
                let merged_title = match clean_title.is_empty() {
                    true => prev.title.clone(),
                    false => clean_title,
                };
                let merged_hash = compute_simhash64(&format!("{merged_title} {merged_content}"));
                let merged_sid = source_session_id
                    .map(String::from)
                    .or_else(|| prev.source_session_id.clone());
                let merged_eid = source_event_id
                    .map(String::from)
                    .or_else(|| prev.source_event_id.clone());

                self.with_conn(|conn| {
                    conn.execute(
                        "UPDATE memory_entries
                         SET kind = ?1, title = ?2, content = ?3, salience = ?4, updated_at = ?5,
                             content_simhash = ?6, source_session_id = ?7, source_event_id = ?8
                         WHERE id = ?9",
                        params![
                            norm_kind,
                            merged_title,
                            merged_content,
                            merged_salience,
                            now_str,
                            merged_hash as i64,
                            merged_sid,
                            merged_eid,
                            prev.id,
                        ],
                    )?;
                    conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![prev.id])?;
                    conn.execute(
                        "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                        params![prev.id, merged_title, merged_content],
                    )?;
                    Ok(())
                })?;

                prev.kind = norm_kind;
                prev.title = merged_title;
                prev.content = merged_content;
                prev.salience = merged_salience;
                prev.updated_at = Some(now);
                prev.content_simhash = merged_hash;
                prev.source_session_id = merged_sid;
                prev.source_event_id = merged_eid;
                return Ok(MemoryUpsertResult {
                    entry: prev,
                    merged: true,
                    similarity: sim,
                });
            }
        }

        let entry = MemoryEntry {
            id: new_id(),
            scope: norm_scope,
            kind: norm_kind,
            title: clean_title,
            content: clean_content,
            salience: clamped_salience,
            access_count: 0,
            last_accessed_at: None,
            updated_at: Some(now),
            content_simhash: simhash,
            source_session_id: source_session_id.map(String::from),
            source_event_id: source_event_id.map(String::from),
            archived: false,
            created_at: now,
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO memory_entries
                 (id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8, ?9, ?10, 0, ?11)",
                params![
                    entry.id,
                    entry.scope,
                    entry.kind,
                    entry.title,
                    entry.content,
                    entry.salience,
                    now_str,
                    entry.content_simhash as i64,
                    entry.source_session_id,
                    entry.source_event_id,
                    now_str,
                ],
            )?;
            conn.execute(
                "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                params![entry.id, entry.title, entry.content],
            )?;
            Ok(())
        })?;
        Ok(MemoryUpsertResult {
            entry,
            merged: false,
            similarity: 0.0,
        })
    }

    pub fn update_memory(
        &self,
        id: &str,
        title: Option<&str>,
        content: Option<&str>,
        kind: Option<&str>,
        salience: Option<f64>,
        scope: Option<&str>,
    ) -> Result<Option<MemoryEntry>> {
        self.update_memory_with_secrets(id, title, content, kind, salience, scope, &[])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_memory_with_secrets(
        &self,
        id: &str,
        title: Option<&str>,
        content: Option<&str>,
        kind: Option<&str>,
        salience: Option<f64>,
        scope: Option<&str>,
        known_secrets: &[String],
    ) -> Result<Option<MemoryEntry>> {
        let Some(mut cur) = self.get_memory(id)? else {
            return Ok(None);
        };
        if let Some(t) = title.map(str::trim).filter(|s| !s.is_empty()) {
            cur.title = sanitize_memory_text_with_secrets(t, known_secrets);
        }
        if let Some(c) = content.map(str::trim).filter(|s| !s.is_empty()) {
            cur.content = sanitize_memory_text_with_secrets(c, known_secrets);
        }
        if let Some(k) = kind.map(str::trim).filter(|s| !s.is_empty()) {
            cur.kind = MemoryKind::from_str_loose(k).as_str().to_string();
        }
        if let Some(s) = salience {
            cur.salience = s.clamp(0.1, 3.0);
        }
        if let Some(sc) = scope.map(str::trim).filter(|s| !s.is_empty()) {
            cur.scope = sc.to_string();
        }
        let now = Utc::now();
        cur.updated_at = Some(now);
        cur.content_simhash = compute_simhash64(&format!("{} {}", cur.title, cur.content));

        self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            tx.execute(
                "UPDATE memory_entries
                 SET scope = ?1, kind = ?2, title = ?3, content = ?4, salience = ?5, updated_at = ?6, content_simhash = ?7
                 WHERE id = ?8",
                params![
                    cur.scope,
                    cur.kind,
                    cur.title,
                    cur.content,
                    cur.salience,
                    now.to_rfc3339(),
                    cur.content_simhash as i64,
                    cur.id,
                ],
            )?;
            tx.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![cur.id])?;
            if !cur.archived {
                tx.execute(
                    "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                    params![cur.id, cur.title, cur.content],
                )?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(Some(cur))
    }

    pub fn get_memory(&self, id: &str) -> Result<Option<MemoryEntry>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries WHERE id = ?1",
                params![id],
                row_to_memory,
            )
            .optional()
        })
    }

    pub fn archive_memory(&self, id: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let n = self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            tx.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![id])?;
            let changed = tx.execute(
                "UPDATE memory_entries SET archived = 1, updated_at = ?1 WHERE id = ?2 AND archived = 0",
                params![now, id],
            )?;
            tx.commit()?;
            Ok(changed)
        })?;
        Ok(n > 0)
    }

    pub fn search_memory_fts(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let hits = self.search_memory_hybrid(query, None, true, None, limit)?;
        Ok(hits.into_iter().map(|h| h.entry).collect())
    }

    /// Workspace-scoped pure-Rust Reciprocal Rank Fusion (RRF) memory search:
    /// 1. 2-tier FTS5 query planner (strict AND -> relaxed OR fallback)
    /// 2. Symbol-aware token-shingle Jaccard re-ranking
    /// 3. Ebbinghaus temporal decay + access reinforcement + git-root project boost
    /// 4. Live filesystem path staleness verification (when `workspace_dir` is provided)
    /// 5. Atomic `access_count` + `last_accessed_at` reinforcement on returned hits
    pub fn search_memory_hybrid(
        &self,
        query: &str,
        project_scope: Option<&str>,
        include_global: bool,
        workspace_dir: Option<&Path>,
        limit: usize,
    ) -> Result<Vec<ScoredMemoryHit>> {
        let cap = limit.clamp(1, 50);
        let trimmed = query.trim();

        let scope_matches = |entry_scope: &str| -> bool {
            match (project_scope, include_global) {
                (Some(ps), true) => entry_scope == ps || entry_scope == MEMORY_SCOPE_GLOBAL,
                (Some(ps), false) => entry_scope == ps,
                (None, true) => true,
                (None, false) => entry_scope == MEMORY_SCOPE_GLOBAL,
            }
        };

        if trimmed.is_empty() {
            let rows = self.list_memories_scoped(project_scope, include_global, false, cap)?;
            let now = Utc::now();
            return Ok(rows
                .into_iter()
                .map(|entry| {
                    let stale_paths = workspace_dir
                        .map(|ws| find_missing_workspace_paths(ws, &entry.content))
                        .unwrap_or_default();
                    let age = (now - entry.updated_at.unwrap_or(entry.created_at)).num_hours() as f64
                        / 24.0;
                    let is_proj = project_scope.is_some_and(|ps| entry.scope == ps);
                    let score = hybrid_memory_score(
                        None,
                        None,
                        age,
                        MemoryKind::from_str_loose(&entry.kind),
                        entry.salience,
                        entry.access_count,
                        is_proj,
                        !stale_paths.is_empty(),
                    );
                    ScoredMemoryHit {
                        entry,
                        score,
                        stale_paths,
                    }
                })
                .collect());
        }

        let plan = build_fts5_query_plan(trimmed);
        let candidate_cap = (cap * 4).max(24) as i64;
        let mut bm25_ordered: Vec<MemoryEntry> = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        self.with_conn(|conn| {
            let run_fts = |fts_q: &str,
                           require_overlap: bool,
                           out: &mut Vec<MemoryEntry>,
                           seen: &mut std::collections::HashSet<String>|
             -> rusqlite::Result<()> {
                let collected: Vec<MemoryEntry> = match (project_scope, include_global) {
                    (Some(ps), true) => {
                        let mut stmt = conn.prepare(
                            "SELECT m.id, m.scope, m.kind, m.title, m.content, m.salience, m.access_count,
                                    m.last_accessed_at, m.updated_at, m.content_simhash,
                                    m.source_session_id, m.source_event_id, m.archived, m.created_at
                             FROM memory_fts f
                             JOIN memory_entries m ON m.id = f.entry_id
                             WHERE memory_fts MATCH ?1 AND m.archived = 0
                               AND (m.scope = ?2 OR m.scope = ?3)
                             ORDER BY bm25(memory_fts) ASC
                             LIMIT ?4",
                        )?;
                        stmt.query_map(
                            params![fts_q, ps, MEMORY_SCOPE_GLOBAL, candidate_cap],
                            row_to_memory,
                        )?
                        .flatten()
                        .collect()
                    }
                    (Some(ps), false) => {
                        let mut stmt = conn.prepare(
                            "SELECT m.id, m.scope, m.kind, m.title, m.content, m.salience, m.access_count,
                                    m.last_accessed_at, m.updated_at, m.content_simhash,
                                    m.source_session_id, m.source_event_id, m.archived, m.created_at
                             FROM memory_fts f
                             JOIN memory_entries m ON m.id = f.entry_id
                             WHERE memory_fts MATCH ?1 AND m.archived = 0
                               AND m.scope = ?2
                             ORDER BY bm25(memory_fts) ASC
                             LIMIT ?3",
                        )?;
                        stmt.query_map(params![fts_q, ps, candidate_cap], row_to_memory)?
                            .flatten()
                            .collect()
                    }
                    (None, true) => {
                        let mut stmt = conn.prepare(
                            "SELECT m.id, m.scope, m.kind, m.title, m.content, m.salience, m.access_count,
                                    m.last_accessed_at, m.updated_at, m.content_simhash,
                                    m.source_session_id, m.source_event_id, m.archived, m.created_at
                             FROM memory_fts f
                             JOIN memory_entries m ON m.id = f.entry_id
                             WHERE memory_fts MATCH ?1 AND m.archived = 0
                             ORDER BY bm25(memory_fts) ASC
                             LIMIT ?2",
                        )?;
                        stmt.query_map(params![fts_q, candidate_cap], row_to_memory)?
                            .flatten()
                            .collect()
                    }
                    (None, false) => {
                        let mut stmt = conn.prepare(
                            "SELECT m.id, m.scope, m.kind, m.title, m.content, m.salience, m.access_count,
                                    m.last_accessed_at, m.updated_at, m.content_simhash,
                                    m.source_session_id, m.source_event_id, m.archived, m.created_at
                             FROM memory_fts f
                             JOIN memory_entries m ON m.id = f.entry_id
                             WHERE memory_fts MATCH ?1 AND m.archived = 0
                               AND m.scope = ?2
                             ORDER BY bm25(memory_fts) ASC
                             LIMIT ?3",
                        )?;
                        stmt.query_map(
                            params![fts_q, MEMORY_SCOPE_GLOBAL, candidate_cap],
                            row_to_memory,
                        )?
                        .flatten()
                        .collect()
                    }
                };
                for row in collected {
                    if !scope_matches(&row.scope) {
                        continue;
                    }
                    if require_overlap {
                        let hay = format!("{} {}", row.title, row.content);
                        if relaxed_term_overlap(&plan.signal_terms, &hay) < MEMORY_RELAXED_MIN_OVERLAP
                        {
                            continue;
                        }
                    }
                    if seen.insert(row.id.clone()) {
                        out.push(row);
                    }
                }
                Ok(())
            };

            if let Some(ref eq) = plan.exact_and {
                let _ = run_fts(eq, false, &mut bm25_ordered, &mut seen_ids);
            }
            if bm25_ordered.len() < cap
                && let Some(ref sq) = plan.split_and {
                    let _ = run_fts(sq, false, &mut bm25_ordered, &mut seen_ids);
                }
            if bm25_ordered.len() < cap
                && let Some(ref rq) = plan.relaxed_or {
                    let _ = run_fts(rq, true, &mut bm25_ordered, &mut seen_ids);
                }
            Ok(())
        })?;

        if bm25_ordered.is_empty() {
            return Ok(Vec::new());
        }

        // Secondary lexical rank via symbol-aware shingle Jaccard overlap
        let mut jac_pairs: Vec<(usize, f64)> = bm25_ordered
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let text = format!("{} {}", e.title, e.content);
                (idx, shingle_jaccard(trimmed, &text))
            })
            .collect();
        jac_pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut jac_rank_map: HashMap<usize, usize> = HashMap::with_capacity(jac_pairs.len());
        for (rank, (orig_idx, sim)) in jac_pairs.into_iter().enumerate() {
            if sim > 0.0 {
                jac_rank_map.insert(orig_idx, rank);
            }
        }

        let now = Utc::now();
        let mut scored: Vec<ScoredMemoryHit> = bm25_ordered
            .into_iter()
            .enumerate()
            .map(|(bm25_rank, entry)| {
                let stale_paths = workspace_dir
                    .map(|ws| find_missing_workspace_paths(ws, &entry.content))
                    .unwrap_or_default();
                let ref_ts = entry.updated_at.unwrap_or(entry.created_at);
                let age_days = (now - ref_ts).num_hours().max(0) as f64 / 24.0;
                let is_proj = project_scope.is_some_and(|ps| entry.scope == ps);
                let score = hybrid_memory_score(
                    Some(bm25_rank),
                    jac_rank_map.get(&bm25_rank).copied(),
                    age_days,
                    MemoryKind::from_str_loose(&entry.kind),
                    entry.salience,
                    entry.access_count,
                    is_proj,
                    !stale_paths.is_empty(),
                );
                ScoredMemoryHit {
                    entry,
                    score,
                    stale_paths,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(cap);

        // ACT-R access reinforcement on returned hits
        let now_str = now.to_rfc3339();
        let hit_ids: Vec<String> = scored.iter().map(|h| h.entry.id.clone()).collect();
        let _ = self.with_conn(|conn| {
            for id in &hit_ids {
                let _ = conn.execute(
                    "UPDATE memory_entries SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
                    params![now_str, id],
                );
            }
            Ok(())
        });
        for hit in &mut scored {
            hit.entry.access_count += 1;
            hit.entry.last_accessed_at = Some(now);
        }

        Ok(scored)
    }

    pub fn list_memories(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        self.list_memories_scoped(None, true, false, limit)
    }

    pub fn list_memories_scoped(
        &self,
        project_scope: Option<&str>,
        include_global: bool,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries WHERE 1=1",
            );
            let mut bind_vals: Vec<rusqlite::types::Value> = Vec::new();
            if !include_archived {
                sql.push_str(" AND archived = 0");
            }
            match (project_scope, include_global) {
                (Some(ps), true) => {
                    sql.push_str(" AND (scope = ? OR scope = ?)");
                    bind_vals.push(rusqlite::types::Value::Text(ps.to_string()));
                    bind_vals.push(rusqlite::types::Value::Text(MEMORY_SCOPE_GLOBAL.to_string()));
                }
                (Some(ps), false) => {
                    sql.push_str(" AND scope = ?");
                    bind_vals.push(rusqlite::types::Value::Text(ps.to_string()));
                }
                (None, false) => {
                    sql.push_str(" AND scope = ?");
                    bind_vals.push(rusqlite::types::Value::Text(MEMORY_SCOPE_GLOBAL.to_string()));
                }
                (None, true) => {}
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            bind_vals.push(rusqlite::types::Value::Integer(limit.clamp(1, 200) as i64));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(bind_vals), row_to_memory)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Stage-0 zero-LLM deterministic hygiene:
    /// 1. Oldest-first SQL sweep archiving dormant low-salience entries older than `MEMORY_ARCHIVE_INACTIVE_DAYS`
    ///    (never cut off by recent-200 caps even on multi-month 24/7 databases).
    /// 2. Live workspace path staleness check with bounded salience demotion (`MEMORY_STALE_SALIENCE_DEMOTION`).
    /// 3. SimHash + Jaccard near-duplicate archival within each scope.
    pub fn run_memory_hygiene(&self, workspace_dir: Option<&Path>) -> Result<MemoryHygieneReport> {
        let mut report = MemoryHygieneReport::default();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cutoff = (now - Duration::days(MEMORY_ARCHIVE_INACTIVE_DAYS)).to_rfc3339();

        // Pass 1: Dedicated oldest-first archival of dormant low-salience memories
        let dormant_ids: Vec<String> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM memory_entries
                 WHERE archived = 0
                   AND access_count = 0
                   AND salience <= ?1
                   AND COALESCE(last_accessed_at, updated_at, created_at) < ?2
                 ORDER BY created_at ASC
                 LIMIT 500",
            )?;
            let rows = stmt.query_map(params![MEMORY_ARCHIVE_MAX_SALIENCE, cutoff], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        for id in dormant_ids {
            if self.archive_memory(&id).unwrap_or(false) {
                report.entries_archived += 1;
            }
        }

        // Pass 2: Active scan (up to 500 entries) for stale-path demotion + SimHash/Jaccard dedup
        let entries: Vec<MemoryEntry> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries
                 WHERE archived = 0
                 ORDER BY created_at DESC LIMIT 500",
            )?;
            let rows = stmt.query_map([], row_to_memory)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        let mut kept: Vec<(String, String, u64, String)> = Vec::new();
        for e in entries {
            if let Some(ws) = workspace_dir
                && !find_missing_workspace_paths(ws, &e.content).is_empty() {
                    report.stale_flagged += 1;
                    let demoted = (e.salience - MEMORY_STALE_SALIENCE_DEMOTION).max(0.2);
                    if (demoted - e.salience).abs() > f64::EPSILON {
                        let _ = self.with_conn(|conn| {
                            conn.execute(
                                "UPDATE memory_entries SET salience = ?1, updated_at = ?2 WHERE id = ?3",
                                params![demoted, now_str, e.id],
                            )?;
                            Ok(())
                        });
                    }
                }

            let text = format!("{} {}", e.title, e.content);
            let hash = match e.content_simhash {
                0 => compute_simhash64(&text),
                h => h,
            };
            let is_dup = kept.iter().any(|(_, sc, kh, kt)| {
                sc == &e.scope
                    && hamming_distance(hash, *kh) <= MEMORY_DEDUP_SIMHASH_MAX_HAMMING
                    && shingle_jaccard(&text, kt) >= MEMORY_DEDUP_JACCARD_THRESHOLD
            });
            match is_dup {
                true => {
                    if self.archive_memory(&e.id).unwrap_or(false) {
                        report.duplicates_merged += 1;
                    }
                }
                false => kept.push((e.id, e.scope, hash, text)),
            }
        }
        Ok(report)
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![id])?;
            conn.execute("DELETE FROM memory_entries WHERE id = ?1", params![id])
        })?;
        Ok(n > 0)
    }

    // ─── Cross-Session Recall (`session_events_fts`) ───────────────────────────

    /// Sub-millisecond cross-session FTS5 recall with 3-tier exact->split->relaxed planner and filters.
    pub fn search_sessions_fts(
        &self,
        filter: &SessionSearchFilter,
    ) -> Result<Vec<SessionSearchHit>> {
        let limit = filter
            .limit
            .unwrap_or(SESSION_SEARCH_DEFAULT_LIMIT)
            .clamp(1, SESSION_SEARCH_MAX_LIMIT);
        let plan = build_fts5_query_plan(&filter.query);
        if plan.exact_and.is_none() && plan.split_and.is_none() && plan.relaxed_or.is_none() {
            return Ok(Vec::new());
        }

        let mut results: Vec<SessionSearchHit> = Vec::new();
        let mut seen_events = std::collections::HashSet::new();

        self.with_conn(|conn| {
            let run_query = |fts_q: &str,
                             rem: usize,
                             require_overlap: bool,
                             out: &mut Vec<SessionSearchHit>,
                             seen: &mut std::collections::HashSet<String>|
             -> rusqlite::Result<()> {
                let mut sql = String::from(
                    "SELECT s.id, s.title, s.channel, s.workspace_dir, e.id, e.role,
                            snippet(session_events_fts, 3, '', '', '...', 24) AS snip,
                            bm25(session_events_fts) AS rank,
                            e.created_at
                     FROM session_events_fts f
                     JOIN session_events e ON e.id = f.event_id
                     JOIN sessions s ON s.id = f.session_id
                     WHERE session_events_fts MATCH ?",
                );
                let mut binds: Vec<rusqlite::types::Value> =
                    vec![rusqlite::types::Value::Text(fts_q.to_string())];

                if let Some(since) = filter.since {
                    sql.push_str(" AND e.created_at >= ?");
                    binds.push(rusqlite::types::Value::Text(since.to_rfc3339()));
                }
                if let Some(ref ch) = filter.channel
                    && !ch.trim().is_empty() && !ch.eq_ignore_ascii_case("all") {
                        sql.push_str(" AND s.channel = ?");
                        binds.push(rusqlite::types::Value::Text(ch.trim().to_string()));
                    }
                if let Some(ref ws) = filter.workspace_dir
                    && !ws.trim().is_empty() {
                        sql.push_str(" AND s.workspace_dir = ?");
                        binds.push(rusqlite::types::Value::Text(ws.trim().to_string()));
                    }
                if let Some(ref sid) = filter.session_id
                    && !sid.trim().is_empty() {
                        sql.push_str(" AND s.id = ?");
                        binds.push(rusqlite::types::Value::Text(sid.trim().to_string()));
                    }
                if let Some(ref ex_sid) = filter.exclude_session_id
                    && !ex_sid.trim().is_empty() {
                        sql.push_str(" AND s.id != ?");
                        binds.push(rusqlite::types::Value::Text(ex_sid.trim().to_string()));
                    }
                if let Some(ref role) = filter.role
                    && !role.trim().is_empty() && !role.eq_ignore_ascii_case("all") {
                        sql.push_str(" AND e.role = ?");
                        binds.push(rusqlite::types::Value::Text(role.trim().to_string()));
                    }

                sql.push_str(" ORDER BY rank ASC, e.created_at DESC LIMIT ?");
                binds.push(rusqlite::types::Value::Integer((rem * 2).max(8) as i64));

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| {
                    Ok(SessionSearchHit {
                        session_id: r.get(0)?,
                        session_title: r.get(1)?,
                        channel: r.get(2)?,
                        workspace_dir: r.get(3)?,
                        event_id: r.get(4)?,
                        role: r.get(5)?,
                        snippet: r.get(6)?,
                        rank: r.get(7)?,
                        created_at: parse_dt(&r.get::<_, String>(8)?),
                    })
                })?;
                for hit in rows.flatten() {
                    if require_overlap
                        && relaxed_term_overlap(&plan.signal_terms, &hit.snippet)
                            < MEMORY_RELAXED_MIN_OVERLAP
                    {
                        continue;
                    }
                    if seen.insert(hit.event_id.clone()) {
                        out.push(hit);
                        if out.len() >= limit {
                            break;
                        }
                    }
                }
                Ok(())
            };

            if let Some(ref eq) = plan.exact_and {
                let _ = run_query(eq, limit, false, &mut results, &mut seen_events);
            }
            if results.len() < limit
                && let Some(ref sq) = plan.split_and {
                    let rem = limit.saturating_sub(results.len());
                    let _ = run_query(sq, rem, false, &mut results, &mut seen_events);
                }
            if results.len() < limit
                && let Some(ref rq) = plan.relaxed_or {
                    let rem = limit.saturating_sub(results.len());
                    let _ = run_query(rq, rem, true, &mut results, &mut seen_events);
                }
            Ok(())
        })?;

        Ok(results)
    }

    /// Hermes-style lineage traversal: expands ±`window` surrounding events around `event_id`
    /// on that session's active branch (or chronological event log) in sub-millisecond SQLite I/O.
    pub fn get_session_event_context(
        &self,
        event_id: &str,
        window: usize,
    ) -> Result<Option<SessionContextWindow>> {
        let sid_opt: Option<String> = self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id FROM session_events WHERE id = ?1",
                params![event_id],
                |r| r.get(0),
            )
            .optional()
        })?;
        let Some(sid) = sid_opt else {
            return Ok(None);
        };
        let Some(session) = self.get_session(&sid)? else {
            return Ok(None);
        };
        let mut events = self.get_active_branch(&sid)?;
        if !events.iter().any(|e| e.id == event_id) {
            events = self.list_all_session_events(&sid)?;
        }
        let Some(anchor_idx) = events.iter().position(|e| e.id == event_id) else {
            return Ok(None);
        };
        let rad = window.clamp(1, SESSION_CONTEXT_MAX_WINDOW);
        let start = anchor_idx.saturating_sub(rad);
        let end = (anchor_idx + rad + 1).min(events.len());
        let turns = events[start..end]
            .iter()
            .filter_map(|ev| {
                let text = extract_searchable_event_text(ev.role, ev.kind, &ev.content_json);
                if text.is_empty() && ev.id != event_id {
                    return None;
                }
                Some(SessionContextTurn {
                    event_id: ev.id.clone(),
                    role: ev.role.as_str().to_string(),
                    kind: ev.kind.as_str().to_string(),
                    text,
                    is_anchor: ev.id == event_id,
                    created_at: ev.created_at,
                })
            })
            .collect();

        Ok(Some(SessionContextWindow {
            session_id: session.id,
            session_title: session.title,
            channel: session.channel,
            workspace_dir: session.workspace_dir,
            anchor_event_id: event_id.to_string(),
            events: turns,
        }))
    }

    pub fn get_rollout_summary(
        &self,
        session_id: &str,
        expected_updated_at: &str,
    ) -> Result<Option<SessionRolloutSummary>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id, scope, digest, session_updated_at, created_at
                 FROM session_rollout_summaries
                 WHERE session_id = ?1 AND session_updated_at = ?2",
                params![session_id, expected_updated_at],
                |r| {
                    Ok(SessionRolloutSummary {
                        session_id: r.get(0)?,
                        scope: r.get(1)?,
                        digest: r.get(2)?,
                        session_updated_at: r.get(3)?,
                        created_at: parse_dt(&r.get::<_, String>(4)?),
                    })
                },
            )
            .optional()
        })
    }

    pub fn upsert_rollout_summary(&self, summary: &SessionRolloutSummary) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO session_rollout_summaries
                 (session_id, scope, digest, session_updated_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    summary.session_id,
                    summary.scope,
                    summary.digest,
                    summary.session_updated_at,
                    summary.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    // ─── Autonomous Dream Consolidation Lease, Watermark & Audit Log ───────────

    /// Transactional time-bounded SQLite lease (Codex `codex-rs` pattern — self-heals on crash,
    /// immune to stale lockfiles). Uses an RAII `IMMEDIATE` transaction so any error automatically
    /// rolls back without leaving the shared connection stuck inside an open transaction.
    pub fn try_claim_dream_lease(&self, owner_token: &str, lease_secs: i64) -> Result<bool> {
        let now = Utc::now();
        let clamped_secs = lease_secs.clamp(30, 86_400 * 7);
        let expires = now + Duration::seconds(clamped_secs);
        self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let existing: Option<String> = tx
                .query_row(
                    "SELECT v FROM kv_state WHERE k = ?1",
                    params![DREAM_KV_LEASE_KEY],
                    |r| r.get(0),
                )
                .optional()?;
            let occupied = existing
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .is_some_and(|v| {
                    let owner = v["owner"].as_str().unwrap_or("");
                    let exp = v["expires_at"].as_str().map(parse_dt).unwrap_or(now);
                    owner != owner_token && exp > now
                });
            if occupied {
                tx.rollback()?;
                return Ok(false);
            }
            let payload = serde_json::json!({
                "owner": owner_token,
                "expires_at": expires.to_rfc3339(),
            })
            .to_string();
            tx.execute(
                "INSERT OR REPLACE INTO kv_state (k, v, updated_at) VALUES (?1, ?2, ?3)",
                params![DREAM_KV_LEASE_KEY, payload, now.to_rfc3339()],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn release_dream_lease(&self, owner_token: &str) -> Result<()> {
        self.with_conn(|conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT v FROM kv_state WHERE k = ?1",
                    params![DREAM_KV_LEASE_KEY],
                    |r| r.get(0),
                )
                .optional()?;
            let owns = existing
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .is_some_and(|v| v["owner"].as_str() == Some(owner_token));
            if owns {
                conn.execute("DELETE FROM kv_state WHERE k = ?1", params![DREAM_KV_LEASE_KEY])?;
            }
            Ok(())
        })
    }

    pub fn is_dream_lease_active(&self) -> bool {
        let now = Utc::now();
        self.get_kv(DREAM_KV_LEASE_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_some_and(|v| {
                v["expires_at"].as_str().map(parse_dt).is_some_and(|e| e > now)
            })
    }

    /// Lists idle sessions updated after `since_ts` that have at least `min_events` events.
    pub fn list_sessions_for_dream(
        &self,
        since_ts: Option<&str>,
        min_events: usize,
        limit: usize,
    ) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id,
                        s.model_override, s.permission_mode, s.status, s.head_event_id,
                        s.frozen_system_hash, s.workspace_dir, s.parent_session_id, s.agent_role,
                        s.agent_depth, s.worktree_path, s.worktree_branch, s.total_tokens,
                        s.total_cost_usd, s.created_at, s.updated_at
                 FROM sessions s
                 WHERE s.status = 'idle'
                   AND s.parent_session_id IS NULL
                   AND (SELECT COUNT(*) FROM session_events e WHERE e.session_id = s.id AND e.role != 'system') >= ?",
            );
            let mut binds: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Integer(min_events as i64)];
            if let Some(ts) = since_ts.map(str::trim).filter(|s| !s.is_empty()) {
                sql.push_str(" AND s.updated_at > ?");
                binds.push(rusqlite::types::Value::Text(ts.to_string()));
            }
            sql.push_str(" ORDER BY s.updated_at ASC LIMIT ?");
            binds.push(rusqlite::types::Value::Integer(limit.clamp(1, 50) as i64));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn insert_dream_run(&self, rec: &DreamRunRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO dream_runs
                 (id, trigger, status, model, sessions_scanned, memories_added, memories_merged, memories_archived, docs_patched, skills_proposed, diff_preview, detail, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    rec.id,
                    rec.trigger,
                    rec.status,
                    rec.model,
                    rec.sessions_scanned as i64,
                    rec.memories_added as i64,
                    rec.memories_merged as i64,
                    rec.memories_archived as i64,
                    rec.docs_patched as i64,
                    rec.skills_proposed as i64,
                    rec.diff_preview,
                    rec.detail,
                    rec.started_at.to_rfc3339(),
                    rec.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn finish_dream_run(&self, rec: &DreamRunRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE dream_runs
                 SET status = ?1, model = ?2, sessions_scanned = ?3, memories_added = ?4,
                     memories_merged = ?5, memories_archived = ?6, docs_patched = ?7,
                     skills_proposed = ?8, diff_preview = ?9, detail = ?10, finished_at = ?11
                 WHERE id = ?12",
                params![
                    rec.status,
                    rec.model,
                    rec.sessions_scanned as i64,
                    rec.memories_added as i64,
                    rec.memories_merged as i64,
                    rec.memories_archived as i64,
                    rec.docs_patched as i64,
                    rec.skills_proposed as i64,
                    rec.diff_preview,
                    rec.detail,
                    rec.finished_at.map(|t| t.to_rfc3339()),
                    rec.id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_dream_runs(&self, limit: usize) -> Result<Vec<DreamRunRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, trigger, status, model, sessions_scanned, memories_added, memories_merged,
                        memories_archived, docs_patched, skills_proposed, diff_preview, detail, started_at, finished_at
                 FROM dream_runs ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit.clamp(1, 50) as i64], |r| {
                Ok(DreamRunRecord {
                    id: r.get(0)?,
                    trigger: r.get(1)?,
                    status: r.get(2)?,
                    model: r.get(3)?,
                    sessions_scanned: r.get::<_, i64>(4)?.max(0) as usize,
                    memories_added: r.get::<_, i64>(5)?.max(0) as usize,
                    memories_merged: r.get::<_, i64>(6)?.max(0) as usize,
                    memories_archived: r.get::<_, i64>(7)?.max(0) as usize,
                    docs_patched: r.get::<_, i64>(8)?.max(0) as usize,
                    skills_proposed: r.get::<_, i64>(9).unwrap_or(0).max(0) as usize,
                    diff_preview: r.get(10)?,
                    detail: r.get(11)?,
                    started_at: parse_dt(&r.get::<_, String>(12)?),
                    finished_at: r.get::<_, Option<String>>(13)?.map(|s| parse_dt(&s)),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

}
