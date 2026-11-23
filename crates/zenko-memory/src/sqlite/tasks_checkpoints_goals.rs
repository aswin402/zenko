use super::*;

impl SqliteStore {
    // ─── Phase 6: Durable SQLite Session Task DAG ──────────────────────────────

    pub fn upsert_session_task(&self, task: &SessionTask) -> Result<()> {
        let deps_json = serde_json::to_string(&task.depends_on)?;
        self.with_conn(|conn| {
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_tasks WHERE session_id = ?1 AND id != ?2",
                params![task.session_id, task.id],
                |r| r.get(0),
            )?;
            if existing as usize >= SESSION_TASK_MAX_ITEMS {
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                    std::io::Error::other(format!(
                        "session task DAG limit reached (max {SESSION_TASK_MAX_ITEMS})"
                    )),
                )));
            }
            conn.execute(
                "INSERT OR REPLACE INTO session_tasks
                 (id, session_id, branch_event_id, title, description, active_label, status,
                  depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                  priority, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    task.id,
                    task.session_id,
                    task.branch_event_id,
                    task.title,
                    task.description,
                    task.active_label,
                    task.status.as_str(),
                    deps_json,
                    task.owner_agent,
                    task.verification_cmd,
                    task.verification_evidence,
                    task.notes,
                    task.priority,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn replace_session_tasks(&self, session_id: &str, tasks: &[SessionTask]) -> Result<()> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1",
                params![session_id],
            )?;
            for task in tasks.iter().take(SESSION_TASK_MAX_ITEMS) {
                let deps_json = serde_json::to_string(&task.depends_on).unwrap_or_else(|_| "[]".into());
                tx.execute(
                    "INSERT INTO session_tasks
                     (id, session_id, branch_event_id, title, description, active_label, status,
                      depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                      priority, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        task.id,
                        session_id,
                        task.branch_event_id,
                        task.title,
                        task.description,
                        task.active_label,
                        task.status.as_str(),
                        deps_json,
                        task.owner_agent,
                        task.verification_cmd,
                        task.verification_evidence,
                        task.notes,
                        task.priority,
                        task.created_at.to_rfc3339(),
                        task.updated_at.to_rfc3339(),
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_session_task(&self, session_id: &str, task_id: &str) -> Result<Option<SessionTask>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, branch_event_id, title, description, active_label, status,
                        depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                        priority, created_at, updated_at
                 FROM session_tasks
                 WHERE session_id = ?1 AND id = ?2",
                params![session_id, task_id],
                row_to_session_task,
            )
            .optional()
        })
    }

    pub fn list_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, branch_event_id, title, description, active_label, status,
                        depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                        priority, created_at, updated_at
                 FROM session_tasks
                 WHERE session_id = ?1
                 ORDER BY priority DESC, created_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![session_id], row_to_session_task)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// ACID atomic task claim for multi-agent coordination: succeeds only when unclaimed or already
    /// owned by `owner_agent` and not in a terminal/blocked state.
    pub fn claim_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        owner_agent: &str,
    ) -> Result<Option<SessionTask>> {
        let now = Utc::now().to_rfc3339();
        let in_prog = SessionTaskStatus::InProgress.as_str();
        let pending = SessionTaskStatus::Pending.as_str();
        let updated = self.with_conn(|conn| {
            conn.execute(
                "UPDATE session_tasks
                 SET owner_agent = ?1, status = ?2, updated_at = ?3
                 WHERE session_id = ?4 AND id = ?5
                   AND status IN (?6, ?2)
                   AND (owner_agent IS NULL OR owner_agent = '' OR owner_agent = ?1)",
                params![owner_agent, in_prog, now, session_id, task_id, pending],
            )
        })?;
        match updated > 0 {
            true => self.get_session_task(session_id, task_id),
            false => Ok(None),
        }
    }

    pub fn delete_session_task(&self, session_id: &str, task_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1 AND id = ?2",
                params![session_id, task_id],
            )
        })?;
        Ok(n > 0)
    }

    pub fn clear_session_tasks(&self, session_id: &str) -> Result<usize> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1",
                params![session_id],
            )
        })
    }

    // ─── Phase 6: Workspace Checkpoints & Rewind Metadata ──────────────────────

    pub fn insert_checkpoint(&self, cp: &WorkspaceCheckpoint) -> Result<()> {
        let files_json = serde_json::to_string(&cp.changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO workspace_checkpoints
                 (id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref, post_ref,
                  trigger_tool, label, changed_files_json, is_safety_backup, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    cp.id,
                    cp.session_id,
                    cp.turn_id,
                    cp.event_id,
                    cp.workspace_dir,
                    cp.backend.as_str(),
                    cp.pre_ref,
                    cp.post_ref,
                    cp.trigger_tool,
                    cp.label,
                    files_json,
                    if cp.is_safety_backup { 1i64 } else { 0i64 },
                    cp.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_checkpoint_post_state(
        &self,
        checkpoint_id: &str,
        post_ref: &str,
        changed_files: &[String],
    ) -> Result<()> {
        let files_json = serde_json::to_string(changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE workspace_checkpoints
                 SET post_ref = ?1, changed_files_json = ?2
                 WHERE id = ?3",
                params![post_ref, files_json, checkpoint_id],
            )?;
            Ok(())
        })
    }

    pub fn get_checkpoint(&self, checkpoint_id: &str) -> Result<Option<WorkspaceCheckpoint>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE id = ?1",
                params![checkpoint_id],
                row_to_checkpoint,
            )
            .optional()
        })
    }

    pub fn latest_checkpoint_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<WorkspaceCheckpoint>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE workspace_dir = ?1 AND post_ref IS NOT NULL AND is_safety_backup = 0
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT 1",
                params![workspace_dir],
                row_to_checkpoint,
            )
            .optional()
        })
    }

    /// Returns recent finalized checkpoints (including safety backups) newest-first for exact
    /// per-file Zenko provenance & manual hand-edit detection across chained rewinds.
    pub fn recent_finalized_checkpoints_for_workspace(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cap = limit.clamp(1, 64) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE workspace_dir = ?1 AND post_ref IS NOT NULL
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![workspace_dir, cap], row_to_checkpoint)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Collects all content-addressed manifest IDs still referenced by surviving `content_blobs`
    /// checkpoints so pruning never deletes shared manifests and can safely mark-and-sweep blobs.
    pub fn list_live_blob_manifest_refs(&self) -> Result<std::collections::HashSet<String>> {
        let backend_blobs = CheckpointBackend::ContentBlobs.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT pre_ref, post_ref FROM workspace_checkpoints WHERE backend = ?1",
            )?;
            let mut live = std::collections::HashSet::new();
            let rows = stmt.query_map(params![backend_blobs], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            for row in rows.flatten() {
                if !row.0.is_empty() {
                    live.insert(row.0);
                }
                if let Some(post) = row.1.filter(|s| !s.is_empty()) {
                    live.insert(post);
                }
            }
            Ok(live)
        })
    }

    pub fn list_checkpoints(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cap = limit.clamp(1, 200) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE (?1 IS NULL OR session_id = ?1)
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, cap], row_to_checkpoint)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_checkpoint(&self, checkpoint_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM workspace_checkpoints WHERE id = ?1",
                params![checkpoint_id],
            )
        })?;
        Ok(n > 0)
    }

    /// Prunes excess checkpoints beyond `max_per_session` for a session and returns removed rows.
    pub fn prune_session_checkpoints(
        &self,
        session_id: &str,
        max_per_session: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let keep = max_per_session.max(1) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT -1 OFFSET ?2",
            )?;
            let stale = stmt
                .query_map(params![session_id, keep], row_to_checkpoint)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for cp in &stale {
                conn.execute(
                    "DELETE FROM workspace_checkpoints WHERE id = ?1",
                    params![cp.id],
                )?;
            }
            Ok(stale)
        })
    }

    /// Prunes checkpoints older than `retention_days` across all sessions and returns removed rows.
    pub fn prune_expired_checkpoints(
        &self,
        retention_days: u64,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cutoff = (Utc::now() - Duration::days(retention_days.max(1) as i64)).to_rfc3339();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE created_at < ?1",
            )?;
            let expired = stmt
                .query_map(params![cutoff], row_to_checkpoint)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for cp in &expired {
                conn.execute(
                    "DELETE FROM workspace_checkpoints WHERE id = ?1",
                    params![cp.id],
                )?;
            }
            Ok(expired)
        })
    }

    // ─── Phase 8: Persistent 24/7 Bounded Goal Loops & Export Queries ─────────

    pub fn upsert_goal_loop(&self, goal: &GoalLoopRecord) -> Result<()> {
        let cmd_json = serde_json::to_string(&goal.verify_cmd).unwrap_or_else(|_| "[]".into());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO goal_loops
                 (id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                  max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                  auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                  consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                  last_verify_output, next_run_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
                params![
                    goal.id,
                    goal.name,
                    goal.session_id,
                    goal.workspace_dir,
                    goal.goal_text,
                    cmd_json,
                    goal.interval_secs as i64,
                    goal.max_iterations as i64,
                    goal.max_budget_usd,
                    goal.max_consecutive_stalls as i64,
                    if goal.checkpoint_each_iter { 1i64 } else { 0i64 },
                    if goal.auto_rewind_on_stall { 1i64 } else { 0i64 },
                    goal.model,
                    goal.notify_channel,
                    goal.status.as_str(),
                    goal.iterations_done as i64,
                    goal.consecutive_stalls as i64,
                    goal.cost_spent_usd,
                    goal.last_verify_exit,
                    goal.last_verify_hash,
                    goal.last_verify_output,
                    goal.next_run_at.map(|t| t.to_rfc3339()),
                    goal.created_at.to_rfc3339(),
                    goal.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_goal_loop(&self, id: &str) -> Result<Option<GoalLoopRecord>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                        max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                        auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                        consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                        last_verify_output, next_run_at, created_at, updated_at
                 FROM goal_loops WHERE id = ?1",
                params![id],
                row_to_goal_loop,
            )
            .optional()
        })
    }

    pub fn list_goal_loops(&self, limit: usize) -> Result<Vec<GoalLoopRecord>> {
        let cap = limit.clamp(1, 200.max(GOAL_LOOP_LIST_DEFAULT_LIMIT)) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                        max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                        auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                        consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                        last_verify_output, next_run_at, created_at, updated_at
                 FROM goal_loops ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![cap], row_to_goal_loop)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_goal_loop(&self, id: &str) -> Result<bool> {
        self.with_conn(|conn| {
            let n = conn.execute("DELETE FROM goal_loops WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    pub fn insert_goal_loop_run(&self, run: &GoalLoopRun) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO goal_loop_runs
                 (id, goal_id, iteration, checkpoint_id, verify_exit_code, verify_output_excerpt,
                  verify_hash, tokens_used, cost_usd, duration_ms, outcome, reward, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    run.id,
                    run.goal_id,
                    run.iteration as i64,
                    run.checkpoint_id,
                    run.verify_exit_code,
                    run.verify_output_excerpt,
                    run.verify_hash,
                    run.tokens_used as i64,
                    run.cost_usd,
                    run.duration_ms as i64,
                    run.outcome,
                    run.reward,
                    run.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_goal_loop_runs(&self, goal_id: &str, limit: usize) -> Result<Vec<GoalLoopRun>> {
        let cap = limit.clamp(1, 200.max(GOAL_LOOP_RUNS_DEFAULT_LIMIT)) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, goal_id, iteration, checkpoint_id, verify_exit_code, verify_output_excerpt,
                        verify_hash, tokens_used, cost_usd, duration_ms, outcome, reward, created_at
                 FROM goal_loop_runs WHERE goal_id = ?1 ORDER BY iteration DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![goal_id, cap], row_to_goal_loop_run)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Returns the most recent verification outcome for `session_id` (joins `goal_loops` + `goal_loop_runs`).
    pub fn latest_goal_run_for_session(&self, session_id: &str) -> Result<Option<GoalLoopRun>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT r.id, r.goal_id, r.iteration, r.checkpoint_id, r.verify_exit_code,
                        r.verify_output_excerpt, r.verify_hash, r.tokens_used, r.cost_usd,
                        r.duration_ms, r.outcome, r.reward, r.created_at
                 FROM goal_loop_runs r
                 INNER JOIN goal_loops g ON g.id = r.goal_id
                 WHERE g.session_id = ?1
                 ORDER BY r.created_at DESC
                 LIMIT 1",
                params![session_id],
                row_to_goal_loop_run,
            )
            .optional()
        })
    }

    /// Lists sessions filtered by optional `since` RFC3339 timestamp and `channel` for batch export.
    pub fn list_sessions_for_export(
        &self,
        since: Option<DateTime<Utc>>,
        channel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Session>> {
        let cap = limit.clamp(1, EXPORT_MAX_BATCH_LIMIT) as i64;
        let since_iso = since.map(|dt| dt.to_rfc3339());
        let chan = channel
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id,
                        s.model_override, s.permission_mode, s.status, s.head_event_id,
                        s.frozen_system_hash, s.workspace_dir, s.parent_session_id, s.agent_role,
                        s.agent_depth, s.worktree_path, s.worktree_branch, s.total_tokens,
                        s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents a WHERE a.parent_session_id = s.id)
                 FROM sessions s
                 WHERE (?1 IS NULL OR s.updated_at >= ?1)
                   AND (?2 IS NULL OR s.channel = ?2)
                 ORDER BY s.updated_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![since_iso, chan, cap], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }
}
