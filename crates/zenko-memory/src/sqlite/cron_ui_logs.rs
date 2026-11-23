use super::*;

impl SqliteStore {
    // ─── Cron & Heartbeats ─────────────────────────────────────────────────────

    pub fn upsert_cron_job(&self, job: &CronJob) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO cron_jobs
                 (id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    job.id,
                    job.name,
                    job.cron_expr,
                    job.kind.as_str(),
                    job.prompt,
                    job.script_path,
                    job.target_channel,
                    job.target_chat_id,
                    job.target_thread_id,
                    if job.enabled { 1 } else { 0 },
                    job.next_run_at.map(|t| t.to_rfc3339()),
                    job.last_run_at.map(|t| t.to_rfc3339()),
                    job.last_status,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status
                 FROM cron_jobs WHERE id = ?1",
                params![id],
                row_to_cron,
            )
            .optional()
        })
    }

    pub fn list_cron_jobs(&self) -> Result<Vec<CronJob>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status
                 FROM cron_jobs ORDER BY name ASC",
            )?;
            let rows = stmt.query_map([], row_to_cron)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_cron_job(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id]))?;
        Ok(n > 0)
    }

    // ─── UI Profiles (Deep Customization Engine) ───────────────────────────────

    pub fn get_active_ui_profile(&self) -> Result<UiProfile> {
        let opt = self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, is_active, config_json, updated_at FROM ui_profiles WHERE is_active = 1 LIMIT 1",
                [],
                row_to_ui_profile,
            )
            .optional()
        })?;
        match opt {
            Some(p) => Ok(p),
            None => Ok(UiProfile {
                id: "default".into(),
                name: "Zenko Vermilion (Default)".into(),
                is_active: true,
                config: UiCustomization::default(),
                updated_at: Utc::now(),
            }),
        }
    }

    pub fn list_ui_profiles(&self) -> Result<Vec<UiProfile>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, is_active, config_json, updated_at FROM ui_profiles ORDER BY is_active DESC, updated_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_ui_profile)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn save_ui_profile(&self, profile: &UiProfile) -> Result<()> {
        let json = serde_json::to_string(&profile.config)?;
        self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            if profile.is_active {
                tx.execute("UPDATE ui_profiles SET is_active = 0", [])?;
            }
            tx.execute(
                "INSERT OR REPLACE INTO ui_profiles (id, name, is_active, config_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile.id,
                    profile.name,
                    if profile.is_active { 1 } else { 0 },
                    json,
                    profile.updated_at.to_rfc3339(),
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn activate_ui_profile(&self, id: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let n = self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM ui_profiles WHERE id = ?1 LIMIT 1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                tx.rollback()?;
                return Ok(0);
            }
            tx.execute("UPDATE ui_profiles SET is_active = 0", [])?;
            let changed = tx.execute(
                "UPDATE ui_profiles SET is_active = 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            tx.commit()?;
            Ok(changed)
        })?;
        Ok(n > 0)
    }

    pub fn delete_ui_profile(&self, id: &str) -> Result<bool> {
        if id == "default" {
            return Ok(false);
        }
        let n = self.with_conn(|conn| conn.execute("DELETE FROM ui_profiles WHERE id = ?1", params![id]))?;
        self.ensure_default_ui_profile()?;
        Ok(n > 0)
    }

    // ─── Usage & Cost Ledger ───────────────────────────────────────────────────

    pub fn record_usage(&self, session_id: &str, provider: &str, model: &str, u: &UsageStats) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO usage_ledger (id, session_id, provider, model, prompt_tokens, completion_tokens, cache_read_tokens, cost_usd, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    new_id(),
                    session_id,
                    provider,
                    model,
                    u.prompt_tokens as i64,
                    u.completion_tokens as i64,
                    u.cache_read_tokens as i64,
                    u.cost_usd,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn daily_cost_usd(&self) -> Result<f64> {
        let day_start = Utc::now().format("%Y-%m-%dT00:00:00").to_string();
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_ledger WHERE created_at >= ?1",
                params![day_start],
                |r| r.get(0),
            )
        })
    }

    pub fn total_usage_summary(&self) -> Result<(f64, f64)> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(cache_read_tokens), 0) FROM usage_ledger",
                [],
                |r| {
                    let cost: f64 = r.get(0)?;
                    let prompt: i64 = r.get(1)?;
                    let cached: i64 = r.get(2)?;
                    let pct = if prompt > 0 {
                        (cached as f64 / prompt as f64) * 100.0
                    } else {
                        0.0
                    };
                    Ok((cost, pct))
                },
            )
        })
    }

    /// Aggregates live Dashboard analytics in bounded SQL queries (zero full-table memory loads).
    pub fn dashboard_db_metrics(&self, days: i64) -> Result<SqliteDashboardMetrics> {
        let window_days = days.clamp(7, 366);
        let cutoff = (Utc::now() - Duration::days(window_days))
            .format("%Y-%m-%dT00:00:00")
            .to_string();

        self.with_conn(|conn| {
            let mut by_day: BTreeMap<String, DailyActivityPoint> = BTreeMap::new();

            // 1. Daily session events (turns, tool calls, tokens, cost)
            {
                let mut stmt = conn.prepare(
                    "SELECT substr(created_at, 1, 10) AS day,
                            COUNT(*) AS ev_cnt,
                            SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END) AS turns,
                            SUM(CASE WHEN role = 'tool' THEN 1 ELSE 0 END) AS tools,
                            COALESCE(SUM(tokens_in + tokens_out), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM session_events
                     WHERE created_at >= ?1
                     GROUP BY day
                     ORDER BY day ASC",
                )?;
                let mut rows = stmt.query(params![&cutoff])?;
                while let Some(r) = rows.next()? {
                    let date: String = r.get(0)?;
                    if date.len() != 10 {
                        continue;
                    }
                    by_day.insert(
                        date.clone(),
                        DailyActivityPoint {
                            date,
                            events: r.get::<_, i64>(1)?.max(0) as u64,
                            turns: r.get::<_, i64>(2)?.max(0) as u64,
                            tool_calls: r.get::<_, i64>(3)?.max(0) as u64,
                            tokens: r.get::<_, i64>(4)?.max(0) as u64,
                            cache_read_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                            cost_usd: r.get::<_, f64>(6)?.max(0.0),
                        },
                    );
                }
            }

            // 2. Merge usage_ledger daily tokens/cache/cost (captures subagents/cron/compaction too)
            {
                let mut stmt = conn.prepare(
                    "SELECT substr(created_at, 1, 10) AS day,
                            COUNT(*) AS calls,
                            COALESCE(SUM(prompt_tokens + completion_tokens), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM usage_ledger
                     WHERE created_at >= ?1
                     GROUP BY day
                     ORDER BY day ASC",
                )?;
                let mut rows = stmt.query(params![&cutoff])?;
                while let Some(r) = rows.next()? {
                    let date: String = r.get(0)?;
                    if date.len() != 10 {
                        continue;
                    }
                    let calls = r.get::<_, i64>(1)?.max(0) as u64;
                    let toks = r.get::<_, i64>(2)?.max(0) as u64;
                    let cached = r.get::<_, i64>(3)?.max(0) as u64;
                    let cost = r.get::<_, f64>(4)?.max(0.0);
                    let entry = by_day
                        .entry(date.clone())
                        .or_insert_with(|| DailyActivityPoint {
                            date,
                            ..Default::default()
                        });
                    entry.tokens = entry.tokens.max(toks);
                    entry.cache_read_tokens = entry.cache_read_tokens.max(cached);
                    entry.cost_usd = entry.cost_usd.max(cost);
                    if entry.events == 0 {
                        entry.events = calls;
                    }
                }
            }

            // 3. Top tools from tool events via native SQLite json_extract (zero regex)
            let mut top_tools = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT COALESCE(json_extract(content_json, '$.tool_name'), 'tool') AS tname,
                            COUNT(*) AS cnt,
                            SUM(CASE WHEN COALESCE(json_extract(content_json, '$.is_error'), 0) != 0 THEN 1 ELSE 0 END) AS errs,
                            COALESCE(AVG(CAST(COALESCE(json_extract(content_json, '$.duration_ms'), 0) AS REAL)), 0.0) AS avg_ms
                     FROM session_events
                     WHERE role = 'tool'
                     GROUP BY tname
                     ORDER BY cnt DESC
                     LIMIT 12",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    top_tools.push(ToolUsageStat {
                        name: r.get(0)?,
                        calls: r.get::<_, i64>(1)?.max(0) as u64,
                        errors: r.get::<_, i64>(2)?.max(0) as u64,
                        avg_duration_ms: r.get::<_, f64>(3)?.max(0.0).round() as u64,
                    });
                }
            }

            // 4. Top models from usage_ledger
            let mut top_models = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT provider, model,
                            COUNT(*) AS calls,
                            COALESCE(SUM(prompt_tokens + completion_tokens), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM usage_ledger
                     GROUP BY provider, model
                     ORDER BY toks DESC, calls DESC
                     LIMIT 10",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    top_models.push(ModelUsageStat {
                        provider: r.get(0)?,
                        model: r.get(1)?,
                        calls: r.get::<_, i64>(2)?.max(0) as u64,
                        tokens: r.get::<_, i64>(3)?.max(0) as u64,
                        cache_read_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                        cost_usd: r.get::<_, f64>(5)?.max(0.0),
                    });
                }
            }

            // 5. Sessions by channel
            let mut channels = BTreeMap::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT channel, COUNT(*) FROM sessions GROUP BY channel ORDER BY COUNT(*) DESC",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    channels.insert(r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64);
                }
            }

            // 6. Aggregate totals & entity counts
            let count_query = |sql: &str| -> u64 {
                conn.query_row(sql, [], |r| r.get::<_, i64>(0))
                    .unwrap_or(0)
                    .max(0) as u64
            };
            let counts = DashboardCounts {
                sessions: count_query("SELECT COUNT(*) FROM sessions"),
                messages: count_query("SELECT COUNT(*) FROM session_events WHERE role IN ('user', 'assistant')"),
                tool_calls: count_query("SELECT COUNT(*) FROM session_events WHERE role = 'tool'"),
                memories: count_query("SELECT COUNT(*) FROM memory_entries"),
                skills_enabled: 0,
                skills_total: 0,
                cron_jobs: count_query("SELECT COUNT(*) FROM cron_jobs"),
                goal_loops: count_query("SELECT COUNT(*) FROM goal_loops"),
                background_jobs: count_query("SELECT COUNT(*) FROM background_jobs"),
                subagents: count_query("SELECT COUNT(*) FROM subagents"),
                checkpoints: count_query("SELECT COUNT(*) FROM workspace_checkpoints"),
                mcp_servers: 0,
                paired_peers: count_query("SELECT COUNT(*) FROM gateway_peers"),
            };

            let (ledger_tokens, ledger_cached): (u64, u64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0), COALESCE(SUM(cache_read_tokens), 0) FROM usage_ledger",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
                )
                .unwrap_or((0, 0));
            let (ev_tokens, ev_cached): (u64, u64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(tokens_in + tokens_out), 0), COALESCE(SUM(cache_read_tokens), 0) FROM session_events",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
                )
                .unwrap_or((0, 0));

            // 7. Compute consecutive active day streak ending today or yesterday
            let active_days = by_day.values().filter(|d| d.events > 0 || d.tokens > 0).count() as u32;
            let today = Utc::now().date_naive();
            let mut streak = 0u32;
            let mut cursor = today;
            let today_key = today.format("%Y-%m-%d").to_string();
            if !by_day.get(&today_key).map(|d| d.events > 0 || d.tokens > 0).unwrap_or(false) {
                cursor = today - Duration::days(1);
            }
            for _ in 0..366 {
                let key = cursor.format("%Y-%m-%d").to_string();
                if by_day.get(&key).map(|d| d.events > 0 || d.tokens > 0).unwrap_or(false) {
                    streak += 1;
                    cursor -= Duration::days(1);
                } else {
                    break;
                }
            }

            Ok(SqliteDashboardMetrics {
                counts,
                daily_activity: by_day.into_values().collect(),
                top_tools,
                top_models,
                channels,
                streak_days: streak,
                active_days,
                total_tokens: ledger_tokens.max(ev_tokens),
                total_cache_read_tokens: ledger_cached.max(ev_cached),
            })
        })
    }

    // ─── KV State ──────────────────────────────────────────────────────────────

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row("SELECT v FROM kv_state WHERE k = ?1", params![key], |r| r.get(0))
                .optional()
        })
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO kv_state (k, v, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
            Ok(())
        })
    }

    pub fn delete_kv(&self, key: &str) -> Result<bool> {
        self.with_conn(|conn| {
            let rows = conn.execute("DELETE FROM kv_state WHERE k = ?1", params![key])?;
            Ok(rows > 0)
        })
    }

    /// Returns the stored byte/character length of a `kv_state` value without allocating it in Rust.
    pub fn kv_value_len(&self, key: &str) -> Result<Option<usize>> {
        self.with_conn(|conn| {
            let len: Option<i64> = conn
                .query_row(
                    "SELECT length(v) FROM kv_state WHERE k = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(len.map(|n| n.max(0) as usize))
        })
    }

    // ─── Categorized Persistent Debug Logs (Indexed Ring Table) ────────────────

    pub fn insert_log(&self, entry: &LogEntry) -> Result<()> {
        let id = new_id();
        let payload_str = entry
            .payload
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let should_prune =
            (self.log_inserts.fetch_add(1, Ordering::Relaxed) + 1).is_multiple_of(LOG_SQLITE_PRUNE_INTERVAL);

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO logs (id, ts, level, category, target, session_id, turn_id, call_id, message, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    entry.timestamp.to_rfc3339(),
                    entry.level,
                    entry.category,
                    entry.target,
                    entry.session_id,
                    entry.turn_id,
                    entry.call_id,
                    entry.message,
                    payload_str,
                ],
            )?;
            if should_prune {
                conn.execute(
                    "DELETE FROM logs WHERE id NOT IN (SELECT id FROM logs ORDER BY ts DESC, id DESC LIMIT ?1)",
                    params![LOG_SQLITE_MAX_ROWS as i64],
                )?;
            }
            Ok(())
        })
    }

    pub fn query_logs(&self, filter: &LogFilter) -> Result<Vec<LogEntry>> {
        let limit = filter.limit.unwrap_or(120).clamp(1, 1000) as i64;
        let category = filter
            .category
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let channel = filter
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let level = filter
            .level
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .map(str::to_ascii_uppercase);
        let session_id = filter
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let search_like = filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", q.to_ascii_lowercase()));

        let mut rows = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT ts, level, category, target, session_id, turn_id, call_id, message, payload_json
                 FROM logs
                 WHERE (?1 IS NULL OR category = ?1)
                   AND (?2 IS NULL OR target = ?2)
                   AND (?3 IS NULL OR level = ?3)
                   AND (?4 IS NULL OR session_id = ?4)
                   AND (?5 IS NULL OR LOWER(message) LIKE ?5 OR LOWER(COALESCE(payload_json, '')) LIKE ?5)
                 ORDER BY ts DESC, id DESC
                 LIMIT ?6",
            )?;
            let mapped = stmt.query_map(
                params![category, channel, level, session_id, search_like, limit],
                row_to_log_entry,
            )?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.reverse();
        Ok(rows)
    }

    pub fn log_category_counts(&self) -> Result<BTreeMap<String, u64>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT category, COUNT(*) FROM logs GROUP BY category ORDER BY category ASC")?;
            let rows = stmt.query_map([], |r| {
                let cat: String = r.get(0)?;
                let cnt: i64 = r.get(1)?;
                Ok((cat, cnt.max(0) as u64))
            })?;
            let mut map = BTreeMap::new();
            for item in rows {
                let (k, v) = item?;
                map.insert(k, v);
            }
            Ok(map)
        })
    }

}
