use super::*;

impl SqliteStore {
    // ─── Sessions ──────────────────────────────────────────────────────────────

    pub fn create_session(
        &self,
        title: &str,
        channel: &str,
        external_chat_id: Option<&str>,
        external_thread_id: Option<&str>,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: new_id(),
            title: title.to_string(),
            channel: channel.to_string(),
            external_chat_id: external_chat_id.map(String::from),
            external_thread_id: external_thread_id.map(String::from),
            model_override: None,
            permission_mode,
            status: SessionStatus::Idle,
            head_event_id: None,
            frozen_system_hash: None,
            workspace_dir: None,
            parent_session_id: None,
            agent_role: None,
            agent_depth: 0,
            worktree_path: None,
            worktree_branch: None,
            subagent_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            created_at: now,
            updated_at: now,
        };
        self.insert_session_row(&session)?;
        Ok(session)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_child_session(
        &self,
        parent_session_id: &str,
        role: &str,
        depth: u8,
        title: &str,
        workspace_dir: &str,
        worktree_path: Option<&str>,
        worktree_branch: Option<&str>,
        model_override: Option<&str>,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: new_id(),
            title: title.to_string(),
            channel: "subagent".to_string(),
            external_chat_id: None,
            external_thread_id: None,
            model_override: model_override.map(String::from),
            permission_mode,
            status: SessionStatus::Running,
            head_event_id: None,
            frozen_system_hash: None,
            workspace_dir: Some(workspace_dir.to_string()),
            parent_session_id: Some(parent_session_id.to_string()),
            agent_role: Some(role.to_string()),
            agent_depth: depth,
            worktree_path: worktree_path.map(String::from),
            worktree_branch: worktree_branch.map(String::from),
            subagent_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            created_at: now,
            updated_at: now,
        };
        self.insert_session_row(&session)?;
        Ok(session)
    }

    fn insert_session_row(&self, session: &Session) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, channel, external_chat_id, external_thread_id, model_override, permission_mode, status, head_event_id, frozen_system_hash, workspace_dir, parent_session_id, agent_role, agent_depth, worktree_path, worktree_branch, total_tokens, total_cost_usd, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    session.id,
                    session.title,
                    session.channel,
                    session.external_chat_id,
                    session.external_thread_id,
                    session.model_override,
                    session.permission_mode.as_str(),
                    session.status.as_str(),
                    session.head_event_id,
                    session.frozen_system_hash,
                    session.workspace_dir,
                    session.parent_session_id,
                    session.agent_role,
                    session.agent_depth as i64,
                    session.worktree_path,
                    session.worktree_branch,
                    session.total_tokens as i64,
                    session.total_cost_usd,
                    session.created_at.to_rfc3339(),
                    session.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn set_session_workspace(&self, session_id: &str, workspace_dir: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET workspace_dir = ?1 WHERE id = ?2",
                params![workspace_dir, session_id],
            )?;
            Ok(())
        })
    }

    pub fn set_session_worktree(
        &self,
        session_id: &str,
        workspace_dir: &str,
        worktree_path: &str,
        worktree_branch: &str,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET workspace_dir = ?1, worktree_path = ?2, worktree_branch = ?3 WHERE id = ?4",
                params![workspace_dir, worktree_path, worktree_branch, session_id],
            )?;
            Ok(())
        })
    }

    pub fn find_or_create_channel_session(
        &self,
        channel: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        title: &str,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let existing = self.with_conn(|conn| {
            conn.query_row(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s
                 WHERE s.channel = ?1 AND s.external_chat_id = ?2 AND COALESCE(s.external_thread_id, '') = COALESCE(?3, '')
                 ORDER BY s.updated_at DESC LIMIT 1",
                params![channel, chat_id, thread_id],
                row_to_session,
            )
            .optional()
        })?;
        if let Some(s) = existing {
            return Ok(s);
        }
        self.create_session(title, channel, Some(chat_id), thread_id, permission_mode)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s WHERE s.id = ?1",
                params![id],
                row_to_session,
            )
            .optional()
        })
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s ORDER BY s.updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Lists top-level user/channel root sessions (`parent_session_id IS NULL`) with exact
    /// SQL-aggregated `subagent_count` so high subagent volume never crowds out root chats.
    pub fn list_root_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s
                 WHERE s.parent_session_id IS NULL
                 ORDER BY s.updated_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute("DELETE FROM session_events_fts WHERE session_id = ?1", params![id])?;
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
        })?;
        Ok(n > 0)
    }

    pub fn clear_session_events(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute("DELETE FROM session_events_fts WHERE session_id = ?1", params![id])?;
            conn.execute("DELETE FROM session_events WHERE session_id = ?1", params![id])?;
            conn.execute("DELETE FROM session_vars WHERE session_id = ?1", params![id])?;
            conn.execute(
                "UPDATE sessions SET head_event_id = NULL, total_tokens = 0, total_cost_usd = 0.0, status = 'idle', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    pub fn update_session_status(&self, id: &str, status: SessionStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, id],
            )?;
            Ok(())
        })
    }

    pub fn update_session_meta(
        &self,
        id: &str,
        title: Option<&str>,
        model_override: Option<Option<&str>>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            if let Some(t) = title {
                conn.execute(
                    "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
                    params![t, now, id],
                )?;
            }
            if let Some(m) = model_override {
                conn.execute(
                    "UPDATE sessions SET model_override = ?1, updated_at = ?2 WHERE id = ?3",
                    params![m, now, id],
                )?;
            }
            if let Some(p) = permission_mode {
                conn.execute(
                    "UPDATE sessions SET permission_mode = ?1, updated_at = ?2 WHERE id = ?3",
                    params![p.as_str(), now, id],
                )?;
            }
            Ok(())
        })
    }

    pub fn set_session_head(&self, session_id: &str, event_id: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET head_event_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![event_id, now, session_id],
            )?;
            Ok(())
        })
    }

    pub fn set_session_frozen_hash(&self, session_id: &str, hash: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET frozen_system_hash = ?1 WHERE id = ?2",
                params![hash, session_id],
            )?;
            Ok(())
        })
    }

    pub fn recover_interrupted_sessions(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            // Expire orphaned pending approvals whose in-memory oneshot channels were lost on restart.
            conn.execute(
                "UPDATE approvals SET status = 'denied', resolved_by = 'system:restart', resolved_at = ?1 WHERE status = 'pending'",
                params![now],
            )?;
            conn.execute(
                "UPDATE sessions SET status = 'idle', updated_at = ?1 WHERE status IN ('running', 'waiting_approval')",
                params![now],
            )
        })
    }

    // ─── Event Tree (Durable Trajectory + Branching) ───────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn append_event(
        &self,
        session_id: &str,
        parent_id: Option<&str>,
        role: EventRole,
        kind: EventKind,
        content_json: serde_json::Value,
        usage: &UsageStats,
    ) -> Result<SessionEvent> {
        let now = Utc::now();
        let id = new_id();
        let content_str = serde_json::to_string(&content_json)?;

        let branch_index: i64 = self.with_conn(|conn| {
            let sibling_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND COALESCE(parent_id, '') = COALESCE(?2, '')",
                params![session_id, parent_id],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO session_events (id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    session_id,
                    parent_id,
                    sibling_count,
                    role.as_str(),
                    kind.as_str(),
                    content_str,
                    usage.prompt_tokens as i64,
                    usage.completion_tokens as i64,
                    usage.cache_read_tokens as i64,
                    usage.cost_usd,
                    now.to_rfc3339(),
                ],
            )?;
            let searchable = extract_searchable_event_text(role, kind, &content_json);
            if !searchable.is_empty() {
                conn.execute(
                    "INSERT INTO session_events_fts (event_id, session_id, role, body) VALUES (?1, ?2, ?3, ?4)",
                    params![id, session_id, role.as_str(), searchable],
                )?;
            }
            let added_tokens = (usage.prompt_tokens + usage.completion_tokens) as i64;
            conn.execute(
                "UPDATE sessions
                 SET head_event_id = ?1,
                     total_tokens = total_tokens + ?2,
                     total_cost_usd = total_cost_usd + ?3,
                     updated_at = ?4
                 WHERE id = ?5",
                params![id, added_tokens, usage.cost_usd, now.to_rfc3339(), session_id],
            )?;
            Ok(sibling_count)
        })?;

        Ok(SessionEvent {
            id,
            session_id: session_id.to_string(),
            parent_id: parent_id.map(String::from),
            branch_index,
            role,
            kind,
            content_json,
            tokens_in: usage.prompt_tokens,
            tokens_out: usage.completion_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cost_usd: usage.cost_usd,
            created_at: now,
        })
    }

    pub fn get_active_branch(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        let session = match self.get_session(session_id)? {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let all = self.list_all_session_events(session_id)?;
        let Some(head_id) = session.head_event_id else {
            return Ok(Vec::new());
        };
        let mut by_id = std::collections::HashMap::with_capacity(all.len());
        for ev in all {
            by_id.insert(ev.id.clone(), ev);
        }
        let mut chain = Vec::new();
        let mut cur = Some(head_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            if let Some(ev) = by_id.remove(&id) {
                cur = ev.parent_id.clone();
                chain.push(ev);
            } else {
                break;
            }
        }
        chain.reverse();
        Ok(chain)
    }

    pub fn list_all_session_events(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at
                 FROM session_events WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(params![session_id], row_to_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn list_sibling_branches(
        &self,
        session_id: &str,
        parent_id: Option<&str>,
    ) -> Result<Vec<SessionEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at
                 FROM session_events
                 WHERE session_id = ?1 AND COALESCE(parent_id, '') = COALESCE(?2, '')
                 ORDER BY branch_index ASC",
            )?;
            let rows = stmt.query_map(params![session_id, parent_id], row_to_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Finds the deepest leaf descending from `event_id` (latest branch tip).
    pub fn find_latest_leaf(&self, session_id: &str, event_id: &str) -> Result<String> {
        let events = self.list_all_session_events(session_id)?;
        let mut cur = event_id.to_string();
        loop {
            let next = events
                .iter()
                .filter(|e| e.parent_id.as_deref() == Some(cur.as_str()))
                .max_by_key(|e| e.created_at)
                .map(|e| e.id.clone());
            match next {
                Some(n) => cur = n,
                None => return Ok(cur),
            }
        }
    }

    // ─── Compaction-Surviving Session Scratchpad (`session_vars`, Prime Agent pattern) ──

    pub fn upsert_session_var_with_secrets(
        &self,
        session_id: &str,
        raw_key: &str,
        raw_value: &str,
        raw_summary: Option<&str>,
        known_secrets: &[String],
    ) -> Result<SessionVarEntry> {
        let key: String = raw_key
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            .take(SESSION_VAR_MAX_KEY_CHARS)
            .collect();
        if key.is_empty() {
            return Err(ZenkoError::Tool(
                "session variable `key` must contain ASCII letters, digits, `_`, `-`, or `.`"
                    .into(),
            ));
        }
        let clean_value = sanitize_memory_text_with_secrets(raw_value, known_secrets);
        let bounded_value = match clean_value.len() <= SESSION_VAR_MAX_VALUE_BYTES {
            true => clean_value,
            false => {
                let cut = clean_value.floor_char_boundary(SESSION_VAR_MAX_VALUE_BYTES);
                clean_value[..cut].to_string()
            }
        };
        let summary_src = raw_summary
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| sanitize_memory_text_with_secrets(s, known_secrets))
            .unwrap_or_else(|| {
                compact_preview(&bounded_value.replace('\n', " "), SESSION_VAR_PREVIEW_CHARS)
            });
        let byte_len = bounded_value.len();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO session_vars (session_id, var_key, summary, value, byte_len, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    session_id,
                    key,
                    summary_src,
                    bounded_value,
                    byte_len as i64,
                    now_str,
                ],
            )?;
            // Enforce strict per-session cap (evicting oldest updated vars first)
            conn.execute(
                "DELETE FROM session_vars
                 WHERE session_id = ?1
                   AND var_key NOT IN (
                       SELECT var_key FROM session_vars
                       WHERE session_id = ?1
                       ORDER BY updated_at DESC
                       LIMIT ?2
                   )",
                params![session_id, SESSION_VAR_MAX_ITEMS as i64],
            )?;
            Ok(())
        })?;

        Ok(SessionVarEntry {
            session_id: session_id.to_string(),
            key,
            summary: summary_src,
            value: bounded_value,
            byte_len,
            updated_at: now,
        })
    }

    pub fn get_session_var(&self, session_id: &str, key: &str) -> Result<Option<SessionVarEntry>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id, var_key, summary, value, byte_len, updated_at
                 FROM session_vars WHERE session_id = ?1 AND var_key = ?2",
                params![session_id, key.trim()],
                |r| {
                    Ok(SessionVarEntry {
                        session_id: r.get(0)?,
                        key: r.get(1)?,
                        summary: r.get(2)?,
                        value: r.get(3)?,
                        byte_len: r.get::<_, i64>(4)?.max(0) as usize,
                        updated_at: parse_dt(&r.get::<_, String>(5)?),
                    })
                },
            )
            .optional()
        })
    }

    pub fn list_session_vars(&self, session_id: &str) -> Result<Vec<SessionVarEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, var_key, summary, value, byte_len, updated_at
                 FROM session_vars
                 WHERE session_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, SESSION_VAR_MAX_ITEMS as i64], |r| {
                Ok(SessionVarEntry {
                    session_id: r.get(0)?,
                    key: r.get(1)?,
                    summary: r.get(2)?,
                    value: r.get(3)?,
                    byte_len: r.get::<_, i64>(4)?.max(0) as usize,
                    updated_at: parse_dt(&r.get::<_, String>(5)?),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_session_var(&self, session_id: &str, key: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM session_vars WHERE session_id = ?1 AND var_key = ?2",
                params![session_id, key.trim()],
            )
        })?;
        Ok(n > 0)
    }

}
