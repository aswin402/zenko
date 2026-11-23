use super::*;

impl SqliteStore {
    // ─── Async Background Jobs ─────────────────────────────────────────────────

    pub fn upsert_job(&self, job: &BackgroundJobRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO background_jobs
                 (id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    job.id,
                    job.session_id,
                    job.kind.as_str(),
                    job.label,
                    job.command,
                    job.workspace_dir,
                    job.pid.map(|p| p as i64),
                    job.status.as_str(),
                    job.exit_code,
                    job.log_path,
                    job.bytes_written as i64,
                    job.started_at.to_rfc3339(),
                    job.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_job(&self, id: &str) -> Result<Option<BackgroundJobRecord>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs WHERE id = ?1",
                params![id],
                row_to_job,
            )
            .optional()
        })
    }

    pub fn list_jobs(
        &self,
        session_id: Option<&str>,
        kind: Option<TaskKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundJobRecord>> {
        let cap = limit.clamp(1, JOB_MAX_HISTORY) as i64;
        let kind_str = kind.map(TaskKind::as_str);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY started_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![session_id, kind_str, cap], row_to_job)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn update_job_meta(
        &self,
        id: &str,
        label: Option<String>,
        kind: Option<TaskKind>,
    ) -> Result<Option<BackgroundJobRecord>> {
        let Some(mut rec) = self.get_job(id)? else {
            return Ok(None);
        };
        if let Some(lbl) = label {
            let trimmed = lbl.trim();
            rec.label = match trimmed.is_empty() {
                true => None,
                false => Some(trimmed.to_string()),
            };
        }
        if let Some(k) = kind {
            rec.kind = k;
        }
        self.upsert_job(&rec)?;
        Ok(Some(rec))
    }

    pub fn delete_job(&self, id: &str) -> Result<Option<BackgroundJobRecord>> {
        let existing = self.get_job(id)?;
        if existing.is_some() {
            self.with_conn(|conn| {
                conn.execute("DELETE FROM background_jobs WHERE id = ?1", params![id])
            })?;
        }
        Ok(existing)
    }

    pub fn prune_finished_jobs(&self, kind: Option<TaskKind>) -> Result<Vec<BackgroundJobRecord>> {
        let running_str = BackgroundJobStatus::Running.as_str();
        let kind_str = kind.map(TaskKind::as_str);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs
                 WHERE status != ?1 AND (?2 IS NULL OR kind = ?2)",
            )?;
            let removed = stmt
                .query_map(params![running_str, kind_str], row_to_job)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            conn.execute(
                "DELETE FROM background_jobs WHERE status != ?1 AND (?2 IS NULL OR kind = ?2)",
                params![running_str, kind_str],
            )?;
            Ok(removed)
        })
    }

    pub fn recover_interrupted_jobs(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let interrupted_str = BackgroundJobStatus::Interrupted.as_str();
        let running_str = BackgroundJobStatus::Running.as_str();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE background_jobs
                 SET status = ?1, finished_at = COALESCE(finished_at, ?2)
                 WHERE status = ?3",
                params![interrupted_str, now, running_str],
            )
        })
    }

    // ─── Durable SQLite Follow-Up Prompt Queue ─────────────────────────────────

    pub fn enqueue_followup(&self, session_id: &str, text: &str) -> Result<(QueuedPrompt, usize)> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let item = QueuedPrompt {
            id: new_id(),
            session_id: session_id.to_string(),
            text: text.to_string(),
            status: zenko_core::QueuedPromptStatus::Queued,
            created_at: Utc::now(),
        };
        let inner = self.with_conn(|conn| {
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_queue WHERE session_id = ?1 AND status = ?2",
                params![session_id, queued_str],
                |r| r.get(0),
            )?;
            if existing as usize >= SESSION_QUEUE_MAX_ITEMS {
                return Ok(Err(ZenkoError::PolicyDenied(format!(
                    "Session follow-up queue is full (max {SESSION_QUEUE_MAX_ITEMS})."
                ))));
            }
            conn.execute(
                "INSERT INTO session_queue (id, session_id, text, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    item.id,
                    item.session_id,
                    item.text,
                    item.status.as_str(),
                    item.created_at.to_rfc3339(),
                ],
            )?;
            Ok(Ok((existing as usize) + 1))
        })?;
        let new_count = inner?;
        Ok((item, new_count))
    }

    pub fn list_queued_followups(&self, session_id: &str) -> Result<Vec<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, text, status, created_at
                 FROM session_queue
                 WHERE session_id = ?1 AND status = ?2
                 ORDER BY created_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![session_id, queued_str], row_to_queued_prompt)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn pop_next_followup(&self, session_id: &str) -> Result<Option<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let dispatched_str = zenko_core::QueuedPromptStatus::Dispatched.as_str();
        self.with_conn(|conn| {
            let next: Option<QueuedPrompt> = conn
                .query_row(
                    "SELECT id, session_id, text, status, created_at
                     FROM session_queue
                     WHERE session_id = ?1 AND status = ?2
                     ORDER BY created_at ASC, id ASC
                     LIMIT 1",
                    params![session_id, queued_str],
                    row_to_queued_prompt,
                )
                .optional()?;
            if let Some(ref item) = next {
                conn.execute(
                    "UPDATE session_queue SET status = ?1 WHERE id = ?2",
                    params![dispatched_str, item.id],
                )?;
            }
            Ok(next)
        })
    }

    pub fn cancel_queued_followup(&self, session_id: &str, queue_id: &str) -> Result<bool> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let cancelled_str = zenko_core::QueuedPromptStatus::Cancelled.as_str();
        let n = self.with_conn(|conn| {
            conn.execute(
                "UPDATE session_queue SET status = ?1
                 WHERE id = ?2 AND session_id = ?3 AND status = ?4",
                params![cancelled_str, queue_id, session_id, queued_str],
            )
        })?;
        Ok(n > 0)
    }

    pub fn pop_queued_followup_by_id(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Result<Option<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let dispatched_str = zenko_core::QueuedPromptStatus::Dispatched.as_str();
        self.with_conn(|conn| {
            let found: Option<QueuedPrompt> = conn
                .query_row(
                    "SELECT id, session_id, text, status, created_at
                     FROM session_queue
                     WHERE id = ?1 AND session_id = ?2 AND status = ?3",
                    params![queue_id, session_id, queued_str],
                    row_to_queued_prompt,
                )
                .optional()?;
            if let Some(mut item) = found {
                let updated = conn.execute(
                    "UPDATE session_queue SET status = ?1
                     WHERE id = ?2 AND session_id = ?3 AND status = ?4",
                    params![dispatched_str, queue_id, session_id, queued_str],
                )?;
                if updated > 0 {
                    item.status = zenko_core::QueuedPromptStatus::Dispatched;
                    return Ok(Some(item));
                }
            }
            Ok(None)
        })
    }

    // ─── Phase 5: Async Subagents & Inter-Agent Mailbox ────────────────────────

    pub fn upsert_subagent(&self, rec: &SubagentRecord) -> Result<()> {
        let output_schema_json = rec
            .output_schema
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let structured_json = rec
            .structured_output
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let files_json = serde_json::to_string(&rec.changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO subagents
                 (session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                  worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                  output_schema_json, structured_output_json, diff_stat, changed_files_json,
                  tokens_used, cost_usd, resume_count, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    rec.id,
                    rec.parent_session_id,
                    rec.role,
                    rec.depth as i64,
                    rec.goal,
                    rec.status.as_str(),
                    rec.workspace_mode.as_str(),
                    rec.workspace_dir,
                    rec.worktree_branch,
                    rec.worktree_base_commit,
                    match rec.worktree_dirty_baseline {
                        true => 1i64,
                        false => 0i64,
                    },
                    rec.model,
                    rec.summary,
                    output_schema_json,
                    structured_json,
                    rec.diff_stat,
                    files_json,
                    rec.tokens_used as i64,
                    rec.cost_usd,
                    rec.resume_count as i64,
                    rec.started_at.to_rfc3339(),
                    rec.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_subagent(&self, session_id: &str) -> Result<Option<SubagentRecord>> {
        let mut rec = self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                        worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                        output_schema_json, structured_output_json, diff_stat, changed_files_json,
                        tokens_used, cost_usd, resume_count, started_at, finished_at
                 FROM subagents WHERE session_id = ?1",
                params![session_id],
                row_to_subagent,
            )
            .optional()
        })?;
        if let Some(ref mut single) = rec {
            self.hydrate_subagent_tree_rollups(std::slice::from_mut(single))?;
        }
        Ok(rec)
    }

    pub fn list_subagents(
        &self,
        parent_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SubagentRecord>> {
        let cap = limit.clamp(1, SUBAGENT_LIST_DEFAULT_LIMIT.max(100)) as i64;
        let mut rows = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                        worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                        output_schema_json, structured_output_json, diff_stat, changed_files_json,
                        tokens_used, cost_usd, resume_count, started_at, finished_at
                 FROM subagents
                 WHERE (?1 IS NULL OR parent_session_id = ?1)
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![parent_session_id, cap], row_to_subagent)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        if !rows.is_empty() {
            self.hydrate_subagent_tree_rollups(&mut rows)?;
        }
        Ok(rows)
    }

    /// Computes Prime-Agent hierarchical subtree rollups (`tree_tokens`, `tree_cost_usd`,
    /// `descendant_count`) across all subagent rows in a single flat query + BFS (zero N+1 SQL).
    fn hydrate_subagent_tree_rollups(&self, records: &mut [SubagentRecord]) -> Result<()> {
        let edges: Vec<(String, String, u64, f64)> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, parent_session_id, tokens_used, cost_usd FROM subagents",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?.max(0) as u64,
                    r.get::<_, f64>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        let mut children_by_parent: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut stats_by_id: HashMap<&str, (u64, f64)> = HashMap::with_capacity(edges.len());
        for (sid, pid, toks, cost) in &edges {
            children_by_parent
                .entry(pid.as_str())
                .or_default()
                .push(sid.as_str());
            stats_by_id.insert(sid.as_str(), (*toks, *cost));
        }

        for rec in records.iter_mut() {
            let mut total_tok = rec.tokens_used;
            let mut total_cost = rec.cost_usd;
            let mut desc_count = 0usize;
            let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
            let mut queue: std::collections::VecDeque<&str> = std::collections::VecDeque::new();
            visited.insert(rec.id.as_str());
            queue.push_back(rec.id.as_str());
            while let Some(cur) = queue.pop_front() {
                if let Some(kids) = children_by_parent.get(cur) {
                    for &kid in kids {
                        if visited.insert(kid) {
                            desc_count += 1;
                            if let Some(&(kt, kc)) = stats_by_id.get(kid) {
                                total_tok = total_tok.saturating_add(kt);
                                total_cost += kc;
                            }
                            queue.push_back(kid);
                        }
                    }
                }
            }
            rec.tree_tokens = total_tok;
            rec.tree_cost_usd = total_cost;
            rec.descendant_count = desc_count;
        }
        Ok(())
    }

    /// Returns directory names under `<data_dir>/worktrees/` that belong to live or reviewable
    /// subagent records (excludes typed `SubagentStatus::Interrupted` crash orphans — zero string sniffing).
    pub fn list_preserved_worktree_ids(&self) -> Result<std::collections::HashSet<String>> {
        let interrupted_str = SubagentStatus::Interrupted.as_str();
        let wt_mode = SubagentWorkspaceMode::Worktree.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id FROM subagents WHERE workspace_mode = ?1 AND status != ?2",
            )?;
            let rows = stmt.query_map(params![wt_mode, interrupted_str], |r| r.get::<_, String>(0))?;
            Ok(rows.flatten().collect())
        })
    }

    /// Breadth-first traversal collecting all descendant subagent session IDs rooted under `root_session_id`.
    pub fn list_descendant_session_ids(&self, root_session_id: &str) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let mut out = Vec::new();
            let mut frontier = std::collections::VecDeque::new();
            frontier.push_back(root_session_id.to_string());
            let mut visited = std::collections::HashSet::new();
            visited.insert(root_session_id.to_string());
            let mut stmt = conn.prepare(
                "SELECT session_id FROM subagents WHERE parent_session_id = ?1",
            )?;
            while let Some(cur) = frontier.pop_front() {
                let children: Vec<String> = stmt
                    .query_map(params![cur], |r| r.get(0))?
                    .filter_map(std::result::Result::ok)
                    .collect();
                for child in children {
                    if visited.insert(child.clone()) {
                        out.push(child.clone());
                        frontier.push_back(child);
                    }
                }
            }
            Ok(out)
        })
    }

    pub fn recover_interrupted_subagents(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let interrupted_str = SubagentStatus::Interrupted.as_str();
        let running_str = SubagentStatus::Running.as_str();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE subagents
                 SET status = ?1,
                     summary = COALESCE(summary, 'Subagent interrupted by daemon restart.'),
                     finished_at = COALESCE(finished_at, ?2)
                 WHERE status = ?3",
                params![interrupted_str, now, running_str],
            )
        })
    }

    pub fn insert_agent_message(
        &self,
        from_session_id: &str,
        to_session_id: &str,
        body: &str,
    ) -> Result<AgentMailMessage> {
        let now = Utc::now();
        let msg = AgentMailMessage {
            id: new_id(),
            from_session_id: from_session_id.to_string(),
            to_session_id: to_session_id.to_string(),
            body: body.to_string(),
            read_at: None,
            created_at: now,
            delivery_action: None,
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_messages (id, from_session_id, to_session_id, body, read_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![
                    msg.id,
                    msg.from_session_id,
                    msg.to_session_id,
                    msg.body,
                    msg.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })?;
        Ok(msg)
    }

    /// Marks a specific mailbox message as read (used when directly resuming an idle child with that message).
    pub fn mark_agent_message_read(&self, id: &str) -> Result<()> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE agent_messages SET read_at = COALESCE(read_at, ?1) WHERE id = ?2",
                params![now_str, id],
            )?;
            Ok(())
        })
    }

    /// Atomically reads and marks unread mailbox messages delivered to `to_session_id`.
    pub fn drain_unread_agent_messages(
        &self,
        to_session_id: &str,
    ) -> Result<Vec<AgentMailMessage>> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cap = SUBAGENT_MAILBOX_MAX_MESSAGES as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, from_session_id, to_session_id, body, read_at, created_at
                 FROM agent_messages
                 WHERE to_session_id = ?1 AND read_at IS NULL
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?2",
            )?;
            let mut msgs = stmt
                .query_map(params![to_session_id, cap], row_to_agent_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for msg in &mut msgs {
                conn.execute(
                    "UPDATE agent_messages SET read_at = ?1 WHERE id = ?2",
                    params![now_str, msg.id],
                )?;
                msg.read_at = Some(now);
            }
            Ok(msgs)
        })
    }

    pub fn list_agent_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<AgentMailMessage>> {
        let cap = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, from_session_id, to_session_id, body, read_at, created_at
                 FROM agent_messages
                 WHERE from_session_id = ?1 OR to_session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let mut rows = stmt
                .query_map(params![session_id, cap], row_to_agent_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse();
            Ok(rows)
        })
    }

}
