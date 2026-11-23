use super::*;

impl AgentRuntime {
    // ─── Reactive Batched Idle-Session Wakeup Loop (Zero Busy-Polling) ─────────

    /// Event-driven daemon task that wakes idle sessions when their background jobs complete,
    /// coalescing burst completions within `JOB_WAKE_COALESCE_MS` into a single follow-up turn.
    pub async fn run_job_wakeup_loop(self: Arc<Self>, cancel: CancellationToken) {
        let Some(mut rx) = self.wake_rx.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        info!("reactive background-job wakeup loop started (zero-polling)");

        loop {
            let first_sid = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Some(sid) => sid,
                    None => break,
                }
            };

            let mut batch: HashSet<String> = HashSet::with_capacity(4);
            batch.insert(first_sid);
            let deadline =
                tokio::time::Instant::now() + Duration::from_millis(JOB_WAKE_COALESCE_MS);
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep_until(deadline) => break,
                    more = rx.recv() => match more {
                        Some(sid) => { batch.insert(sid); }
                        None => break,
                    }
                }
            }

            for sid in batch {
                if self.is_session_running(&sid) || !self.has_job_notices(&sid) {
                    continue;
                }
                if self.store.get_session(&sid).ok().flatten().is_none() {
                    let _ = self.drain_job_notices(&sid);
                    continue;
                }
                let rt = self.clone();
                tokio::spawn(async move {
                    if rt.is_session_running(&sid) {
                        return;
                    }
                    let notices = rt.drain_job_notices(&sid);
                    if notices.is_empty() {
                        return;
                    }
                    let mut shell_lines: Vec<&str> = Vec::new();
                    let mut agent_lines: Vec<&str> = Vec::new();
                    for n in &notices {
                        match n {
                            WakeupNotice::ShellJob(s) => shell_lines.push(s.as_str()),
                            WakeupNotice::Subagent { summary, .. } => agent_lines.push(summary.as_str()),
                        }
                    }
                    let wakeup_prompt = match (shell_lines.is_empty(), agent_lines.is_empty()) {
                        (false, true) => format!(
                            "[Background Job Update]\n{}\n\nReview the completed background job(s), call `job_read` if you need detailed output, and report concise findings or continue the task.",
                            shell_lines.join("\n")
                        ),
                        (true, false) => format!(
                            "[Subagent Completion Update]\n{}\n\nReview the completed subagent result(s), use `agent_worktree` (`diff`/`merge`/`discard`) for worktree edits or `agent_message` to resume if follow-up is needed, and synthesize concise findings.",
                            agent_lines.join("\n")
                        ),
                        _ => format!(
                            "[Async Completion Update]\nBackground Jobs:\n{}\n\nSubagents:\n{}\n\nReview all completed items (use `job_read` for shell jobs and `agent_worktree`/`agent_list` for subagents) and report concise findings or continue.",
                            shell_lines.join("\n"),
                            agent_lines.join("\n")
                        ),
                    };
                    let headless = !rt.is_telegram_connected();
                    if let Err(e) = rt
                        .run_turn_with_options(
                            &sid,
                            &wakeup_prompt,
                            TurnOptions {
                                headless_no_wait: headless,
                                send_mode: SendMode::Queue,
                                ..TurnOptions::default()
                            },
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "reactive job wakeup turn failed");
                    }
                });
            }
        }
    }

    /// Lists categorized creations inside `~/zenko_hub/` (projects, scripts, media, artifacts).
    pub fn list_hub_entries(
        &self,
        category_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HubEntry>> {
        self.config().list_hub_entries(category_filter, limit)
    }

    /// Executes a cron job immediately (shared by both `run_cron_scheduler` and manual Run-Now).
    /// Supports both LLM-driven (`Cron`/`Heartbeat`) and zero-token script-only (`Script`) modes.
    pub async fn dispatch_cron_job(self: &Arc<Self>, job: &CronJob) -> Result<Value> {
        let cfg = self.config();
        match job.kind {
            CronKind::Script => {
                let raw_script = job
                    .script_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool(format!(
                            "script cron job `{}` is missing `script_path`",
                            job.name
                        ))
                    })?;
                let scripts_dir = cfg.ensure_hub_layout()?.join(HUB_SUBDIR_SCRIPTS);
                let resolved =
                    zenko_tools::jail::resolve_jailed_path(&scripts_dir, raw_script, true)?;
                let cmd = build_script_command(&resolved);
                let task = self
                    .jobs
                    .spawn_shell(
                        "cron",
                        &cmd,
                        &scripts_dir,
                        TaskKind::Cron,
                        Some(format!("cron: {}", job.name)),
                        false,
                    )
                    .await?;
                let cron_entry = zenko_core::emit_hono_line(
                    zenko_core::LogDirection::Work,
                    zenko_core::LogChannel::Cron,
                    &format!("{} ({}) -> {}", job.name, job.cron_expr, raw_script),
                    Some(("script", zenko_core::LogStatusTone::Info)),
                    Some("0-tok"),
                )
                .with_call(task.id.clone())
                .with_payload(json!({
                    "job_id": job.id,
                    "task_id": task.id,
                    "script_path": raw_script,
                    "log_path": task.log_path,
                }));
                self.logs.push(cron_entry);
                Ok(json!({
                    "mode": "script",
                    "task": task,
                }))
            }
            CronKind::Cron | CronKind::Heartbeat => {
                let session = self.store.find_or_create_channel_session(
                    &job.target_channel,
                    job.target_chat_id.as_deref().unwrap_or(&job.id),
                    job.target_thread_id.as_deref(),
                    &format!("Cron: {}", job.name),
                    cfg.policy.mode,
                )?;
                let cron_entry = zenko_core::emit_hono_line(
                    zenko_core::LogDirection::Work,
                    zenko_core::LogChannel::Cron,
                    &format!("{} ({})", job.name, job.cron_expr),
                    Some((job.kind.as_str(), zenko_core::LogStatusTone::Info)),
                    Some("firing"),
                )
                .with_session(session.id.clone())
                .with_payload(json!({
                    "job_id": job.id,
                    "job_name": job.name,
                    "cron_expr": job.cron_expr,
                    "kind": job.kind.as_str(),
                    "target_channel": job.target_channel,
                }));
                self.logs.push(cron_entry);

                let rt = self.clone();
                let prompt = job.prompt.clone();
                let job_name = job.name.clone();
                let job_id = job.id.clone();
                let sid = session.id.clone();
                let headless = !rt.is_telegram_connected();
                tokio::spawn(async move {
                    if let Err(e) = rt
                        .run_turn_with_options(
                            &sid,
                            &prompt,
                            TurnOptions {
                                headless_no_wait: headless,
                                ..TurnOptions::default()
                            },
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "cron job run failed");
                        let err_entry = zenko_core::emit_hono_line(
                            zenko_core::LogDirection::Err,
                            zenko_core::LogChannel::Cron,
                            &format!("{job_name} · {e}"),
                            Some(("err", zenko_core::LogStatusTone::Err)),
                            None,
                        )
                        .with_session(sid.clone())
                        .with_payload(json!({
                            "job_id": job_id,
                            "error": e.to_string(),
                        }));
                        rt.logs.push(err_entry);
                    }
                });
                Ok(json!({
                    "mode": job.kind.as_str(),
                    "session_id": session.id,
                }))
            }
        }
    }

    pub async fn trigger_cron_job_now(self: &Arc<Self>, id: &str) -> Result<Value> {
        let mut job = self
            .store
            .get_cron_job(id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("cron job `{id}` not found")))?;
        job.last_run_at = Some(Utc::now());
        match self.dispatch_cron_job(&job).await {
            Ok(info) => {
                job.last_status = Some("dispatched".into());
                let _ = self.store.upsert_cron_job(&job);
                Ok(json!({ "ok": true, "job": job, "dispatch": info }))
            }
            Err(e) => {
                job.last_status = Some(format!("error: {e}"));
                let _ = self.store.upsert_cron_job(&job);
                Err(e)
            }
        }
    }

    /// Records a workspace mutation timestamp for watcher self-edit feedback-loop suppression.
    pub fn record_workspace_mutation(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_workspace_mutation_ms
            .store(now_ms, Ordering::Relaxed);
    }

    pub fn recently_mutated_workspace(&self, window_secs: u64) -> bool {
        let last = self.last_workspace_mutation_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        now_ms.saturating_sub(last) <= window_secs.saturating_mul(1_000)
    }

    /// Runs a reusable zero-token script inside `~/zenko_hub/scripts/` via `LocalJobSupervisor`.
    pub async fn dispatch_hub_script(
        self: &Arc<Self>,
        label: &str,
        script_rel: &str,
    ) -> Result<BackgroundJobRecord> {
        let cfg = self.config();
        let scripts_dir = cfg.ensure_hub_layout()?.join(HUB_SUBDIR_SCRIPTS);
        let resolved = zenko_tools::jail::resolve_jailed_path(&scripts_dir, script_rel, true)?;
        let cmd = build_script_command(&resolved);
        self.jobs
            .spawn_shell(
                "trigger",
                &cmd,
                &scripts_dir,
                TaskKind::Script,
                Some(label.to_string()),
                false,
            )
            .await
    }

    // ─── Phase 8: Persistent Goal Loops, Event Triggers & Trajectory Export ────

    pub fn create_goal_loop(&self, req: GoalLoopCreateRequest) -> Result<GoalLoopRecord> {
        let cfg = self.config();
        if req.name.trim().is_empty() {
            return Err(ZenkoError::Tool("goal loop `name` cannot be empty".into()));
        }
        if req.goal_text.trim().is_empty() {
            return Err(ZenkoError::Tool(
                "goal loop `goal_text` cannot be empty".into(),
            ));
        }
        evaluate_verify_command_safety(&cfg.policy, &req.verify_cmd)
            .map_err(ZenkoError::PolicyDenied)?;

        let ws_dir = req
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(zenko_core::expand_tilde)
            .unwrap_or_else(|| cfg.resolved_workspace_dir());

        let session_id = match req
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(sid) => sid.to_string(),
            None => {
                let s = self.store.create_session(
                    &format!("Goal: {}", req.name.trim()),
                    "goal",
                    None,
                    None,
                    cfg.policy.mode,
                )?;
                let _ = self
                    .store
                    .set_session_workspace(&s.id, &ws_dir.display().to_string());
                s.id
            }
        };

        let now = Utc::now();
        let goal = GoalLoopRecord {
            id: new_goal_loop_id(),
            name: req.name.trim().to_string(),
            session_id,
            workspace_dir: ws_dir.display().to_string(),
            goal_text: req.goal_text.trim().to_string(),
            verify_cmd: req.verify_cmd,
            interval_secs: req
                .interval_secs
                .unwrap_or(GOAL_LOOP_DEFAULT_INTERVAL_SECS),
            max_iterations: req
                .max_iterations
                .unwrap_or(GOAL_LOOP_DEFAULT_MAX_ITERATIONS)
                .clamp(1, GOAL_LOOP_MAX_ITERATIONS_CAP),
            max_budget_usd: req
                .max_budget_usd
                .unwrap_or(GOAL_LOOP_DEFAULT_MAX_BUDGET_USD)
                .max(0.0),
            max_consecutive_stalls: req
                .max_consecutive_stalls
                .unwrap_or(GOAL_LOOP_DEFAULT_STALL_LIMIT)
                .clamp(1, GOAL_LOOP_MAX_STALL_CAP),
            checkpoint_each_iter: req.checkpoint_each_iter.unwrap_or(true),
            auto_rewind_on_stall: req.auto_rewind_on_stall.unwrap_or(false),
            model: req
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            notify_channel: req
                .notify_channel
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(GOAL_LOOP_DEFAULT_NOTIFY_CHANNEL)
                .to_string(),
            status: GoalLoopStatus::Active,
            iterations_done: 0,
            consecutive_stalls: 0,
            cost_spent_usd: 0.0,
            last_verify_exit: None,
            last_verify_hash: None,
            last_verify_output: None,
            next_run_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        self.store.upsert_goal_loop(&goal)?;
        self.goal_notify.notify_one();
        self.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        Ok(goal)
    }

    pub fn update_goal_loop(
        &self,
        id: &str,
        patch: GoalLoopPatch,
    ) -> Result<Option<GoalLoopRecord>> {
        let Some(mut goal) = self.store.get_goal_loop(id)? else {
            return Ok(None);
        };
        let cfg = self.config();
        if let Some(name) = patch.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            goal.name = name.to_string();
        }
        if let Some(text) = patch
            .goal_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            goal.goal_text = text.to_string();
        }
        if let Some(cmd) = patch.verify_cmd {
            evaluate_verify_command_safety(&cfg.policy, &cmd).map_err(ZenkoError::PolicyDenied)?;
            goal.verify_cmd = cmd;
        }
        if let Some(iv) = patch.interval_secs {
            goal.interval_secs = iv;
        }
        if let Some(mi) = patch.max_iterations {
            goal.max_iterations = mi.clamp(1, GOAL_LOOP_MAX_ITERATIONS_CAP);
        }
        if let Some(mb) = patch.max_budget_usd {
            goal.max_budget_usd = mb.max(0.0);
        }
        if patch.reset_stalls == Some(true) {
            goal.consecutive_stalls = 0;
        }
        if let Some(status) = patch.status {
            goal.status = status;
            goal.next_run_at = match status {
                GoalLoopStatus::Active => Some(Utc::now()),
                _ => None,
            };
        }
        goal.updated_at = Utc::now();
        self.store.upsert_goal_loop(&goal)?;
        self.goal_notify.notify_one();
        self.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        Ok(Some(goal))
    }

    pub fn resume_and_wake_goal(&self, id: &str) -> Result<Option<GoalLoopRecord>> {
        self.update_goal_loop(
            id,
            GoalLoopPatch {
                status: Some(GoalLoopStatus::Active),
                reset_stalls: Some(true),
                ..GoalLoopPatch::default()
            },
        )
    }

    pub fn delete_goal_loop(&self, id: &str) -> Result<bool> {
        let deleted = self.store.delete_goal_loop(id)?;
        if deleted {
            self.goal_notify.notify_one();
        }
        Ok(deleted)
    }

    pub fn list_triggers(&self) -> Vec<TriggerStatusEntry> {
        let cfg = self.config();
        let mut out = Vec::new();
        for rule in &cfg.webhooks.rules {
            let snap = self.trigger_limiter.snapshot(&format!("webhook:{}", rule.id));
            let target = match rule.goal_loop_id.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(gid) => format!("goal:{gid}"),
                None => format!(
                    "session:{}",
                    rule.session_key.as_deref().unwrap_or(rule.source.as_str())
                ),
            };
            let filter_desc = match rule.match_pointer.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(ptr) => format!(
                    "source={} ptr={} values=[{}]",
                    rule.source,
                    ptr,
                    rule.match_values.join(",")
                ),
                None => format!("source={}", rule.source),
            };
            out.push(TriggerStatusEntry {
                id: rule.id.clone(),
                kind: "webhook_rule".into(),
                enabled: cfg.webhooks.enabled && rule.enabled,
                target,
                fires_last_hour: snap.fires_last_hour,
                max_per_hour: rule.max_per_hour,
                cooldown_secs: rule.cooldown_secs,
                last_fired_at: snap.last_fired_at,
                detail: filter_desc,
            });
        }
        for w in &cfg.watchers {
            let snap = self.trigger_limiter.snapshot(&format!("watcher:{}", w.id));
            let target = match w.action {
                zenko_core::WatcherActionKind::GoalLoop => {
                    format!("goal:{}", w.goal_loop_id.as_deref().unwrap_or("-"))
                }
                zenko_core::WatcherActionKind::Script => {
                    format!("script:{}", w.script_path.as_deref().unwrap_or("-"))
                }
                zenko_core::WatcherActionKind::Prompt => format!("watcher:{}", w.id),
            };
            out.push(TriggerStatusEntry {
                id: w.id.clone(),
                kind: "watcher".into(),
                enabled: w.enabled,
                target,
                fires_last_hour: snap.fires_last_hour,
                max_per_hour: w.max_per_hour,
                cooldown_secs: w.cooldown_secs,
                last_fired_at: snap.last_fired_at,
                detail: format!("path={} glob={} action={}", w.path, w.glob, w.action.as_str()),
            });
        }
        out
    }

    pub fn export_session_trajectory(
        &self,
        session_id: &str,
        opts: &ExportOptions,
    ) -> Result<ExportReport> {
        let secrets = self.vault.all_values();
        let home = zenko_core::expand_tilde("~");
        crate::export::export_session(&self.store, session_id, opts, &secrets, Some(&home))
    }

    pub fn export_batch_trajectories(&self, opts: &ExportOptions) -> Result<ExportReport> {
        let secrets = self.vault.all_values();
        let home = zenko_core::expand_tilde("~");
        crate::export::export_batch(&self.store, opts, &secrets, Some(&home))
    }

    // ─── Zero-CPU `sleep_until` Cron & Heartbeat Scheduler ─────────────────────

    pub async fn run_cron_scheduler(self: Arc<Self>, cancel: CancellationToken) {
        info!("cron & heartbeat scheduler started (zero-polling sleep_until)");
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let jobs = self.store.list_cron_jobs().unwrap_or_default();
            let now = Utc::now();
            let mut next_due: Option<(chrono::DateTime<Utc>, CronJob)> = None;

            for mut job in jobs.into_iter().filter(|j| j.enabled) {
                let due = match job.next_run_at {
                    Some(t) => t,
                    None => {
                        let Ok(cron) = Cron::new(&job.cron_expr).parse() else {
                            continue;
                        };
                        let Ok(n) = cron.find_next_occurrence(&now, false) else {
                            continue;
                        };
                        job.next_run_at = Some(n);
                        // Guard against resurrecting a job deleted while this run was in flight.
                    if self.store.get_cron_job(&job.id).ok().flatten().is_some() {
                        let _ = self.store.upsert_cron_job(&job);
                    }
                        n
                    }
                };
                if next_due.as_ref().map(|(t, _)| due < *t).unwrap_or(true) {
                    next_due = Some((due, job));
                }
            }

            match next_due {
                None => {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = self.cron_notify.notified() => continue,
                    }
                }
                Some((due, mut job)) => {
                    let wait_ms = (due - now).num_milliseconds();
                    if wait_ms > 0 {
                        let sleep = tokio::time::sleep(Duration::from_millis(wait_ms as u64));
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = self.cron_notify.notified() => continue,
                            _ = sleep => {}
                        }
                    }

                    let next = Cron::new(&job.cron_expr)
                        .parse()
                        .ok()
                        .and_then(|c| c.find_next_occurrence(&Utc::now(), false).ok());
                    job.last_run_at = Some(Utc::now());
                    job.next_run_at = next;

                    match self.dispatch_cron_job(&job).await {
                        Ok(_) => {
                            job.last_status = Some("dispatched".into());
                        }
                        Err(e) => {
                            let err_entry = zenko_core::emit_hono_line(
                                zenko_core::LogDirection::Err,
                                zenko_core::LogChannel::Cron,
                                &format!("{} · {e}", job.name),
                                Some(("err", zenko_core::LogStatusTone::Err)),
                                None,
                            )
                            .with_payload(json!({
                                "job_id": job.id,
                                "error": e.to_string(),
                            }));
                            self.logs.push(err_entry);
                            job.last_status = Some(format!("error: {e}"));
                        }
                    }
                    // Guard against resurrecting a job deleted while this run was in flight.
                    if self.store.get_cron_job(&job.id).ok().flatten().is_some() {
                        let _ = self.store.upsert_cron_job(&job);
                    }
                }
            }
        }
    }

}
