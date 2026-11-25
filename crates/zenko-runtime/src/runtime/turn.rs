use super::*;

impl AgentRuntime {
    pub async fn run_turn(
        self: &Arc<Self>,
        session_id: &str,
        user_text: &str,
        parent_event_override: Option<Option<String>>,
    ) -> Result<String> {
        let res = self
            .run_turn_with_options(
                session_id,
                user_text,
                TurnOptions {
                    parent_event_override,
                    ..TurnOptions::default()
                },
            )
            .await?;
        Ok(res.reply)
    }

    /// Runs a full streaming ReAct turn with per-session serialization, generational
    /// cancellation safety, context compaction, and optional headless/workspace/mode overrides.
    pub async fn run_turn_with_options(
        self: &Arc<Self>,
        session_id: &str,
        user_text: &str,
        opts: TurnOptions,
    ) -> Result<TurnResult> {
        let cfg = self.config();

        if cfg.general.daily_spend_limit_usd > 0.0 {
            let today = self.store.daily_cost_usd().unwrap_or(0.0);
            if today >= cfg.general.daily_spend_limit_usd {
                return Err(ZenkoError::BudgetExceeded(format!(
                    "Daily spend limit (${:.2}) reached (today: ${:.4})",
                    cfg.general.daily_spend_limit_usd, today
                )));
            }
        }

        // Non-destructive concurrency control (algebraic dispatch via `resolve_session_dispatch`):
        // - Auto: Queue during focused foreground work, Steer while managing bg jobs / subagents
        // - Steer: inject guidance mid-flight if a turn is active (otherwise starts normally)
        // - Queue: persist into durable SQLite `session_queue` if busy (auto-runs FIFO on completion)
        // - Interrupt: explicitly cancel active turn and start immediately
        match opts.send_mode {
            SendMode::Interrupt => {
                self.cancel_session(session_id);
            }
            mode => match self.resolve_session_dispatch(session_id, mode) {
                AutoDispatchDecision::Steer => {
                    if self.steer_session(session_id, user_text) {
                        return Ok(TurnResult {
                            session_id: session_id.to_string(),
                            reply: "Steering guidance queued for active turn.".into(),
                            model: cfg.provider.lead_model,
                            usage: UsageStats::default(),
                            tool_calls_count: 0,
                        });
                    }
                }
                AutoDispatchDecision::Queue => {
                    let (queued, count) = self.enqueue_session_followup(session_id, user_text)?;
                    return Ok(TurnResult {
                        session_id: session_id.to_string(),
                        reply: format!(
                            "Queued follow-up prompt (#{count}, id={}). It will run automatically after the current turn.",
                            queued.id
                        ),
                        model: cfg.provider.lead_model,
                        usage: UsageStats::default(),
                        tool_calls_count: 0,
                    });
                }
                AutoDispatchDecision::StartTurn => {}
            },
        }

        let headless_for_drain = opts.headless_no_wait;
        let result = self
            .run_single_turn(&cfg, session_id, user_text, opts)
            .await;

        if result.is_ok() {
            let has_queued = self
                .store
                .list_queued_followups(session_id)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            match (has_queued, self.has_job_notices(session_id)) {
                (true, _) => {
                    let rt = self.clone();
                    let sid = session_id.to_string();
                    tokio::spawn(async move {
                        rt.drain_followup_queue(&sid, headless_for_drain).await;
                    });
                }
                (false, true) => {
                    let _ = self.wake_tx.send(session_id.to_string());
                }
                (false, false) => {}
            }
        }

        result
    }

    async fn drain_followup_queue(self: &Arc<Self>, session_id: &str, headless_no_wait: bool) {
        while let Ok(Some(next_prompt)) = self.store.pop_next_followup(session_id) {
            let remaining = self
                .store
                .list_queued_followups(session_id)
                .map(|v| v.len())
                .unwrap_or(0);
            self.emit(AgentEvent::QueueUpdated {
                session_id: session_id.to_string(),
                queued_count: remaining,
            });
            let cfg = self.config();
            match self
                .run_single_turn(
                    &cfg,
                    session_id,
                    &next_prompt.text,
                    TurnOptions {
                        headless_no_wait,
                        send_mode: SendMode::Queue,
                        ..TurnOptions::default()
                    },
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    warn!(session_id = %session_id, error = %e, "queued follow-up turn failed");
                    break;
                }
            }
        }
        if self.has_job_notices(session_id) {
            let _ = self.wake_tx.send(session_id.to_string());
        }
    }

    async fn run_single_turn(
        self: &Arc<Self>,
        cfg: &ZenkoConfig,
        session_id: &str,
        user_text: &str,
        opts: TurnOptions,
    ) -> Result<TurnResult> {
        let session_lock = self.session_mutex(session_id);
        let _turn_guard = session_lock.lock().await;

        let mut session = self
            .store
            .get_session(session_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("session {session_id} not found")))?;

        let my_gen = self.turn_seq.fetch_add(1, Ordering::Relaxed);
        let cancel = opts
            .external_cancel
            .as_ref()
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let demote_bg = Arc::new(Notify::new());
        {
            let mut map = self
                .active_cancels
                .lock()
                .map_err(|_| ZenkoError::Config("cancel lock poisoned".into()))?;
            map.insert(session_id.to_string(), (my_gen, cancel.clone()));
        }
        if let Ok(mut dmap) = self.demote_signals.lock() {
            dmap.insert(session_id.to_string(), (my_gen, demote_bg.clone()));
        }

        let mut turn_checkpoint_id: Option<String> = None;
        let result = self
            .execute_turn_inner(
                cfg,
                &mut session,
                user_text,
                opts,
                cancel,
                demote_bg,
                &mut turn_checkpoint_id,
            )
            .await;

        // Always finalize pre-mutation workspace checkpoint regardless of Ok/Err/Cancel so
        // partial mutations never leave `post_ref = None` or break hand-edit detection!
        if let Some(ref cp_id) = turn_checkpoint_id {
            let _ = self
                .checkpoints
                .finalize_checkpoint(&cfg.checkpoints, cp_id)
                .await;
        }

        // Generational cleanup: only remove our own tokens so a newer turn is never clobbered.
        if let Ok(mut map) = self.active_cancels.lock()
            && map.get(session_id).map(|(g, _)| *g) == Some(my_gen) {
                map.remove(session_id);
            }
        if let Ok(mut dmap) = self.demote_signals.lock()
            && dmap.get(session_id).map(|(g, _)| *g) == Some(my_gen) {
                dmap.remove(session_id);
            }

        match &result {
            Ok(_) | Err(ZenkoError::Cancelled) => {
                let _ = self.store.update_session_status(session_id, SessionStatus::Idle);
                self.emit(AgentEvent::SessionUpdated {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Idle,
                    title: None,
                });
            }
            Err(e) => {
                let _ = self.store.update_session_status(session_id, SessionStatus::Error);
                self.emit(AgentEvent::TurnError {
                    session_id: session_id.to_string(),
                    message: e.to_string(),
                });
                self.emit(AgentEvent::SessionUpdated {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Error,
                    title: None,
                });
            }
        }

        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_turn_inner(
        self: &Arc<Self>,
        cfg: &ZenkoConfig,
        session: &mut Session,
        user_text: &str,
        opts: TurnOptions,
        cancel: CancellationToken,
        demote_bg: Arc<Notify>,
        turn_checkpoint_id: &mut Option<String>,
    ) -> Result<TurnResult> {
        let session_id = session.id.clone();
        self.store.update_session_status(&session_id, SessionStatus::Running)?;

        let is_first_turn = session.head_event_id.is_none();
        if is_first_turn {
            let _ = run_hooks(
                &cfg.hooks,
                HookEvent::SessionStart,
                None,
                json!({ "session_id": session_id, "channel": session.channel }),
            )
            .await;
        }

        let new_title = match is_first_turn
            && (session.title == "New Session"
                || session.title.starts_with("Session ")
                || session.title == "CLI Session")
        {
            true => {
                let t = make_session_title(user_text);
                self.store.update_session_meta(&session_id, Some(&t), None, None)?;
                session.title = t.clone();
                Some(t)
            }
            false => None,
        };

        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.clone(),
            status: SessionStatus::Running,
            title: new_title,
        });

        if let Err(reason) = run_hooks(
            &cfg.hooks,
            HookEvent::UserPromptSubmit,
            None,
            json!({ "session_id": session_id, "text": user_text }),
        )
        .await
        {
            return Err(ZenkoError::PolicyDenied(reason));
        }

        let workspace_dir = opts
            .workspace_override
            .clone()
            .or_else(|| {
                session
                    .workspace_dir
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| cfg.resolved_workspace_dir());
        let canonical_ws_str = resolve_git_root(&workspace_dir).display().to_string();
        if session.workspace_dir.as_deref() != Some(canonical_ws_str.as_str()) {
            let _ = self.store.set_session_workspace(&session_id, &canonical_ws_str);
            session.workspace_dir = Some(canonical_ws_str);
        }
        let project_scope = project_scope_for_workspace(&workspace_dir);
        let snapshot = self.frozen_snapshot_for_session(&session_id, &workspace_dir, cfg)?;
        if session.frozen_system_hash.is_none() {
            let _ = self
                .store
                .set_session_frozen_hash(&session_id, &snapshot.tier1_hash);
        }

        let parent_for_user = match opts.parent_event_override {
            Some(explicit_parent) => explicit_parent,
            None => session.head_event_id.clone(),
        };
        let user_ev = self.store.append_event(
            &session_id,
            parent_for_user.as_deref(),
            EventRole::User,
            EventKind::Message,
            json!({ "text": user_text }),
            &UsageStats::default(),
        )?;
        let mut head_id = Some(user_ev.id.clone());

        self.emit(AgentEvent::UserMessageAccepted {
            session_id: session_id.clone(),
            event_id: user_ev.id,
            text: user_text.to_string(),
        });

        let effective_permission = opts.permission_override.unwrap_or(session.permission_mode);
        let initial_branch = self
            .store
            .get_active_branch(&session_id)
            .unwrap_or_default();
        let prior_turn_errors = initial_branch
            .iter()
            .rev()
            .take(8)
            .filter(|ev| {
                ev.role == EventRole::Tool
                    && ev
                        .content_json
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
            .count();

        let mut orch = orchestrator::prepare_turn(
            cfg,
            &session_id,
            user_text,
            workspace_dir.clone(),
            cfg.resolved_hub_dir(),
            effective_permission,
            is_first_turn,
            prior_turn_errors,
            opts.model_override.as_deref(),
            session.model_override.as_deref(),
            opts.headless_no_wait,
            self.store.clone(),
            self.jobs.clone(),
        )
        .await;

        // Multi-turn plan continuity: hydrate durable SQLite `session_tasks` DAG first
        // (surviving compaction & daemon restarts!), falling back to latest event-branch plan.
        let persisted_tasks = self
            .clarifications
            .list_session_tasks(&session_id)
            .unwrap_or_default();
        if let Some(sqlite_plan) = execution_plan_from_session_tasks(&session.title, &persisted_tasks) {
            if !orchestrator::incomplete_steps(&sqlite_plan).is_empty() {
                orch.guard.hydrate_active_plan(sqlite_plan);
            }
        } else if let Some(carried_plan) = load_latest_active_plan(&initial_branch) {
            orch.guard.hydrate_active_plan(carried_plan);
        }
        // Hydrate latest abandoned-branch rewind summary if the session was just rewound
        let latest_rewind_summary: Option<String> = initial_branch
            .iter()
            .rev()
            .take(3)
            .find_map(|ev| {
                if ev.role == EventRole::System
                    && ev.kind == EventKind::SystemNote
                    && ev.content_json.get("note_kind").and_then(Value::as_str)
                        == Some(NOTE_KIND_REWIND_SUMMARY)
                {
                    ev.content_json
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(String::from)
                } else {
                    None
                }
            });
        // Hydrate session-scoped activated deferred tools from prior turns
        let _ = self.hydrate_activated_deferred(&session_id, &initial_branch);

        // Oh My Pi cache-safe First-Turn Auto-Recall for Standard turns (Orchestrated turns
        // already recall inside parallel recon). Persisted as `NOTE_KIND_MEMORY_RECALL` once.
        let mut first_turn_recall_block: Option<String> = None;
        if is_first_turn
            && orch.recon.is_none()
            && orch.triage.tier != zenko_core::ExecutionTier::Instant
            && cfg.memory.workspace_scoping
        {
            let already_recalled = initial_branch.iter().any(|ev| {
                ev.role == EventRole::System
                    && ev.kind == EventKind::SystemNote
                    && ev.content_json.get("note_kind").and_then(Value::as_str)
                        == Some(NOTE_KIND_MEMORY_RECALL)
            });
            if !already_recalled {
                let q = filter_query_terms(user_text)
                    .into_iter()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !q.is_empty() {
                    let ws_check = match cfg.memory.verify_file_staleness {
                        true => Some(workspace_dir.as_path()),
                        false => None,
                    };
                    if let Ok(hits) = self.store.search_memory_hybrid(
                        &q,
                        Some(&project_scope),
                        true,
                        ws_check,
                        MEMORY_AUTO_RECALL_LIMIT,
                    ) {
                        let strong: Vec<_> = hits
                            .into_iter()
                            .filter(|h| h.score >= MEMORY_AUTO_RECALL_MIN_SCORE)
                            .collect();
                        if !strong.is_empty() {
                            let lines: Vec<String> = strong
                                .iter()
                                .map(|h| {
                                    let stale = match h.stale_paths.is_empty() {
                                        true => String::new(),
                                        false => format!(
                                            " [stale-path: {}]",
                                            h.stale_paths.join(", ")
                                        ),
                                    };
                                    format!(
                                        "- [{}][{}] {}: {}{}",
                                        h.entry.scope,
                                        h.entry.kind,
                                        h.entry.title,
                                        compact_preview(&h.entry.content, 120),
                                        stale
                                    )
                                })
                                .collect();
                            let block = lines.join("\n");
                            if let Ok(rec_ev) = self.store.append_event(
                                &session_id,
                                head_id.as_deref(),
                                EventRole::System,
                                EventKind::SystemNote,
                                json!({
                                    "note_kind": NOTE_KIND_MEMORY_RECALL,
                                    "scope": &project_scope,
                                    "summary": &block,
                                }),
                                &UsageStats::default(),
                            ) {
                                head_id = Some(rec_ev.id);
                            }
                            first_turn_recall_block = Some(block);
                        }
                    }
                }
            }
        }

        let mut model = match orch.route.as_ref() {
            Some(dec) => dec.model_id.clone(),
            None => self
                .list_models_cached()
                .await
                .ok()
                .and_then(|list| list.into_iter().next().map(|m| m.id))
                .ok_or_else(|| {
                    ZenkoError::Config(
                        "No model selected and live model discovery returned empty. Choose a model in Settings or `zenko setup`.".into(),
                    )
                })?,
        };
        let mut reasoning_effort = orch.reasoning_effort;

        if cfg.orchestrator.enabled {
            let route_tag = orch.route.as_ref().map(|r| r.source).unwrap_or("discovered");
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_TRIAGED.into(),
                tier: orch.triage.tier,
                detail: format!("{} (route={route_tag})", orch.triage.reason),
                model: Some(model.clone()),
            });
            if let Some(ref rec) = orch.recon {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_RECON.into(),
                    tier: orch.triage.tier,
                    detail: format!(
                        "{} entries, {} markers, {} prompt files, {} memory hits ({}ms)",
                        rec.workspace_entries.len(),
                        rec.repo_markers.len(),
                        rec.mentioned_files.len(),
                        rec.memory_hits.len(),
                        rec.elapsed_ms
                    ),
                    model: None,
                });
            }
        }

        let mut context_window = self.resolve_context_window(&model);
        let mut active_max_tokens = self.effective_output_budget(
            cfg.provider.max_tokens,
            &model,
            cfg.orchestrator.enabled && orch.triage.tier == zenko_core::ExecutionTier::Orchestrated,
        );
        let active_role = opts
            .role_id
            .as_deref()
            .or(session.agent_role.as_deref())
            .and_then(find_subagent_role);
        let effective_custom_read_only = opts.custom_read_only.unwrap_or(false);
        let custom_tool_subset: HashSet<String> = opts
            .custom_tool_subset
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let session_scope = SessionToolScope {
            role_id: active_role.map(|r| r.id.to_string()),
            permission: effective_permission,
            custom_read_only: effective_custom_read_only,
            custom_tool_subset: custom_tool_subset.clone(),
            availability: ToolAvailabilityContext {
                has_mcp_servers: self.has_configured_mcp_servers(),
                has_skills: !snapshot.skills.is_empty(),
                is_subagent: active_role.is_some(),
                read_only: effective_permission == PermissionMode::Plan
                    || effective_custom_read_only
                    || active_role.is_some_and(|r| r.read_only),
            },
        };
        let deferred_catalog = match cfg.context.deferred_tool_loading {
            true => self
                .tools
                .deferred_catalog_text_filtered(&session_scope.availability, |spec| {
                    session_scope.permits_spec(spec)
                }),
            false => None,
        };
        let active_concise_mode = self.effective_concise_mode(&session_id, cfg, &initial_branch);
        let mut effective_system_prompt = snapshot.compose_system_prompt(deferred_catalog.as_deref());
        if let Some(concise_block) = render_concise_prompt_block(
            active_concise_mode,
            cfg.context.auto_clarity,
            session_scope.availability.read_only,
        ) {
            effective_system_prompt.push_str("\n\n");
            effective_system_prompt.push_str(&concise_block);
        }
        if let Some(role_desc) = active_role {
            let is_ro = role_desc.read_only || effective_custom_read_only;
            let custom_block = opts
                .custom_directive
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|d| {
                    format!(
                        "\n\nSpecialist Directive:\n{}",
                        compact_preview(d, SUBAGENT_CUSTOM_DIRECTIVE_MAX_CHARS)
                    )
                })
                .unwrap_or_default();
            effective_system_prompt.push_str(&format!(
                "\n\n<subagent_directive role=\"{}\" read_only=\"{is_ro}\">\n{}{custom_block}\n</subagent_directive>",
                role_desc.id, role_desc.system_addendum
            ));
        }
        if let Some(ref schema) = opts.output_schema {
            effective_system_prompt.push_str("\n\n");
            effective_system_prompt.push_str(&format_output_schema_instruction(schema));
        }
        let session_ctrl = self.session_control(&session_id);
        let _active_scope_guard =
            ActiveSessionScopeGuard::enter(session_ctrl.clone(), session_scope.clone());
        session_ctrl
            .explicit_model_locked
            .store(orch.has_explicit_model_override, Ordering::Relaxed);
        if let Ok(mut pending) = session_ctrl.pending_control.lock() {
            *pending = None;
        }
        let effective_sys_tokens = estimate_text_tokens(&effective_system_prompt);
        let tool_ctx = ToolContext {
            session_id: session_id.clone(),
            workspace_dir,
            data_dir: cfg.resolved_data_dir(),
            hub_dir: cfg.resolved_hub_dir(),
            shell_timeout_secs: cfg.policy.shell_timeout_secs,
            auto_background_after_secs: cfg.policy.auto_background_after_secs,
            max_output_bytes: cfg.policy.max_output_bytes,
            headless_no_wait: opts.headless_no_wait,
            demote_bg,
            file_state: Some(self.file_state.clone() as Arc<dyn FileStateBridge>),
        };

        let mut final_assistant_text = String::new();
        let mut ended_with_final_text = false;
        let mut truncation_retries = 0usize;
        let mut schema_retries = 0usize;
        let mut total_usage = UsageStats::default();
        let mut tool_calls_count = 0usize;
        let mut pending_orch_hints: Vec<String> = Vec::new();
        let turn_checkpoint_event_id = parent_for_user.clone();
        let turn_checkpoint_label =
            format!("Before: {}", compact_preview(user_text, CHECKPOINT_LABEL_MAX_CHARS));
        let max_iters = opts
            .max_iterations_override
            .unwrap_or(cfg.general.max_tool_iterations)
            .clamp(1, 64);

        if effective_permission == PermissionMode::Plan {
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_PLAN_MODE.into(),
                tier: orch.triage.tier,
                detail: "Read-only Plan Mode active — mutations blocked until plan is approved.".into(),
                model: Some(model.clone()),
            });
        }

        for iter_idx in 0..max_iters {
            if cancel.is_cancelled() {
                return Err(ZenkoError::Cancelled);
            }

            if iter_idx > 0 {
                if let Some(shift) = self.take_pending_control(&session_id) {
                    if let Some(tier_role) = shift.cognitive_tier
                        && !orch.has_explicit_model_override
                        && let Some(dec) = orchestrator::router::select_turn_model(
                            cfg,
                            None,
                            None,
                            tier_role,
                            None,
                        )
                        && dec.model_id != model
                    {
                        model = dec.model_id;
                        context_window = self.resolve_context_window(&model);
                        active_max_tokens = self.effective_output_budget(
                            cfg.provider.max_tokens,
                            &model,
                            true,
                        );
                    }
                    if let Some(eff) = shift.reasoning_effort {
                        reasoning_effort = Some(eff);
                    }
                    if let Some(cm) = shift.concise_mode {
                        let desc = find_concise_mode(cm);
                        let hint = match cm.is_active() {
                            true => format!(
                                "Live Concise Mode updated to `{}`: {} Preserve code, paths, symbols, and errors verbatim.",
                                desc.id, desc.directive
                            ),
                            false => "Live Concise Mode disabled (`off`): respond in standard clear English prose.".to_string(),
                        };
                        pending_orch_hints.push(hint);
                    }
                }
                for steer_text in self.drain_steering(&session_id) {
                    let steer_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::User,
                        EventKind::Message,
                        json!({
                            "text": format!("[Live Steering Update] {steer_text}"),
                            "steering": true,
                        }),
                        &UsageStats::default(),
                    )?;
                    head_id = Some(steer_ev.id);
                }
            }

            let active_deferred = match cfg.context.freeze_tool_schemas {
                true => BTreeSet::new(),
                false => self.activated_deferred_for(&session_id),
            };
            let active_tool_specs = self.tools.specs_for_session_filtered(
                cfg.context.deferred_tool_loading,
                &active_deferred,
                &session_scope.availability,
                |spec| session_scope.permits_spec(spec),
            );
            let prefix_tokens =
                effective_sys_tokens + estimate_tool_specs_tokens(&active_tool_specs);

            let mut branch_events = self.store.get_active_branch(&session_id)?;
            let mut ckpt_slice = slice_from_latest_checkpoint(&branch_events);
            let mut raw_messages = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
            if let Some(ref ckpt) = ckpt_slice.checkpoint {
                prepend_checkpoint_to_messages(&mut raw_messages, ckpt);
            }

            // Stage A: Zero-cost selective microcompaction (protects recent tail + planning tools)
            let (mut messages, mc_stats) = match cfg.context.enabled {
                true => microcompact_messages(
                    raw_messages,
                    context_window,
                    prefix_tokens,
                    cfg.context.microcompact_trigger_ratio,
                ),
                false => (raw_messages, Default::default()),
            };
            if mc_stats.compacted_tools > 0 {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_MICROCOMPACT.into(),
                    tier: orch.triage.tier,
                    detail: format!(
                        "Microcompacted {} old tool result(s), reclaimed ~{} tok",
                        mc_stats.compacted_tools, mc_stats.tokens_saved
                    ),
                    model: None,
                });
            }

            // Stage B: Persisted iterative-merge CompactionCheckpoint when context still exceeds trigger
            let stage_b_budget =
                ((context_window as f64) * cfg.context.compaction_trigger_ratio.clamp(0.45, 0.94))
                    as usize;
            let mut compacted_this_iter = false;
            if cfg.context.enabled
                && prefix_tokens + estimate_messages_tokens(&messages) > stage_b_budget
                && let Some(plan) = plan_checkpoint_split(
                    &ckpt_slice,
                    context_window,
                    prefix_tokens,
                    cfg.context.compaction_trigger_ratio,
                ) {
                    let ckpt_payload = self
                        .run_stage_b_compaction(
                            cfg,
                            &session_id,
                            &model,
                            &plan,
                            ckpt_slice.checkpoint.as_ref(),
                            orch.guard.active_plan(),
                            prefix_tokens,
                            &cancel,
                        )
                        .await;
                    let ckpt_json = serde_json::to_value(&ckpt_payload).unwrap_or_else(|_| json!({}));
                    let ckpt_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::System,
                        EventKind::CompactionCheckpoint,
                        ckpt_json,
                        &UsageStats::default(),
                    )?;
                    head_id = Some(ckpt_ev.id);
                    compacted_this_iter = true;
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_COMPACTION.into(),
                        tier: orch.triage.tier,
                        detail: format!(
                            "Compacted {} events ({} -> {} tok, model={})",
                            ckpt_payload.compacted_events_count,
                            ckpt_payload.estimated_tokens_before,
                            ckpt_payload.estimated_tokens_after,
                            ckpt_payload.model
                        ),
                        model: Some(ckpt_payload.model.clone()),
                    });

                    branch_events = self.store.get_active_branch(&session_id)?;
                    ckpt_slice = slice_from_latest_checkpoint(&branch_events);
                    let mut rebuilt = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
                    if let Some(ref ckpt) = ckpt_slice.checkpoint {
                        prepend_checkpoint_to_messages(&mut rebuilt, ckpt);
                    }
                    messages = microcompact_messages(
                        rebuilt,
                        context_window,
                        prefix_tokens,
                        cfg.context.microcompact_trigger_ratio,
                    )
                    .0;
                }

            // Emergency pair-preserving hard ceiling (covers Stage-B cooldown edge cases)
            let before_hard_len = messages.len();
            messages = compact_messages_to_budget(messages, context_window, prefix_tokens);
            let hard_compacted_now = messages.len() < before_hard_len
                && messages
                    .first()
                    .is_some_and(|m| m.content.contains("<compacted_history>"));
            let compacted_now = compacted_this_iter || hard_compacted_now;

            // Collect all volatile turn guidance, hints, plan continuity, and background job notices
            // into a single cache-safe `<system_reminder>` (never injecting mid-array `role: "system"`
            // that would be hoisted by Anthropic/OpenRouter proxies and bust Tier-1 KV cache!).
            let mut tail_reminders: Vec<String> = Vec::with_capacity(4);
            if iter_idx == 0 {
                if effective_permission == PermissionMode::Plan {
                    tail_reminders.push(
                        "<plan_mode active=\"true\">\n\
                         STRICT READ-ONLY PLAN MODE IS ACTIVE.\n\
                         - You may inspect files (`read_file`, `code_search`, `list_dir`), run strictly read-only shell commands (`git status`, `git diff`, `git log`, `rg`, `ls`), spawn read-only subagents (`explorer`, `architect`), and create structured plans via `record_brainstorm`, `plan_update`, and `session_task`.\n\
                         - All file mutations, mutating shell commands, and memory/cron writes are blocked until the user approves the plan.\n\
                         - Produce a concrete, actionable Task DAG (`session_task` or `plan_update`) with dependencies and verification commands, then summarize clearly for approval.\n\
                         </plan_mode>".into(),
                    );
                }
                if let Some(ref rw_sum) = latest_rewind_summary {
                    tail_reminders.push(format!(
                        "<rewind_summary>\nPrevious attempt was rewound. Use these lessons to avoid repeating failed paths:\n{rw_sum}\n</rewind_summary>"
                    ));
                }
                if let Some(ref guide) = orch.guidance_block {
                    tail_reminders.push(guide.clone());
                }
                if let Some(ref recall) = first_turn_recall_block {
                    tail_reminders.push(format!(
                        "<recalled_memories scope=\"{project_scope}\">\nRelevant persistent memories auto-recalled for this workspace:\n{recall}\n</recalled_memories>"
                    ));
                }
                let live_tasks = self
                    .clarifications
                    .list_session_tasks(&session_id)
                    .unwrap_or_default();
                if let Some(dag_block) = format_session_tasks_prompt_block(&live_tasks) {
                    tail_reminders.push(dag_block);
                } else {
                    let plan_already_in_recon = orch.guidance_block.is_some()
                        && orch
                            .recon
                            .as_ref()
                            .and_then(|r| r.prior_plan_summary.as_ref())
                            .is_some();
                    if !plan_already_in_recon
                        && let Some(plan) = orch.guard.active_plan() {
                            tail_reminders.push(format!(
                                "<active_execution_plan>\nContinue active DAG plan (treat decided steps as locked): {}\n</active_execution_plan>",
                                format_plan_summary(plan)
                            ));
                        }
                }
            }
            if compacted_now {
                if iter_idx > 0 {
                    let live_tasks = self
                        .clarifications
                        .list_session_tasks(&session_id)
                        .unwrap_or_default();
                    if let Some(dag_block) = format_session_tasks_prompt_block(&live_tasks) {
                        tail_reminders.push(format!(
                            "Context compacted — resume directly from the durable SQLite Task DAG below without re-planning:\n{dag_block}"
                        ));
                    } else if let Some(plan) = orch.guard.active_plan() {
                        tail_reminders.push(format!(
                            "<active_execution_plan>\nContext compacted — treat existing plan as DECIDED work and resume directly without re-planning: {}\n</active_execution_plan>",
                            format_plan_summary(plan)
                        ));
                    }
                }
                // Oh My Pi post-compaction recall (placed in Tier-3 tail so Tier-1 KV cache stays frozen!)
                if let Ok(proj_mems) = self.store.search_memory_hybrid(
                    "",
                    Some(&project_scope),
                    false,
                    Some(&tool_ctx.workspace_dir),
                    3,
                )
                    && !proj_mems.is_empty() {
                        let bullets = proj_mems
                            .into_iter()
                            .map(|h| {
                                format!(
                                    "- [{}] {}: {}",
                                    h.entry.kind,
                                    h.entry.title,
                                    compact_preview(&h.entry.content, 120)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        tail_reminders.push(format!(
                            "<project_memory_invariants>\nKey workspace invariants recalled post-compaction:\n{bullets}\n</project_memory_invariants>"
                        ));
                    }
            }
            if !pending_orch_hints.is_empty() {
                tail_reminders.append(&mut pending_orch_hints);
            }
            let mem_deltas = self.drain_memory_deltas(&session_id);
            if !mem_deltas.is_empty() {
                tail_reminders.push(format!(
                    "<memory_delta>\nPersistent memory updated mid-session (Tier-1/Tier-2 prefix stays frozen for KV cache):\n- {}\n</memory_delta>",
                    mem_deltas.join("\n- ")
                ));
            }
            let job_notices = self.drain_job_notices(&session_id);
            if !job_notices.is_empty() {
                let lines: Vec<&str> = job_notices.iter().map(WakeupNotice::as_text).collect();
                tail_reminders.push(format!(
                    "<background_job_updates>\n{}\n</background_job_updates>",
                    lines.join("\n")
                ));
            }
            if let Ok(unread_mail) = self.store.drain_unread_agent_messages(&session_id)
                && !unread_mail.is_empty() {
                    let lines: Vec<String> = unread_mail
                        .into_iter()
                        .map(|m| format!("- [from {}]: {}", m.from_session_id, m.body))
                        .collect();
                    tail_reminders.push(format!(
                        "<agent_mailbox>\nMessages received from related agents:\n{}\n</agent_mailbox>",
                        lines.join("\n")
                    ));
                }
            if !tail_reminders.is_empty() {
                append_cache_safe_reminder(&mut messages, tail_reminders.join("\n\n"));
            }

            let effective_effort = match (reasoning_effort, self.resolve_supports_reasoning(&model)) {
                (Some(_), Some(false)) => None,
                (eff, _) => eff,
            };

            let chat_req = ChatRequest {
                session_id: session_id.clone(),
                model: model.clone(),
                fallbacks: cfg.provider.fallbacks.clone(),
                system_prompt: effective_system_prompt.clone(),
                messages,
                tools: active_tool_specs,
                temperature: cfg.provider.temperature,
                max_tokens: active_max_tokens,
                enable_streaming: cfg.provider.enable_streaming,
                enable_prompt_caching: cfg.provider.enable_prompt_caching,
                sticky_session_routing: cfg.provider.sticky_session_routing,
                provider_sort: cfg.provider.provider_sort.clone(),
                reasoning_effort: effective_effort,
            };

            let wire_req_meta = match wire_debug_enabled() {
                true => Some(json!({
                    "iteration": iter_idx,
                    "system_prompt_chars": chat_req.system_prompt.len(),
                    "message_count": chat_req.messages.len(),
                    "messages": &chat_req.messages,
                    "tools_count": chat_req.tools.len(),
                    "temperature": chat_req.temperature,
                    "max_tokens": chat_req.max_tokens,
                    "enable_streaming": chat_req.enable_streaming,
                })),
                false => None,
            };

            let turn_id = new_id();
            let iter_started = std::time::Instant::now();
            let mut thinking_started: Option<std::time::Instant> = None;
            let mut thinking_ended: Option<std::time::Instant> = None;
            let mut stream = self.provider.stream_chat(chat_req, cancel.clone()).await?;

            let mut thinking_buf = String::new();
            let mut text_buf = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut usage = UsageStats::default();
            let mut finish_reason: Option<String> = None;

            while let Some(item) = stream.next().await {
                match item? {
                    LlmDelta::Thinking(t) => {
                        let now = std::time::Instant::now();
                        thinking_started.get_or_insert(now);
                        thinking_ended = Some(now);
                        thinking_buf.push_str(&t);
                        self.emit(AgentEvent::ThinkingDelta {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            text: t,
                        });
                    }
                    LlmDelta::Text(t) => {
                        text_buf.push_str(&t);
                        self.emit(AgentEvent::TextDelta {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            text: t,
                        });
                    }
                    LlmDelta::ToolCallComplete(tc) => {
                        tool_calls.push(tc);
                    }
                    LlmDelta::Usage(u) => {
                        usage = u;
                    }
                    LlmDelta::Done { finish_reason: fr } => {
                        finish_reason = fr;
                    }
                }
            }

            if let Some(req_meta) = wire_req_meta {
                self.log_sink.record_wire_exchange(
                    &session_id,
                    &turn_id,
                    &cfg.provider.kind,
                    &model,
                    req_meta,
                    json!({
                        "thinking": &thinking_buf,
                        "text": &text_buf,
                        "tool_calls": &tool_calls,
                        "usage": &usage,
                    }),
                );
            }

            total_usage.prompt_tokens += usage.prompt_tokens;
            total_usage.completion_tokens += usage.completion_tokens;
            total_usage.cache_read_tokens += usage.cache_read_tokens;
            total_usage.cost_usd += usage.cost_usd;

            if usage.prompt_tokens > 0 || usage.completion_tokens > 0 || usage.cost_usd > 0.0 {
                let _ = self
                    .store
                    .record_usage(&session_id, &cfg.provider.kind, &model, &usage);
            }

            // Live mid-turn spend-limit guard (checks BOTH global daily cap and per-subagent cost ceiling)
            let live_daily_cost = self.store.daily_cost_usd().unwrap_or(total_usage.cost_usd);
            let daily_exceeded = cfg.general.daily_spend_limit_usd > 0.0
                && total_usage.cost_usd > 0.0
                && live_daily_cost >= cfg.general.daily_spend_limit_usd;
            let turn_cap_exceeded = opts
                .max_cost_usd_override
                .filter(|cap| *cap > 0.0)
                .is_some_and(|cap| total_usage.cost_usd >= cap);
            let budget_exceeded = daily_exceeded || turn_cap_exceeded;

            let iter_duration_ms = iter_started.elapsed().as_millis() as u64;
            let thinking_ms = match (thinking_started, thinking_ended) {
                (Some(start), Some(end)) if !thinking_buf.is_empty() => {
                    end.saturating_duration_since(start).as_millis().max(100) as u64
                }
                _ => 0,
            };

            let assistant_ev = self.store.append_event(
                &session_id,
                head_id.as_deref(),
                EventRole::Assistant,
                match tool_calls.is_empty() {
                    true => EventKind::Message,
                    false => EventKind::ToolCall,
                },
                json!({
                    "text": text_buf,
                    "thinking": thinking_buf,
                    "tool_calls": tool_calls,
                    "model": model,
                    "duration_ms": iter_duration_ms,
                    "thinking_ms": thinking_ms,
                }),
                &usage,
            )?;
            head_id = Some(assistant_ev.id.clone());

            self.emit(AgentEvent::TurnCompleted {
                session_id: session_id.clone(),
                turn_id,
                event_id: assistant_ev.id,
                usage,
            });

            let doom_reason = match cfg.orchestrator.enabled && cfg.orchestrator.loop_circuit_breaker
            {
                true => detect_text_doom_loop(&text_buf)
                    .or_else(|| detect_text_doom_loop(&thinking_buf)),
                false => None,
            };

            let raw_tool_leak = text_buf.contains("<tool_call>")
                || text_buf.contains("<invoke name=");
            let was_truncated = finish_reason.as_deref() == Some("length")
                || (tool_calls.is_empty() && text_buf.trim().is_empty())
                || (tool_calls.is_empty() && raw_tool_leak);

            if !text_buf.is_empty() && !(was_truncated && raw_tool_leak) {
                final_assistant_text = text_buf.clone();
            }

            if budget_exceeded {
                // Keep session history 100% protocol-valid if the LLM emitted tool_calls on the halting iteration
                for tc in &tool_calls {
                    let skip_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Tool,
                        EventKind::ToolResult,
                        json!({
                            "tool_call_id": tc.id,
                            "tool_name": tc.name,
                            "content": "Skipped: daily spend limit reached mid-turn.",
                            "is_error": true,
                            "diff": Value::Null,
                            "duration_ms": 0,
                        }),
                        &UsageStats::default(),
                    )?;
                    head_id = Some(skip_ev.id);
                }
                let msg = match (turn_cap_exceeded, opts.max_cost_usd_override) {
                    (true, Some(cap)) => format!(
                        "Paused turn cleanly: subagent spend limit (${cap:.4}) reached mid-turn (${:.4} spent).",
                        total_usage.cost_usd
                    ),
                    _ => format!(
                        "Paused turn cleanly: daily spend limit (${:.2}) reached mid-turn (${:.4} spent today).",
                        cfg.general.daily_spend_limit_usd, live_daily_cost
                    ),
                };
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_BUDGET_GUARD.into(),
                    tier: orch.triage.tier,
                    detail: msg.clone(),
                    model: None,
                });
                let budget_ev = self.store.append_event(
                    &session_id,
                    head_id.as_deref(),
                    EventRole::Assistant,
                    EventKind::Message,
                    json!({
                        "text": msg,
                        "thinking": "",
                        "tool_calls": [],
                        "model": model,
                    }),
                    &UsageStats::default(),
                )?;
                self.emit(AgentEvent::TurnCompleted {
                    session_id: session_id.clone(),
                    turn_id: new_id(),
                    event_id: budget_ev.id,
                    usage: UsageStats::default(),
                });
                final_assistant_text = msg;
                ended_with_final_text = true;
                break;
            }

            if tool_calls.is_empty() {
                if was_truncated
                    && iter_idx + 1 < max_iters
                    && truncation_retries < ORCH_MAX_TRUNCATION_RETRIES
                {
                    truncation_retries += 1;
                    let cap = self
                        .resolve_max_output_tokens(&model)
                        .unwrap_or(ORCH_REASONING_MAX_OUTPUT_CAP)
                        .min(ORCH_REASONING_MAX_OUTPUT_CAP);
                    active_max_tokens = active_max_tokens
                        .saturating_mul(2)
                        .clamp(active_max_tokens, cap.max(active_max_tokens));
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_TRUNCATION_RECOVERY.into(),
                        tier: orch.triage.tier,
                        detail: format!(
                            "Output truncated/empty (finish_reason={:?}, text_chars={}, max_tokens={active_max_tokens}); recovering ({truncation_retries}/{ORCH_MAX_TRUNCATION_RETRIES})",
                            finish_reason,
                            text_buf.len()
                        ),
                        model: None,
                    });
                    pending_orch_hints.push(format!(
                        "<orchestrator_hint type=\"truncation_recovery\">\n\
                         Your previous response was cut off (finish_reason={:?}) or emitted an unparsed/empty payload before finishing. \
                         Keep your internal reasoning brief and keep JSON tool arguments compact (short bullets <= 15 words), then immediately emit your tool call or complete final answer NOW.\n\
                         </orchestrator_hint>",
                        finish_reason.as_deref().unwrap_or("empty")
                    ));
                    continue;
                }
                if let Some(reason) = doom_reason {
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_CIRCUIT_BREAKER.into(),
                        tier: orch.triage.tier,
                        detail: reason.clone(),
                        model: None,
                    });
                    if iter_idx + 1 < max_iters && orch.guard.pivot_count() == 0 {
                        orch.guard.record_text_doom_pivot();
                        let (next_model, next_effort, src) = escalate_on_pivot(
                            cfg,
                            &model,
                            reasoning_effort,
                            orch.has_explicit_model_override,
                        );
                        if next_model != model {
                            context_window = self.resolve_context_window(&next_model);
                        }
                        model = next_model;
                        reasoning_effort = next_effort;
                        pending_orch_hints.push(format!(
                            "<orchestrator_hint type=\"text_doom_loop\">\n\
                             Repetitive output detected ({reason}; escalated={src}). Provide a concise, non-repetitive final answer or concrete next tool call.\n\
                             </orchestrator_hint>"
                        ));
                        continue;
                    }
                    ended_with_final_text = true;
                    break;
                }
                let trailing_steers = self.drain_steering(&session_id);
                if !trailing_steers.is_empty() {
                    for steer_text in trailing_steers {
                        let steer_ev = self.store.append_event(
                            &session_id,
                            head_id.as_deref(),
                            EventRole::User,
                            EventKind::Message,
                            json!({
                                "text": format!("[Live Steering Update] {steer_text}"),
                                "steering": true,
                            }),
                            &UsageStats::default(),
                        )?;
                        head_id = Some(steer_ev.id);
                    }
                    continue;
                }
                if iter_idx + 1 < max_iters {
                    if let Some(ref schema) = opts.output_schema
                        && schema_retries < SUBAGENT_SCHEMA_MAX_RETRIES {
                            let schema_err = match extract_json_for_schema(&final_assistant_text, schema) {
                                Some(parsed) => match validate_json_schema(schema, &parsed) {
                                    Ok(()) => None,
                                    Err(errs) => Some(errs.join("; ")),
                                },
                                None => Some("Response did not contain a valid JSON value.".into()),
                            };
                            if let Some(reason) = schema_err {
                                schema_retries += 1;
                                pending_orch_hints.push(format!(
                                    "<orchestrator_hint type=\"schema_repair\">\n\
                                     Your final reply violated the required JSON Schema contract ({reason}). \
                                     Respond immediately with strictly valid JSON matching the schema and zero extra prose.\n\
                                     </orchestrator_hint>"
                                ));
                                continue;
                            }
                        }
                    if active_role.is_none() && !effective_permission.is_read_only_plan()
                        && let Some(nudge) = orch.guard.check_completion_gate(
                            cfg.orchestrator.enabled && cfg.orchestrator.verification_gate,
                        ) {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_VERIFICATION_GATE.into(),
                                tier: orch.triage.tier,
                                detail: format!(
                                    "Completion/verification gate active (phase={}, modified={})",
                                    orch.guard.phase().as_str(),
                                    orch.guard.mutated_files_count()
                                ),
                                model: None,
                            });
                            pending_orch_hints.push(nudge);
                            continue;
                        }
                }
                ended_with_final_text = true;
                break;
            }

            // Pre-tool deterministic Role Policy Guard (subagent allowlist/denylist) +
            // Phase Gate (`Recon -> Plan -> Execute` on Orchestrated turns) & Read-Only Intent Gate.
            let mut allowed_indexed: Vec<(usize, ToolCall)> = Vec::with_capacity(tool_calls.len());
            let mut redirected_indexed: Vec<(usize, ExecutedToolCall)> = Vec::new();
            let mut phase_gate_event: Option<(&'static str, String, String)> = None;
            let mut saw_phase_gate_redirect = false;

            for (idx, mut tc) in tool_calls.into_iter().enumerate() {
                // Schema-guided argument coercion & `_raw` JSON repair runs BEFORE role/policy
                // inspection and wave lock classification so every downstream check sees clean args.
                tc.arguments = self.coerce_tool_call_args(&tc.name, tc.arguments);

                if let Err(denied) = session_scope.permits_invocation(&tc.name, &tc.arguments) {
                    redirected_indexed.push((
                        idx,
                        ExecutedToolCall {
                            tool_call_id: tc.id,
                            tool_name: tc.name.clone(),
                            arguments: tc.arguments,
                            output: ToolOutput::err(denied),
                            duration_ms: 0,
                        },
                    ));
                    continue;
                }
                match active_role.is_none() && cfg.orchestrator.enabled && cfg.orchestrator.phase_gating {
                    true => match orch
                        .guard
                        .check_pre_tool_phase_gate(true, &tc.name, &tc.arguments)
                    {
                        PhaseGateDecision::Allow => allowed_indexed.push((idx, tc)),
                        PhaseGateDecision::Redirect {
                            stage,
                            reason,
                            hint,
                        } => {
                            if stage == ORCH_STAGE_PHASE_GATE {
                                saw_phase_gate_redirect = true;
                            }
                            if phase_gate_event.is_none() {
                                phase_gate_event = Some((stage, reason.clone(), hint));
                            }
                            let tool_err = match stage {
                                ORCH_STAGE_INTENT_GATE => format!(
                                    "Read-only intent gate: {reason}. Answer strictly via read-only inspection without mutating files or state."
                                ),
                                _ => format!(
                                    "Phase gate redirect: {reason}. Inspect files with read-only tools or call `record_brainstorm`/`plan_update` first."
                                ),
                            };
                            redirected_indexed.push((
                                idx,
                                ExecutedToolCall {
                                    tool_call_id: tc.id,
                                    tool_name: tc.name,
                                    arguments: tc.arguments,
                                    output: ToolOutput::err(tool_err),
                                    duration_ms: 0,
                                },
                            ));
                        }
                    },
                    false => allowed_indexed.push((idx, tc)),
                }
            }

            if saw_phase_gate_redirect {
                orch.guard.record_phase_gate_nudge();
            }

            if let Some((stage, reason, hint)) = phase_gate_event {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: stage.into(),
                    tier: orch.triage.tier,
                    detail: reason,
                    model: None,
                });
                pending_orch_hints.push(hint);
            }

            let total_wave_calls = allowed_indexed.len() + redirected_indexed.len();
            tool_calls_count += allowed_indexed.len();

            // Phase 6 Lazy Pre-Mutation Workspace Checkpoint:
            // Captures at most once per turn right before the first workspace-mutating tool call
            // (file edits, non-read-only shell, worktree merge, or inherited mutating subagent).
            if cfg.checkpoints.enabled
                && turn_checkpoint_id.is_none()
                && effective_permission != PermissionMode::Plan
            {
                let mutating_trigger = allowed_indexed
                    .iter()
                    .find_map(|(_, tc)| is_workspace_mutating_call(tc));
                if let Some(trigger_tool) = mutating_trigger {
                    match self
                        .checkpoints
                        .capture_checkpoint(
                            &cfg.checkpoints,
                            &session_id,
                            &format!("turn_{}", self.turn_seq.load(Ordering::Relaxed)),
                            turn_checkpoint_event_id.clone(),
                            &tool_ctx.workspace_dir,
                            trigger_tool,
                            turn_checkpoint_label.clone(),
                            false,
                        )
                        .await
                    {
                        Ok(cp) => {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_CHECKPOINT.into(),
                                tier: orch.triage.tier,
                                detail: format!(
                                    "Saved pre-mutation checkpoint {} ({}) before `{trigger_tool}`",
                                    &cp.id[..cp.id.len().min(8)],
                                    cp.backend.as_str()
                                ),
                                model: None,
                            });
                            *turn_checkpoint_id = Some(cp.id);
                        }
                        Err(e) => {
                            warn!(error = %e, "pre-mutation workspace checkpoint skipped");
                        }
                    }
                }
            }

            let executed_allowed: Vec<ExecutedToolCall> = match allowed_indexed.is_empty() {
                true => Vec::new(),
                false => {
                    let calls_to_run: Vec<ToolCall> =
                        allowed_indexed.iter().map(|(_, tc)| tc.clone()).collect();
                    match execute_tool_waves(
                        calls_to_run,
                        MAX_PARALLEL_TOOLS,
                        |tc| self.classify_tool_call(cfg, effective_permission, tc),
                        |tc| {
                            let sid = session_id.clone();
                            let ctx_ref = tool_ctx.clone();
                            let cancel_ref = cancel.clone();
                            async move {
                                self.execute_single_tool_call(
                                    cfg,
                                    &sid,
                                    effective_permission,
                                    &ctx_ref,
                                    opts.headless_no_wait,
                                    tc,
                                    &cancel_ref,
                                )
                                .await
                            }
                        },
                    )
                    .await
                    {
                        Ok(results) => results,
                        Err(err) => {
                            // Keep SQLite protocol-valid: every emitted `Assistant(tool_calls)` MUST
                            // have matching `Tool(ToolResult)` events so future turns never 400!
                            let abort_msg = match err {
                                ZenkoError::Cancelled => {
                                    "Skipped: tool execution cancelled by user.".to_string()
                                }
                                ref other => format!("Skipped: tool wave aborted ({other})."),
                            };
                            for (_, tc) in &allowed_indexed {
                                if let Ok(skip_ev) = self.store.append_event(
                                    &session_id,
                                    head_id.as_deref(),
                                    EventRole::Tool,
                                    EventKind::ToolResult,
                                    json!({
                                        "tool_call_id": tc.id,
                                        "tool_name": tc.name,
                                        "content": abort_msg,
                                        "is_error": true,
                                        "diff": Value::Null,
                                        "duration_ms": 0,
                                    }),
                                    &UsageStats::default(),
                                ) {
                                    head_id = Some(skip_ev.id);
                                }
                            }
                            for (_, redir) in &redirected_indexed {
                                if let Ok(skip_ev) = self.store.append_event(
                                    &session_id,
                                    head_id.as_deref(),
                                    EventRole::Tool,
                                    EventKind::ToolResult,
                                    json!({
                                        "tool_call_id": redir.tool_call_id,
                                        "tool_name": redir.tool_name,
                                        "content": redir.output.content,
                                        "is_error": true,
                                        "diff": Value::Null,
                                        "duration_ms": 0,
                                    }),
                                    &UsageStats::default(),
                                ) {
                                    head_id = Some(skip_ev.id);
                                }
                            }
                            return Err(err);
                        }
                    }
                }
            };

            let mut ordered_results: Vec<Option<(ExecutedToolCall, bool)>> =
                (0..total_wave_calls).map(|_| None).collect();
            for ((idx, _), exec) in allowed_indexed.into_iter().zip(executed_allowed) {
                ordered_results[idx] = Some((exec, false));
            }
            for (idx, redir) in redirected_indexed {
                ordered_results[idx] = Some((redir, true));
            }

            let mut escalate_trigger: Option<(String, String)> = None;
            let mut wave_had_plan_or_mutation = false;

            for (done, was_phase_redirect) in ordered_results.into_iter().flatten() {
                if !was_phase_redirect && !done.output.is_error {
                    let cat = orchestrator::classify_tool_category(&done.tool_name, &done.arguments);
                    if matches!(
                        cat,
                        orchestrator::ToolCategory::Planning | orchestrator::ToolCategory::Mutation
                    ) {
                        wave_had_plan_or_mutation = true;
                    }
                }
                // Unwrap KV-cache-safe `invoke_tool` proxy so memory deltas, plan updates,
                // and structured branch notes persist identically for deferred tools.
                let (eff_tool_name, eff_tool_args) = match done.tool_name.as_str() {
                    TOOL_INVOKE_TOOL => {
                        let inner_n = done
                            .arguments
                            .get("tool")
                            .or_else(|| done.arguments.get("name"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        match inner_n {
                            Some(n) => (
                                n,
                                done.arguments
                                    .get("args")
                                    .or_else(|| done.arguments.get("arguments"))
                                    .unwrap_or(&Value::Null),
                            ),
                            None => (done.tool_name.as_str(), &done.arguments),
                        }
                    }
                    other => (other, &done.arguments),
                };

                if !was_phase_redirect && eff_tool_name == TOOL_MEMORY && !done.output.is_error {
                    let act = eff_tool_args
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if is_memory_mutating_action(act) {
                        self.record_memory_delta(
                            &session_id,
                            compact_preview(&done.output.content, 160),
                        );
                    }
                }
                if !was_phase_redirect && eff_tool_name == TOOL_PLAN_UPDATE && !done.output.is_error
                    && let Some(plan) = done
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|v| serde_json::from_value::<ExecutionPlan>(v.clone()).ok())
                    {
                        orch.guard.update_active_plan(plan);
                    }
                if !was_phase_redirect
                    && eff_tool_name == TOOL_SESSION_TASK
                    && !done.output.is_error
                    && let Ok(live_tasks) = self.clarifications.list_session_tasks(&session_id)
                        && let Some(plan) =
                            execution_plan_from_session_tasks(&session.title, &live_tasks)
                        {
                            orch.guard.update_active_plan(plan);
                        }

                if !was_phase_redirect
                    && cfg.orchestrator.enabled
                    && cfg.orchestrator.loop_circuit_breaker
                {
                    match orch.guard.record_tool_outcome(
                        &done.tool_name,
                        &done.arguments,
                        done.output.is_error,
                        &done.output.content,
                        done.output.metadata.as_ref(),
                    ) {
                        GuardVerdict::Continue => {}
                        GuardVerdict::Pivot { reason, hint } => {
                            let (next_model, next_effort, src) = escalate_on_pivot(
                                cfg,
                                &model,
                                reasoning_effort,
                                orch.has_explicit_model_override,
                            );
                            let switched_model = match next_model != model {
                                true => {
                                    context_window = self.resolve_context_window(&next_model);
                                    Some(next_model.clone())
                                }
                                false => None,
                            };
                            model = next_model;
                            reasoning_effort = next_effort;
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_CIRCUIT_BREAKER.into(),
                                tier: orch.triage.tier,
                                detail: format!("{reason} ({src})"),
                                model: switched_model,
                            });
                            pending_orch_hints.push(hint);
                        }
                        GuardVerdict::Escalate { reason, summary } => {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_ESCALATED.into(),
                                tier: orch.triage.tier,
                                detail: summary.clone(),
                                model: None,
                            });
                            escalate_trigger = Some((reason, summary));
                        }
                    }
                }

                let ev = self.store.append_event(
                    &session_id,
                    head_id.as_deref(),
                    EventRole::Tool,
                    EventKind::ToolResult,
                    json!({
                        "tool_call_id": done.tool_call_id,
                        "tool_name": done.tool_name,
                        "content": done.output.content,
                        "is_error": done.output.is_error,
                        "diff": done.output.diff,
                        "duration_ms": done.duration_ms,
                    }),
                    &UsageStats::default(),
                )?;
                head_id = Some(ev.id);

                // Persist structured orchestrator artifacts strictly onto the active branch
                let note_payload = match (
                    was_phase_redirect,
                    eff_tool_name,
                    done.output.is_error,
                ) {
                    (false, TOOL_RECORD_BRAINSTORM, false) => {
                        done.output.metadata.as_ref().map(|spec| {
                            json!({
                                "note_kind": NOTE_KIND_BRAINSTORM_SPEC,
                                "spec": spec,
                            })
                        })
                    }
                    (false, TOOL_PLAN_UPDATE, false) => done.output.metadata.as_ref().map(|plan| {
                        json!({
                            "note_kind": NOTE_KIND_EXECUTION_PLAN,
                            "plan": plan,
                        })
                    }),
                    (false, TOOL_ASK_CLARIFICATION, false) => {
                        done.output.metadata.as_ref().map(|dec| {
                            json!({
                                "note_kind": NOTE_KIND_CLARIFICATION_DECISION,
                                "decision": dec,
                            })
                        })
                    }
                    (false, TOOL_ORCHESTRATE_CONTROL, false) => {
                        done.output.metadata.as_ref().map(|ctrl| {
                            json!({
                                "note_kind": NOTE_KIND_ORCHESTRATE_CONTROL,
                                "control": ctrl,
                            })
                        })
                    }
                    (false, TOOL_TOOL_SEARCH, false) => done
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("activated_tools"))
                        .and_then(Value::as_array)
                        .filter(|arr| !arr.is_empty())
                        .map(|arr| {
                            json!({
                                "note_kind": NOTE_KIND_DEFERRED_TOOLS,
                                "tools": arr,
                            })
                        }),
                    (false, name, false) if self.tools.is_deferred(name) => Some(json!({
                        "note_kind": NOTE_KIND_DEFERRED_TOOLS,
                        "tools": [name],
                    })),
                    _ => None,
                };
                if let Some(payload) = note_payload {
                    let note_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::System,
                        EventKind::SystemNote,
                        payload,
                        &UsageStats::default(),
                    )?;
                    head_id = Some(note_ev.id);
                }
            }

            if let Some((reason, summary)) = escalate_trigger {
                let halt_message: Option<String> =
                    match opts.headless_no_wait || iter_idx + 1 >= max_iters {
                        true => Some(format!(
                            "Stopped cleanly after circuit-breaker escalation: {summary}"
                        )),
                    false => {
                        let ans = self
                            .clarifications
                            .request_clarification(
                                &session_id,
                                &format!("Execution loop detected: {reason}. How should Zenko proceed?"),
                                &summary,
                                vec![
                                    ClarificationOption {
                                        id: ESCALATION_OPT_PIVOT.into(),
                                        label: "Pivot strategy & retry".into(),
                                        description: "Try a fundamentally different approach or inspect ground truth.".into(),
                                        recommended: true,
                                    },
                                    ClarificationOption {
                                        id: ESCALATION_OPT_STOP.into(),
                                        label: "Stop here & report".into(),
                                        description: "Halt tool execution and summarize the blocker.".into(),
                                        recommended: false,
                                    },
                                ],
                                false,
                                cancel.clone(),
                            )
                            .await?;
                        match ans.selected_option_id.as_str() {
                            ESCALATION_OPT_STOP => Some(format!(
                                "Halted by user after loop escalation: {summary}"
                            )),
                            _ => {
                                orch.guard.reset_after_escalation();
                                let note = ans.custom_answer.unwrap_or_default();
                                pending_orch_hints.push(format!(
                                    "<orchestrator_hint type=\"escalation_guidance\">\n\
                                     User chose `{}` after loop escalation ({reason}). {note} Change strategy fundamentally.\n\
                                     </orchestrator_hint>",
                                    ans.selected_option_id
                                ));
                                None
                            }
                        }
                    }
                };
                if let Some(msg) = halt_message {
                    let halt_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Assistant,
                        EventKind::Message,
                        json!({
                            "text": msg,
                            "thinking": "",
                            "tool_calls": [],
                            "model": model,
                        }),
                        &UsageStats::default(),
                    )?;
                    self.emit(AgentEvent::TurnCompleted {
                        session_id: session_id.clone(),
                        turn_id: new_id(),
                        event_id: halt_ev.id,
                        usage: UsageStats::default(),
                    });
                    final_assistant_text = msg;
                    ended_with_final_text = true;
                    break;
                }
            }

            if let Some((detail, hint)) = orch.guard.check_iteration_pacing(
                cfg.orchestrator.enabled,
                iter_idx,
                max_iters,
                wave_had_plan_or_mutation,
            ) {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_PACING.into(),
                    tier: orch.triage.tier,
                    detail,
                    model: None,
                });
                pending_orch_hints.push(hint);
            }
        }

        // Tool-free final wrap-up synthesis pass if max_iters ended on a tool-calling turn,
        // preventing stale intermediate filler prose from being returned as the final answer.
        if !ended_with_final_text && !cancel.is_cancelled() {
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_SYNTHESIS.into(),
                tier: orch.triage.tier,
                detail: format!("Max tool iterations ({max_iters}) reached; running tool-free wrap-up synthesis"),
                model: Some(model.clone()),
            });
            let branch_events = self.store.get_active_branch(&session_id)?;
            let ckpt_slice = slice_from_latest_checkpoint(&branch_events);
            let mut raw_messages = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
            if let Some(ref ckpt) = ckpt_slice.checkpoint {
                prepend_checkpoint_to_messages(&mut raw_messages, ckpt);
            }
            let (mc_messages, _) = microcompact_messages(
                raw_messages,
                context_window,
                effective_sys_tokens,
                cfg.context.microcompact_trigger_ratio,
            );
            let mut messages = compact_messages_to_budget(
                mc_messages,
                context_window,
                effective_sys_tokens,
            );
            append_cache_safe_reminder(
                &mut messages,
                format!(
                    "<orchestrator_hint type=\"final_synthesis\">\n\
                     Tool iteration budget ({max_iters}) reached after {tool_calls_count} tool call(s). Tools are now disabled for this wrap-up turn. \
                     Synthesize a complete, clear final answer directly from all evidence and tool results gathered above.\n\
                     </orchestrator_hint>"
                ),
            );
            let effective_effort = match (reasoning_effort, self.resolve_supports_reasoning(&model)) {
                (Some(_), Some(false)) => None,
                (eff, _) => eff,
            };
            let synth_req = ChatRequest {
                session_id: session_id.clone(),
                model: model.clone(),
                fallbacks: cfg.provider.fallbacks.clone(),
                system_prompt: effective_system_prompt.clone(),
                messages,
                tools: Vec::new(),
                temperature: cfg.provider.temperature,
                max_tokens: active_max_tokens,
                enable_streaming: cfg.provider.enable_streaming,
                enable_prompt_caching: cfg.provider.enable_prompt_caching,
                sticky_session_routing: cfg.provider.sticky_session_routing,
                provider_sort: cfg.provider.provider_sort.clone(),
                reasoning_effort: effective_effort,
            };
            if let Ok(mut stream) = self.provider.stream_chat(synth_req, cancel.clone()).await {
                let synth_turn_id = new_id();
                let synth_started = std::time::Instant::now();
                let mut synth_think_start: Option<std::time::Instant> = None;
                let mut synth_think_end: Option<std::time::Instant> = None;
                let mut synth_thinking = String::new();
                let mut synth_text = String::new();
                let mut synth_usage = UsageStats::default();
                let mut synth_stream_err: Option<String> = None;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(delta) => match delta {
                            LlmDelta::Thinking(t) => {
                                let now = std::time::Instant::now();
                                synth_think_start.get_or_insert(now);
                                synth_think_end = Some(now);
                                synth_thinking.push_str(&t);
                                self.emit(AgentEvent::ThinkingDelta {
                                    session_id: session_id.clone(),
                                    turn_id: synth_turn_id.clone(),
                                    text: t,
                                });
                            }
                            LlmDelta::Text(t) => {
                                synth_text.push_str(&t);
                                self.emit(AgentEvent::TextDelta {
                                    session_id: session_id.clone(),
                                    turn_id: synth_turn_id.clone(),
                                    text: t,
                                });
                            }
                            LlmDelta::Usage(u) => synth_usage = u,
                            LlmDelta::ToolCallComplete(_) | LlmDelta::Done { .. } => {}
                        },
                        Err(err) => {
                            synth_stream_err = Some(err.to_string());
                            break;
                        }
                    }
                }
                total_usage.prompt_tokens += synth_usage.prompt_tokens;
                total_usage.completion_tokens += synth_usage.completion_tokens;
                total_usage.cache_read_tokens += synth_usage.cache_read_tokens;
                total_usage.cost_usd += synth_usage.cost_usd;
                if synth_usage.prompt_tokens > 0
                    || synth_usage.completion_tokens > 0
                    || synth_usage.cost_usd > 0.0
                {
                    let _ = self.store.record_usage(
                        &session_id,
                        &cfg.provider.kind,
                        &model,
                        &synth_usage,
                    );
                }
                if let Some(err_msg) = synth_stream_err {
                    warn!(
                        session_id = %session_id,
                        error = %err_msg,
                        "wrap-up synthesis stream failed; skipping partial assistant persistence"
                    );
                } else if !cancel.is_cancelled() && !synth_text.trim().is_empty() {
                    let synth_duration_ms = synth_started.elapsed().as_millis() as u64;
                    let synth_thinking_ms = match (synth_think_start, synth_think_end) {
                        (Some(start), Some(end)) if !synth_thinking.is_empty() => {
                            end.saturating_duration_since(start).as_millis().max(100) as u64
                        }
                        _ => 0,
                    };
                    let synth_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Assistant,
                        EventKind::Message,
                        json!({
                            "text": synth_text,
                            "thinking": synth_thinking,
                            "tool_calls": [],
                            "model": model,
                            "duration_ms": synth_duration_ms,
                            "thinking_ms": synth_thinking_ms,
                        }),
                        &synth_usage,
                    )?;
                    self.emit(AgentEvent::TurnCompleted {
                        session_id: session_id.clone(),
                        turn_id: synth_turn_id,
                        event_id: synth_ev.id,
                        usage: synth_usage,
                    });
                    final_assistant_text = synth_text;
                }
            }
            if final_assistant_text.trim().is_empty() {
                final_assistant_text = format!(
                    "Reached max tool iterations ({max_iters}) after {tool_calls_count} tool calls without a final summary."
                );
            }
        }

        let _ = run_hooks(
            &cfg.hooks,
            HookEvent::Stop,
            None,
            json!({ "session_id": session_id }),
        )
        .await;

        if cfg.memory.dream_enabled && session.parent_session_id.is_none() {
            self.dream_notify.notify_one();
        }

        Ok(TurnResult {
            session_id,
            reply: final_assistant_text,
            model,
            usage: total_usage,
            tool_calls_count,
        })
    }

    pub(super) fn classify_tool_call(
        &self,
        cfg: &ZenkoConfig,
        effective_permission: PermissionMode,
        tc: &ToolCall,
    ) -> ToolConcurrency {
        tool_router::classify_tool_call(self, cfg, effective_permission, tc)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_single_tool_call(
        &self,
        cfg: &ZenkoConfig,
        session_id: &str,
        effective_permission: PermissionMode,
        tool_ctx: &ToolContext,
        headless_no_wait: bool,
        tc: ToolCall,
        cancel: &CancellationToken,
    ) -> Result<ExecutedToolCall> {
        tool_router::dispatch_tool_call(
            self,
            cfg,
            session_id,
            effective_permission,
            tool_ctx,
            headless_no_wait,
            tc,
            cancel,
        )
        .await
    }

    pub(crate) async fn request_hitl_approval(
        &self,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        args_preview: &str,
        risk: zenko_core::RiskLevel,
        cancel: &CancellationToken,
    ) -> Result<ApprovalDecision> {
        let approval_id = new_id();
        let record = ApprovalRecord {
            id: approval_id.clone(),
            session_id: session_id.to_string(),
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            args_preview: args_preview.to_string(),
            risk,
            status: ApprovalStatus::Pending,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        };

        self.store.insert_approval(&record)?;
        self.store
            .update_session_status(session_id, SessionStatus::WaitingApproval)?;
        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status: SessionStatus::WaitingApproval,
            title: None,
        });

        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending_approvals.lock() {
            map.insert(approval_id.clone(), (session_id.to_string(), tx));
        }

        self.emit(AgentEvent::ToolApprovalRequested { approval: record });

        let decision = tokio::select! {
            _ = cancel.cancelled() => {
                if let Ok(mut map) = self.pending_approvals.lock() {
                    map.remove(&approval_id);
                }
                let _ = self.store.resolve_approval(&approval_id, ApprovalDecision::Deny, APPROVAL_RESOLVER_CANCELLED);
                return Err(ZenkoError::Cancelled);
            }
            res = tokio::time::timeout(Duration::from_secs(DEFAULT_APPROVAL_TIMEOUT_SECS), rx) => {
                match res {
                    Ok(Ok(d)) => d,
                    _ => {
                        if let Ok(mut map) = self.pending_approvals.lock() {
                            map.remove(&approval_id);
                        }
                        let _ = self.store.resolve_approval(&approval_id, ApprovalDecision::Deny, APPROVAL_RESOLVER_TIMEOUT);
                        ApprovalDecision::Deny
                    }
                }
            }
        };

        self.store
            .update_session_status(session_id, SessionStatus::Running)?;
        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status: SessionStatus::Running,
            title: None,
        });

        Ok(decision)
    }

}
