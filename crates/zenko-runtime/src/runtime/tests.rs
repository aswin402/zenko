    use super::*;

    #[test]
    fn steering_injects_only_while_session_active_without_cancelling() {
        let dir = std::env::temp_dir().join(format!("zenko_rt_steer_{}", new_id()));
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("build runtime");

        // Idle session cannot be steered mid-flight
        assert!(!rt.steer_session("s_active", "focus on tests"));

        // Register an active token simulating an in-flight turn
        let tok = CancellationToken::new();
        rt.active_cancels
            .lock()
            .expect("lock")
            .insert("s_active".into(), (1, tok.clone()));

        assert!(rt.steer_session("s_active", "focus on unit tests first"));
        assert!(!tok.is_cancelled(), "steering must NEVER cancel an active turn");
        let drained = rt.drain_steering("s_active");
        assert_eq!(drained, vec!["focus on unit tests first"]);
        assert!(rt.drain_steering("s_active").is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn durable_sqlite_followup_queue_and_3state_tool_classification() {
        let dir = std::env::temp_dir().join(format!("zenko_rt_queue_{}", new_id()));
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("build runtime");

        let session = rt
            .store
            .create_session("Queue Test", "cli", None, None, PermissionMode::Balanced)
            .expect("create session");

        // 1. Durable SQLite follow-up queue enqueue / list / cancel / pop FIFO
        let (q1, c1) = rt
            .enqueue_session_followup(&session.id, "first follow-up")
            .expect("enqueue 1");
        let (q2, c2) = rt
            .enqueue_session_followup(&session.id, "second follow-up")
            .expect("enqueue 2");
        let (q3, c3) = rt
            .enqueue_session_followup(&session.id, "third follow-up")
            .expect("enqueue 3");
        assert_eq!((c1, c2, c3), (1, 2, 3));

        let (cancelled, rem) = rt
            .cancel_session_followup(&session.id, &q2.id)
            .expect("cancel 2");
        assert!(cancelled);
        assert_eq!(rem, 2);

        let popped1 = rt
            .store
            .pop_next_followup(&session.id)
            .expect("pop 1")
            .expect("q1");
        assert_eq!(popped1.id, q1.id);
        let popped2 = rt
            .store
            .pop_next_followup(&session.id)
            .expect("pop 2")
            .expect("q3");
        assert_eq!(popped2.id, q3.id);
        assert!(rt.store.pop_next_followup(&session.id).expect("pop empty").is_none());

        // 2. 3-state concurrency classification respects permission policy
        let cfg_now = rt.config();
        let write_a = ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            arguments: json!({"path": "./src/a.rs", "content": "x"}),
        };
        let write_b = ToolCall {
            id: "c2".into(),
            name: "write_file".into(),
            arguments: json!({"path": "src/b.rs", "content": "y"}),
        };
        let read_a = ToolCall {
            id: "c3".into(),
            name: "read_file".into(),
            arguments: json!({"path": "src/a.rs"}),
        };
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &read_a),
            ToolConcurrency::Shared
        );
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &write_a),
            ToolConcurrency::Keyed(zenko_core::ResourceLockKey::File("src/a.rs".into()))
        );
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &write_b),
            ToolConcurrency::Keyed(zenko_core::ResourceLockKey::File("src/b.rs".into()))
        );
        // Strict mode forces HITL approval -> downgrades writes to Global barrier
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Strict, &write_a),
            ToolConcurrency::Global
        );

        // 3. Per-goal async lock returns the same Arc mutex for identical goal IDs and distinct locks across goals
        let lock_a1 = rt.goal_lock("goal_alpha");
        let lock_a2 = rt.goal_lock("goal_alpha");
        let lock_b = rt.goal_lock("goal_beta");
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2));
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b));
        let _guard = lock_a1.try_lock().expect("first lock acquires");
        assert!(
            lock_a2.try_lock().is_err(),
            "concurrent acquisition on same goal_id must serialize"
        );
        assert!(
            lock_b.try_lock().is_ok(),
            "different goal_id must run independently without blocking"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auto_dispatch_queues_foreground_and_steers_during_bg_or_subagents() {
        // Pure state-machine table check (zero I/O)
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: false,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::StartTurn
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Queue
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 1,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Steer
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 2,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Steer
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: true,
            }),
            AutoDispatchDecision::Steer
        );

        // Runtime integration check with active-tool RAII guard + running session map
        let dir = std::env::temp_dir().join(format!("zenko_autodisp_{}", new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("runtime");
        let sess = rt
            .store
            .create_session("Auto Dispatch", "webui", None, None, PermissionMode::Autonomous)
            .expect("session");

        // Idle -> StartTurn
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::StartTurn
        );

        // Mark running (foreground work) -> Queue
        rt.active_cancels
            .lock()
            .expect("lock")
            .insert(sess.id.clone(), (1, CancellationToken::new()));
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::Queue
        );

        // Monitoring tool active (`job_wait`) -> Steer, then automatically reverts to Queue on Drop!
        {
            let _guard = ActiveToolGuard::enter(
                rt.session_control(&sess.id),
                zenko_core::TOOL_JOB_WAIT.to_string(),
            );
            assert_eq!(
                rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
                AutoDispatchDecision::Steer
            );
        }
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::Queue
        );

        // Promote a queued follow-up directly into live steering
        let (queued, count) = rt
            .enqueue_session_followup(&sess.id, "switch to async")
            .expect("enqueue");
        assert_eq!(count, 1);
        let (promoted, remaining) = rt
            .promote_queued_to_steer(&sess.id, &queued.id)
            .expect("promote");
        assert!(promoted);
        assert_eq!(remaining, 0);
        assert_eq!(rt.drain_steering(&sess.id), vec!["switch to async".to_string()]);

        rt.active_cancels.lock().expect("lock").remove(&sess.id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn sovereign_orchestrate_control_shifts_gear_and_refreshes_recon_live() {
        let dir = std::env::temp_dir().join(format!("zenko_orch_ctrl_{}", new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").expect("write");

        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.join("data").display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        cfg.provider.lead_model = "slot/lead".into();
        cfg.provider.advisor_model = "slot/advisor".into();

        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("runtime");
        assert!(
            rt.tools.get(TOOL_ORCHESTRATE_CONTROL).is_some(),
            "orchestrate_control must be registered eagerly"
        );
        let sess = rt
            .store
            .create_session("Control Test", "webui", None, None, PermissionMode::Autonomous)
            .expect("session");

        // 1. Unlocked session resolves configured advisor slot + returns fresh workspace recon
        let receipt = rt
            .clarifications
            .apply_orchestrate_control(
                &sess.id,
                &dir,
                OrchestrateControlRequest {
                    posture: Some(TurnPosture::Brainstorm),
                    cognitive_tier: Some(ModelRoutingRole::Advisor),
                    reasoning_effort: Some(ReasoningEffort::High),
                    zmode: None,
                    concise_mode: None,
                    minimalism_mode: Some(CodeMinimalismMode::Full),
                    refresh_recon: true,
                    recon_query: Some("Cargo.toml".into()),
                    rationale: Some("Deep architecture comparison".into()),
                },
            )
            .await
            .expect("apply control");
        assert_eq!(receipt.posture, Some(TurnPosture::Brainstorm));
        assert_eq!(receipt.resolved_model.as_deref(), Some("slot/advisor"));
        assert!(!receipt.model_locked_by_override);
        assert!(
            receipt
                .recon_summary
                .as_deref()
                .is_some_and(|s| s.contains("Cargo.toml")),
            "expected live recon to include Cargo.toml, got {:?}",
            receipt.recon_summary
        );

        let pending = rt.take_pending_control(&sess.id).expect("pending shift");
        assert_eq!(pending.posture, Some(TurnPosture::Brainstorm));
        assert_eq!(pending.cognitive_tier, Some(ModelRoutingRole::Advisor));
        assert_eq!(pending.reasoning_effort, Some(ReasoningEffort::High));
        assert!(rt.take_pending_control(&sess.id).is_none());

        // 2. Explicit model override lock prevents model swap while still allowing posture/effort shifts
        rt.session_control(&sess.id)
            .explicit_model_locked
            .store(true, Ordering::Relaxed);
        let locked_receipt = rt
            .clarifications
            .apply_orchestrate_control(
                &sess.id,
                &dir,
                OrchestrateControlRequest {
                    posture: Some(TurnPosture::Review),
                    cognitive_tier: Some(ModelRoutingRole::Fast),
                    reasoning_effort: Some(ReasoningEffort::Low),
                    zmode: Some(zenko_core::ZMode::High),
                    concise_mode: None,
                    minimalism_mode: None,
                    refresh_recon: false,
                    recon_query: None,
                    rationale: None,
                },
            )
            .await
            .expect("apply locked control");
        assert!(locked_receipt.model_locked_by_override);
        assert!(locked_receipt.resolved_model.is_none());
        assert_eq!(locked_receipt.reasoning_effort, Some(ReasoningEffort::Low));
        assert_eq!(locked_receipt.zmode, Some(zenko_core::ZMode::High));
        let cfg_now = rt.config();
        assert_eq!(rt.effective_zmode(&sess.id, &cfg_now, &[]), zenko_core::ZMode::High);
        assert_eq!(
            rt.effective_concise_mode(&sess.id, &cfg_now, &[]),
            zenko_core::ConciseMode::Full
        );
        assert_eq!(
            rt.effective_minimalism_mode(&sess.id, &cfg_now, &[]),
            CodeMinimalismMode::Full
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn minicode_tool_v2_self_healing_scoped_catalog_and_policy_safe_batch() {
        let dir = std::env::temp_dir().join(format!("zenko_toolv2_{}", new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");
        std::fs::write(
            dir.join("src/app.rs"),
            "pub struct App {}\nimpl App {\n    pub fn boot() -> usize { 42 }\n}\n",
        )
        .expect("write");

        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.join("data").display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("runtime");
        let cfg_now = rt.config();

        let sess = rt
            .store
            .create_session("ToolV2", "cli", None, None, PermissionMode::Autonomous)
            .expect("session");
        let tool_ctx = ToolContext {
            session_id: sess.id.clone(),
            workspace_dir: dir.clone(),
            data_dir: dir.join("data"),
            hub_dir: dir.join("hub"),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 16_384,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
            file_state: Some(rt.file_state.clone() as Arc<dyn FileStateBridge>),
        };
        let cancel = CancellationToken::new();

        // 1. Unknown tool returns fuzzy permitted suggestions.
        let typo = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t1".into(),
                    name: "readfile".into(),
                    arguments: json!({}),
                },
                &cancel,
            )
            .await
            .expect("typo");
        assert!(typo.output.is_error);
        assert!(
            typo.output.content.contains("`read_file`"),
            "missing fuzzy suggestion: {}",
            typo.output.content
        );

        // 2. Malformed `_raw` JSON + camelCase + stringified integer coerces cleanly & executes.
        let coerced = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t2".into(),
                    name: "read_file".into(),
                    arguments: json!({
                        "_raw": "```json\n{\"path\": \"src/app.rs\", \"startLine\": \"1\", \"endLine\": \"2\",}\n```"
                    }),
                },
                &cancel,
            )
            .await
            .expect("coerced");
        assert!(!coerced.output.is_error, "err={}", coerced.output.content);
        assert!(coerced.output.content.contains("pub struct App"));

        // 3. Missing required arg appends compact schema hint automatically.
        let missing = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t3".into(),
                    name: "read_file".into(),
                    arguments: json!({}),
                },
                &cancel,
            )
            .await
            .expect("missing");
        assert!(missing.output.is_error);
        assert!(
            missing.output.content.contains("Expected params for `read_file`"),
            "missing schema hint: {}",
            missing.output.content
        );

        // 4. Scoped subagent catalog hides denied & unavailable tools (cron, goal_loop, mcp_resource).
        let explorer_scope = SessionToolScope {
            role_id: Some("explorer".into()),
            permission: PermissionMode::Autonomous,
            custom_read_only: false,
            custom_tool_subset: HashSet::new(),
            availability: ToolAvailabilityContext {
                has_mcp_servers: false,
                has_skills: false,
                is_subagent: true,
                read_only: true,
            },
        };
        let explorer_cat = rt
            .tools
            .deferred_catalog_text_filtered(&explorer_scope.availability, |s| {
                explorer_scope.permits_spec(s)
            })
            .expect("explorer catalog");
        assert!(explorer_cat.contains("git_tool"));
        assert!(!explorer_cat.contains("cron_tool"));
        assert!(!explorer_cat.contains("goal_loop"));
        assert!(!explorer_cat.contains("mcp_resource"));
        assert!(!explorer_cat.contains("agent_worktree"));

        // Worker role hygiene: `coder` denies root-only `orchestrate_control`, `goal_loop`, `skill_manage`.
        let coder = find_subagent_role("coder").expect("coder role");
        assert!(!coder.permits_tool(TOOL_ORCHESTRATE_CONTROL));
        assert!(!coder.permits_tool(zenko_core::TOOL_GOAL_LOOP));
        assert!(!coder.permits_tool(zenko_core::TOOL_SKILL_MANAGE));

        // 5. `tool_search` supports `list_toolsets` & hides unconfigured `mcp`.
        let list_ts = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t4".into(),
                    name: TOOL_TOOL_SEARCH.into(),
                    arguments: json!({ "action": "list_toolsets" }),
                },
                &cancel,
            )
            .await
            .expect("list_toolsets");
        assert!(!list_ts.output.is_error);
        assert!(list_ts.output.content.contains("`git`"));
        assert!(
            !list_ts.output.content.contains("`mcp`"),
            "unconfigured MCP must be hidden"
        );

        // 6. `batch_tools` executes wave-parallel reads/outlines AND enforces Plan Mode per inner call.
        let batch_ok = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t5".into(),
                    name: TOOL_BATCH_TOOLS.into(),
                    arguments: json!({
                        "calls": [
                            { "tool": "read_file", "args": { "path": "src/app.rs", "mode": "outline" } },
                            { "tool": "code_search", "args": { "pattern": "boot()", "fixed_strings": true, "output_mode": "files_only" } }
                        ]
                    }),
                },
                &cancel,
            )
            .await
            .expect("batch_ok");
        assert!(!batch_ok.output.is_error, "batch err={}", batch_ok.output.content);
        assert!(batch_ok.output.content.contains("2/2 ok"));
        assert!(batch_ok.output.content.contains("[fn] pub fn boot"));
        assert!(batch_ok.output.content.contains("src/app.rs"));

        // Plan Mode inside `batch_tools` blocks `write_file` while permitting `read_file`,
        // and `stop_on_error: true` short-circuits subsequent calls.
        let plan_scope = SessionToolScope {
            role_id: None,
            permission: PermissionMode::Plan,
            custom_read_only: false,
            custom_tool_subset: HashSet::new(),
            availability: ToolAvailabilityContext {
                has_mcp_servers: false,
                has_skills: false,
                is_subagent: false,
                read_only: true,
            },
        };
        let _plan_guard = ActiveSessionScopeGuard::enter(rt.session_control(&sess.id), plan_scope);
        let plan_batch = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t6".into(),
                    name: TOOL_BATCH_TOOLS.into(),
                    arguments: json!({
                        "stop_on_error": true,
                        "calls": [
                            { "tool": "read_file", "args": { "path": "src/app.rs", "start_line": 1, "end_line": 1 } },
                            { "tool": "write_file", "args": { "path": "src/pwn.rs", "content": "nope" } },
                            { "tool": "list_dir", "args": { "path": "src" } }
                        ]
                    }),
                },
                &cancel,
            )
            .await
            .expect("plan_batch");
        assert!(
            plan_batch
                .output
                .content
                .contains("1/3 ok, 1 error(s), 1 skipped"),
            "plan_batch={}",
            plan_batch.output.content
        );
        assert!(plan_batch.output.content.contains("Denied by execpolicy"));
        assert!(plan_batch.output.content.contains("SKIPPED"));
        assert!(
            !dir.join("src/pwn.rs").exists(),
            "Plan Mode must never write via batch_tools"
        );

        // 7. LSP diagnostics-on-edit path extraction covers `apply_patch` `path`, `files[]`, and metadata.
        assert_eq!(
            tool_router::extract_edited_rel_paths(
                "apply_patch",
                &json!({ "path": "src/app.rs" }),
                None
            ),
            vec!["src/app.rs".to_string()]
        );
        assert_eq!(
            tool_router::extract_edited_rel_paths(
                "apply_patch",
                &json!({
                    "files": [
                        { "path": "src/a.rs", "action": "update" },
                        { "path": "src/old.rs", "action": "move", "move_to": "src/new.rs" }
                    ]
                }),
                None
            ),
            vec!["src/a.rs".to_string(), "src/new.rs".to_string()]
        );

        // 8. Denied deferred tool in Plan Mode must NOT prematurely auto-activate into session state.
        let denied_cron = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t7".into(),
                    name: zenko_core::TOOL_CRON.into(),
                    arguments: json!({ "action": "create", "name": "nope" }),
                },
                &cancel,
            )
            .await
            .expect("denied cron");
        assert!(denied_cron.output.is_error);
        assert!(
            !rt.activated_deferred_for(&sess.id)
                .contains(zenko_core::TOOL_CRON),
            "denied deferred tool must never auto-activate"
        );

        // 9. `coerce_tool_call_args` recursively coerces inner `batch_tools` args before checkpoint detection.
        let coerced_batch = rt.coerce_tool_call_args(
            TOOL_BATCH_TOOLS,
            json!({
                "calls": [
                    { "tool": "git_tool", "args": { "_raw": "{\"action\": \"COMMIT\"}" } }
                ]
            }),
        );
        let batch_tc = ToolCall {
            id: "t8".into(),
            name: TOOL_BATCH_TOOLS.into(),
            arguments: coerced_batch,
        };
        assert_eq!(is_workspace_mutating_call(&batch_tc), Some(TOOL_BATCH_TOOLS));

        // 10. `inspect_session_context` respects active session scope (Plan Mode reduces counts vs root).
        let plan_ctx_info = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("inspect plan");
        drop(_plan_guard);
        let root_ctx_info = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("inspect root");
        assert!(
            plan_ctx_info.eager_tools_count < root_ctx_info.eager_tools_count,
            "Plan Mode scoped eager count ({}) should be lower than root ({})",
            plan_ctx_info.eager_tools_count,
            root_ctx_info.eager_tools_count
        );

        // 11. Mixed `[hash_edit, shell_exec("cargo test")]` inside `batch_tools` credits trailing verification.
        let mut guard = orchestrator::guard::ExecutionGuard::new(
            zenko_core::ExecutionTier::Standard,
            zenko_core::IntentKind::Implement,
            zenko_core::DeliberationDepth::default(),
        );
        guard.record_tool_outcome(
            TOOL_BATCH_TOOLS,
            &json!({
                "calls": [
                    { "tool": "hash_edit", "args": { "path": "src/app.rs" } },
                    { "tool": "shell_exec", "args": { "command": "cargo test" } }
                ]
            }),
            false,
            "Batch completed (2/2 ok)",
            Some(&json!({
                "steps": [
                    { "index": 0, "tool": "hash_edit", "status": "ok" },
                    { "index": 1, "tool": "shell_exec", "status": "ok" }
                ]
            })),
        );
        assert!(
            guard.check_completion_gate(true).is_none(),
            "trailing passing verification inside batch_tools must satisfy completion guard"
        );

        // 12. Tool Architecture v3: ToolRouter + SessionFileTracker end-to-end stale-read & LSP rename gates!
        std::fs::write(dir.join("tracked.rs"), "let v = 1;\n").expect("tracked");
        let blind_overwrite = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t12a".into(),
                    name: "write_file".into(),
                    arguments: json!({ "path": "tracked.rs", "content": "let v = 9;\n" }),
                },
                &cancel,
            )
            .await
            .expect("blind_overwrite");
        assert!(blind_overwrite.output.is_error);
        assert!(blind_overwrite.output.content.contains("stale-read guard"));

        let read_ok = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t12b".into(),
                    name: "read_file".into(),
                    arguments: json!({ "path": "tracked.rs" }),
                },
                &cancel,
            )
            .await
            .expect("read_ok");
        assert!(!read_ok.output.is_error);

        // External disk drift blocks apply_patch cleanly
        std::fs::write(dir.join("tracked.rs"), "let v = 777;\n").expect("ext_drift");
        let drift_block = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t12c".into(),
                    name: "apply_patch".into(),
                    arguments: json!({ "path": "tracked.rs", "old_text": "777", "new_text": "888" }),
                },
                &cancel,
            )
            .await
            .expect("drift_block");
        assert!(drift_block.output.is_error);
        assert!(drift_block.output.content.contains("changed externally"));

        // Plan Mode blocks mutating `lsp_tool(action="rename")`, while permitting `dry_run: true`!
        let plan_lsp_rename = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t12d".into(),
                    name: TOOL_LSP.into(),
                    arguments: json!({ "action": "rename", "path": "tracked.rs", "line": 1, "column": 5, "new_name": "x" }),
                },
                &cancel,
            )
            .await
            .expect("plan_lsp_rename");
        assert!(plan_lsp_rename.output.is_error);
        assert!(
            plan_lsp_rename.output.content.contains("Denied by execpolicy"),
            "plan_lsp_rename={}",
            plan_lsp_rename.output.content
        );

        let ro_scope = SessionToolScope {
            role_id: Some("explorer".into()),
            permission: PermissionMode::Autonomous,
            custom_read_only: false,
            custom_tool_subset: HashSet::new(),
            availability: ToolAvailabilityContext::default(),
        };
        assert!(
            ro_scope
                .permits_invocation(
                    TOOL_LSP,
                    &json!({ "action": "rename", "path": "tracked.rs", "line": 1, "column": 5, "new_name": "x" })
                )
                .is_err(),
            "read-only explorer subagent must never execute mutating lsp_tool rename"
        );
        assert!(
            ro_scope
                .permits_invocation(
                    TOOL_LSP,
                    &json!({ "action": "rename", "path": "tracked.rs", "line": 1, "column": 5, "new_name": "x", "dry_run": true })
                )
                .is_ok(),
            "read-only explorer subagent may preview dry_run lsp_tool rename"
        );

        // 13. Tool Architecture v4: KV-cache frozen tool surface, `invoke_tool`, dataflow `batch_tools`, and persistent PTY!
        assert!(cfg_now.context.freeze_tool_schemas);
        assert!(cfg_now.policy.os_sandbox);

        // Trimmed Eager surface hides `skill_view` when 0 skills exist & defers `list_dir`/`record_brainstorm`/`plan_update`/`orchestrate_control`.
        let root_scope_no_skills = SessionToolScope {
            role_id: None,
            permission: PermissionMode::Autonomous,
            custom_read_only: false,
            custom_tool_subset: HashSet::new(),
            availability: ToolAvailabilityContext {
                has_mcp_servers: false,
                has_skills: false,
                is_subagent: false,
                read_only: false,
            },
        };
        let frozen_before = rt.tools.specs_for_session_filtered(
            true,
            &BTreeSet::new(),
            &root_scope_no_skills.availability,
            |s| root_scope_no_skills.permits_spec(s),
        );
        let eager_names: Vec<&str> = frozen_before.iter().map(|s| s.name.as_str()).collect();
        assert!(eager_names.contains(&TOOL_INVOKE_TOOL));
        assert!(!eager_names.contains(&"skill_view"));
        assert!(!eager_names.contains(&"list_dir"));
        assert!(!eager_names.contains(&TOOL_RECORD_BRAINSTORM));
        assert!(!eager_names.contains(&TOOL_PLAN_UPDATE));
        assert!(
            eager_names.len() <= 13,
            "expected trimmed <=13 eager tools, got {}: {eager_names:?}",
            eager_names.len()
        );

        // `invoke_tool` executes deferred `list_dir` cleanly while keeping Plan Mode & concurrency exact!
        let invoked = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t13a".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({ "tool": "list_dir", "args": { "path": "src" } }),
                },
                &cancel,
            )
            .await
            .expect("invoke_tool");
        assert!(!invoked.output.is_error, "invoke err={}", invoked.output.content);
        assert!(invoked.output.content.contains("app.rs"));

        // `invoke_tool` in Plan Mode still denies mutating `lsp_tool(rename)`!
        let invoked_plan_deny = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t13b".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({
                        "tool": TOOL_LSP,
                        "args": { "action": "rename", "path": "tracked.rs", "line": 1, "column": 5, "new_name": "z" }
                    }),
                },
                &cancel,
            )
            .await
            .expect("invoke_plan_deny");
        assert!(invoked_plan_deny.output.is_error);
        assert!(invoked_plan_deny.output.content.contains("Denied by execpolicy"));

        // Dataflow `batch_tools`: Step 1 quietly finds `*.rs` paths -> Step 2 reads `${found.first_line}` in 1 turn!
        let dataflow = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t13c".into(),
                    name: TOOL_BATCH_TOOLS.into(),
                    arguments: json!({
                        "calls": [
                            { "tool": "glob_find", "args": { "pattern": "src/*.rs" }, "as": "found", "extract": "paths", "quiet": true },
                            { "tool": "read_file", "args": { "path": "${found.first_line}", "start_line": 1, "end_line": 3 } }
                        ]
                    }),
                },
                &cancel,
            )
            .await
            .expect("dataflow");
        assert!(!dataflow.output.is_error, "df err={}", dataflow.output.content);
        assert!(dataflow.output.content.contains("bound `$found`"));
        assert!(dataflow.output.content.contains("pub fn boot()"));

        // Persistent PTY through `shell_exec` preserves state across turns and closes cleanly!
        let pty1 = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t13d".into(),
                    name: TOOL_SHELL_EXEC.into(),
                    arguments: json!({ "session": "rt_pty", "command": "export Z_V4=hello_pty && cd src" }),
                },
                &cancel,
            )
            .await
            .expect("pty1");
        assert!(!pty1.output.is_error, "pty1 err={}", pty1.output.content);

        let pty2 = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t13e".into(),
                    name: TOOL_SHELL_EXEC.into(),
                    arguments: json!({ "session": "rt_pty", "command": "printf '%s|%s' \"$Z_V4\" \"$(basename $(pwd))\"" }),
                },
                &cancel,
            )
            .await
            .expect("pty2");
        assert!(!pty2.output.is_error, "pty2 err={}", pty2.output.content);
        assert!(pty2.output.content.contains("hello_pty|src"), "pty2={}", pty2.output.content);

        // 14. Native English Concise Mode + Read-Less CCR + Pure-Rust Doc Compress + Script Blocks (v5)
        // a) Read-Less compresses bulky cargo-test-style shell output AND persists 100% raw CCR artifact!
        let rl_cmd = "i=0; while [ $i -lt 30 ]; do echo \"test suite::case_$i ... ok\"; i=$((i+1)); done; echo \"test result: ok. 30 passed; 0 failed; 0 ignored\"";
        let rl_exec = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14a".into(),
                    name: TOOL_SHELL_EXEC.into(),
                    arguments: json!({ "command": rl_cmd }),
                },
                &cancel,
            )
            .await
            .expect("read_less exec");
        assert!(!rl_exec.output.is_error, "rl err={}", rl_exec.output.content);
        assert!(
            rl_exec.output.content.contains("+30 passing tests collapsed"),
            "expected collapsed passing tests, got:\n{}",
            rl_exec.output.content
        );
        assert!(
            rl_exec.output.content.contains("[read-less: test_output")
                && rl_exec.output.content.contains("artifact:"),
            "expected CCR artifact pointer in:\n{}",
            rl_exec.output.content
        );
        let (rl_calls, rl_bytes, rl_toks) = rt.read_less_stats_for(&sess.id);
        assert!(rl_calls >= 1 && rl_bytes >= 256 && rl_toks >= 64);

        // a2) `shell_exec(raw=true)` explicitly bypasses Read-Less/Dedup and tags `raw_bypass` metadata!
        let raw_exec = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14a_raw".into(),
                    name: TOOL_SHELL_EXEC.into(),
                    arguments: json!({ "command": rl_cmd, "raw": true }),
                },
                &cancel,
            )
            .await
            .expect("raw bypass exec");
        assert!(!raw_exec.output.is_error);
        assert!(
            !raw_exec.output.content.contains("[read-less:")
                && !raw_exec.output.content.contains("[dedup:"),
            "raw=true must skip compression/dedup: {}",
            raw_exec.output.content
        );
        assert_eq!(
            raw_exec
                .output
                .metadata
                .as_ref()
                .and_then(|m| m.get("raw_bypass"))
                .and_then(Value::as_bool),
            Some(true)
        );

        // b) `orchestrate_control` switches English concise_mode live & reflects in `inspect_session_context`!
        let cm_shift = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14b".into(),
                    name: TOOL_ORCHESTRATE_CONTROL.into(),
                    arguments: json!({ "concise_mode": "ultra", "rationale": "high-density run" }),
                },
                &cancel,
            )
            .await
            .expect("concise shift");
        assert!(!cm_shift.output.is_error, "cm err={}", cm_shift.output.content);
        assert!(cm_shift.output.content.contains("concise_mode=ultra"));
        let ctx_after_rl = rt.inspect_session_context(&sess.id).await.expect("ctx_after_rl");
        assert_eq!(ctx_after_rl.concise_mode, ConciseMode::Ultra);
        assert!(ctx_after_rl.read_less_enabled);
        assert_eq!(ctx_after_rl.read_less_calls_compressed, rl_calls);
        assert_eq!(ctx_after_rl.read_less_bytes_saved, rl_bytes);

        // c) `memory_tool(action="compress_doc")` compresses MEMORY.md in pure Rust with rollback backup!
        rt.snapshots
            .write_doc(
                "MEMORY.md",
                "# Long-Term Memory\n\n- Please make sure to run `cargo test` in order to verify the build prior to committing.\n- It is important to note that the config lives at `./zenko.toml`.\n",
            )
            .expect("seed memory");
        let compressed_doc = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14c".into(),
                    name: TOOL_MEMORY.into(),
                    arguments: json!({ "action": "compress_doc", "doc": "MEMORY.md", "mode": "full" }),
                },
                &cancel,
            )
            .await
            .expect("compress_doc");
        assert!(!compressed_doc.output.is_error, "cd err={}", compressed_doc.output.content);
        assert!(compressed_doc.output.content.contains("Compressed MEMORY.md [mode=full]"));
        assert!(compressed_doc.output.content.contains("rollback snapshot"));
        let mem_now = rt.snapshots.read_doc("MEMORY.md").expect("read mem");
        assert!(mem_now.contains("`cargo test`") && mem_now.contains("./zenko.toml"));

        // d) `script_block` saves & runs inside Landlock sandbox, and Plan Mode denies mutating `run`!
        let sb_save = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14d".into(),
                    name: zenko_core::TOOL_SCRIPT_BLOCK.into(),
                    arguments: json!({
                        "action": "save",
                        "name": "quick-check",
                        "description": "Verify workspace name",
                        "script": "echo \"block-ok:$1\""
                    }),
                },
                &cancel,
            )
            .await
            .expect("sb_save");
        assert!(!sb_save.output.is_error, "sb_save err={}", sb_save.output.content);
        let sb_run = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14e".into(),
                    name: zenko_core::TOOL_SCRIPT_BLOCK.into(),
                    arguments: json!({ "action": "run", "name": "quick-check", "args": ["v5"] }),
                },
                &cancel,
            )
            .await
            .expect("sb_run");
        assert!(!sb_run.output.is_error, "sb_run err={}", sb_run.output.content);
        assert!(sb_run.output.content.contains("block-ok:v5"));
        let sb_plan_deny = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t14f".into(),
                    name: zenko_core::TOOL_SCRIPT_BLOCK.into(),
                    arguments: json!({ "action": "run", "name": "quick-check" }),
                },
                &cancel,
            )
            .await
            .expect("sb_plan_deny");
        assert!(sb_plan_deny.output.is_error);
        assert!(sb_plan_deny.output.content.contains("Denied by execpolicy"));

        // e) Persisted SQLite `ToolResult.read_less` stats survive simulated daemon restart!
        let _ = rt
            .store
            .append_event(
                &sess.id,
                None,
                EventRole::Tool,
                EventKind::ToolResult,
                json!({
                    "tool_call_id": "t14_persist",
                    "tool_name": TOOL_SHELL_EXEC,
                    "content": "compressed output",
                    "is_error": false,
                    "diff": Value::Null,
                    "duration_ms": 12,
                    "read_less": {
                        "shape": "test_output",
                        "original_bytes": 5000,
                        "compressed_bytes": 904,
                        "bytes_saved": 4096,
                        "tokens_saved": 1024,
                        "savings_pct": 81.9
                    }
                }),
                &UsageStats::default(),
            )
            .expect("append read_less event");
        rt.clear_session_deferred_tools(&sess.id); // clears in-memory SessionControl simulating restart
        let after_restart = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("inspect after restart");
        assert_eq!(after_restart.read_less_calls_compressed, 1);
        assert_eq!(after_restart.read_less_bytes_saved, 4096);
        assert_eq!(after_restart.read_less_tokens_saved, 1024);

        // 15. End-to-end Token Economy wiring: demoted tools searchable via `tool_search`,
        // live ContextBreakdown accuracy, Plan Mode read-only gating, and `batch_tools` shell tracing.
        let before_econ = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("before_econ");
        assert!(
            rt.economy_apply_tool_prune(Some(&dir), TOOL_SPAWN_AGENT)
                .expect("apply_prune"),
            "non-core eager tool `spawn_agent` should soft-prune cleanly"
        );
        let _ = rt
            .economy_add_micro_skill(
                Some(&dir),
                "Run `just verify` instead of bare `cargo test` in this repo.",
                Some("cargo test"),
                Some("just verify"),
            )
            .expect("add_micro_skill");

        let after_econ = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("after_econ");
        assert_eq!(after_econ.eager_tools_count + 1, before_econ.eager_tools_count);
        assert_eq!(
            after_econ.deferred_available_count,
            before_econ.deferred_available_count + 1
        );
        assert!(after_econ.tool_schema_tokens < before_econ.tool_schema_tokens);
        assert!(after_econ.tier2_tokens > before_econ.tier2_tokens);
        assert_eq!(after_econ.economy_pruned_tools_count, 1);
        assert_eq!(after_econ.economy_active_micro_skills, 1);

        // Soft-demoted `spawn_agent` is now discoverable via `tool_search`!
        let found_demoted = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t15a".into(),
                    name: TOOL_TOOL_SEARCH.into(),
                    arguments: json!({ "names": [TOOL_SPAWN_AGENT] }),
                },
                &cancel,
            )
            .await
            .expect("tool_search demoted");
        assert!(!found_demoted.output.is_error);
        assert!(
            found_demoted.output.content.contains(TOOL_SPAWN_AGENT),
            "tool_search must return soft-demoted tool schema: {}",
            found_demoted.output.content
        );

        // Plan Mode permits read-only `token_economy(action="status")` via `invoke_tool`
        // while denying mutating `add_micro_skill`.
        let plan_econ_ro = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t15b".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({
                        "tool": TOOL_TOKEN_ECONOMY,
                        "args": { "action": "status" }
                    }),
                },
                &cancel,
            )
            .await
            .expect("plan_econ_ro");
        assert!(
            !plan_econ_ro.output.is_error,
            "Plan Mode must permit read-only token_economy: {}",
            plan_econ_ro.output.content
        );
        assert!(plan_econ_ro.output.content.contains("Token Economy"));

        let plan_econ_mut = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t15c".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({
                        "tool": TOOL_TOKEN_ECONOMY,
                        "args": { "action": "add_micro_skill", "fact": "should block" }
                    }),
                },
                &cancel,
            )
            .await
            .expect("plan_econ_mut");
        assert!(plan_econ_mut.output.is_error);
        assert!(plan_econ_mut.output.content.contains("Denied by execpolicy"));

        // `batch_tools` shell steps carry `shell_command` + `output_preview` metadata for economy tracing.
        let batch_shell = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t15d".into(),
                    name: TOOL_BATCH_TOOLS.into(),
                    arguments: json!({
                        "calls": [
                            { "tool": TOOL_SHELL_EXEC, "args": { "command": "echo econ-batch-ok" } }
                        ]
                    }),
                },
                &cancel,
            )
            .await
            .expect("batch_shell");
        assert!(!batch_shell.output.is_error);
        let mut trace = crate::economy::TurnEconomyTrace::new("project:test".into());
        crate::economy::record_tool_outcome_in_economy_trace(
            &mut trace,
            TOOL_BATCH_TOOLS,
            &json!({}),
            &batch_shell.output,
        );
        assert!(trace.used_tools.contains(TOOL_SHELL_EXEC));
        assert_eq!(trace.shell_traces.len(), 1);
        assert_eq!(trace.shell_traces[0].command, "echo econ-batch-ok");
        assert!(trace.shell_traces[0].output_preview.contains("econ-batch-ok"));

        // 16. Native Rust Code Minimalism Engine: read-only Plan Mode access, live `orchestrate_control`
        // switching (zero /commands), debt scanning, and live ContextBreakdown churn telemetry.
        std::fs::write(
            dir.join("src/debt_sample.rs"),
            "pub fn sample_helper() {\n    // DEBT: replace temporary linear lookup\n}\n",
        )
        .expect("write debt_sample");
        let plan_debt = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t16a".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({
                        "tool": TOOL_CODE_MINIMALISM,
                        "args": { "action": "debt_scan" }
                    }),
                },
                &cancel,
            )
            .await
            .expect("plan_debt");
        assert!(
            !plan_debt.output.is_error,
            "Plan Mode must permit read-only code_minimalism: {}",
            plan_debt.output.content
        );
        assert!(plan_debt.output.content.contains("DEBT:"));
        assert!(plan_debt.output.content.contains("debt_sample.rs"));

        let shift_min = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t16b".into(),
                    name: TOOL_ORCHESTRATE_CONTROL.into(),
                    arguments: json!({ "minimalism_mode": "ultra" }),
                },
                &cancel,
            )
            .await
            .expect("shift_min");
        assert!(!shift_min.output.is_error);
        assert!(shift_min.output.content.contains("minimalism_mode=ultra"));

        let repo_scope = project_scope_for_workspace(&dir);
        rt.store
            .record_turn_code_metrics(&repo_scope, 42, 18, 1, 0)
            .expect("record metrics");
        let min_ctx = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("min_ctx");
        assert_eq!(min_ctx.minimalism_mode, CodeMinimalismMode::Ultra);
        assert_eq!(min_ctx.economy_lines_added, 42);
        assert_eq!(min_ctx.economy_lines_deleted, 18);

        let gain_out = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t16c".into(),
                    name: TOOL_INVOKE_TOOL.into(),
                    arguments: json!({
                        "tool": TOOL_CODE_MINIMALISM,
                        "args": { "action": "gain" }
                    }),
                },
                &cancel,
            )
            .await
            .expect("gain_out");
        assert!(!gain_out.output.is_error);
        assert!(gain_out.output.content.contains("mode=ultra"));
        assert!(gain_out.output.content.contains("+42 / -18 LOC"));

        // Verify live JIT hierarchical DOX / nested AGENTS.md single-injection, ContextBreakdown
        // reporting, and compaction/rewind reset behavior.
        std::fs::create_dir_all(dir.join("crates/submod/src")).expect("mkdir submod");
        std::fs::write(
            dir.join("crates/submod/AGENTS.md"),
            "# Submod DOX\n\n## Purpose\nHandles payment settlement state machines.\n",
        )
        .expect("submod agents");
        std::fs::write(
            dir.join("crates/submod/src/worker.rs"),
            "pub fn settle() -> bool { true }\n",
        )
        .expect("worker.rs");

        let first_read = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t17a".into(),
                    name: zenko_core::TOOL_READ_FILE.into(),
                    arguments: json!({ "path": "crates/submod/src/worker.rs" }),
                },
                &cancel,
            )
            .await
            .expect("first_read");
        assert!(!first_read.output.is_error);
        assert!(first_read
            .output
            .content
            .contains("<scoped_repo_rules path=\"crates/submod/AGENTS.md\">"));
        assert!(first_read
            .output
            .content
            .contains("Handles payment settlement state machines."));

        // Second read inside the same scope must NOT re-inject the already-loaded rule.
        let second_read = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t17b".into(),
                    name: zenko_core::TOOL_READ_FILE.into(),
                    arguments: json!({ "path": "crates/submod/src/worker.rs" }),
                },
                &cancel,
            )
            .await
            .expect("second_read");
        assert!(!second_read.output.is_error);
        assert!(!second_read.output.content.contains("<scoped_repo_rules"));

        let dox_ctx = rt
            .inspect_session_context(&sess.id)
            .await
            .expect("dox_ctx");
        assert_eq!(dox_ctx.scoped_rules_loaded, vec!["crates/submod/AGENTS.md"]);
        assert!(dox_ctx.cache_stability_warnings.is_empty());

        // A new session started after `crates/submod/AGENTS.md` exists immediately indexes it in Tier-2 `<repo_doc_tree>`.
        let sess_dox = rt
            .store
            .create_session(
                "dox-index",
                "cli",
                Some(&dir.display().to_string()),
                None,
                PermissionMode::Autonomous,
            )
            .expect("create sess_dox");
        let fresh_dox_ctx = rt
            .inspect_session_context(&sess_dox.id)
            .await
            .expect("fresh_dox_ctx");
        assert!(fresh_dox_ctx.nested_repo_docs_count >= 1);
        assert!(fresh_dox_ctx.cache_stability_warnings.is_empty());

        // Compaction/rewind reset clears loaded_scoped_rules so next read re-injects cleanly.
        rt.clear_session_scoped_rules(&sess.id);
        let third_read = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t17c".into(),
                    name: zenko_core::TOOL_READ_FILE.into(),
                    arguments: json!({ "path": "crates/submod/src/worker.rs" }),
                },
                &cancel,
            )
            .await
            .expect("third_read");
        assert!(!third_read.output.is_error);
        assert!(third_read
            .output
            .content
            .contains("<scoped_repo_rules path=\"crates/submod/AGENTS.md\">"));

        // Verify JIT scoped rules survive oversized artifact spilling verbatim (injected AFTER spill).
        std::fs::create_dir_all(dir.join("crates/huge/src")).expect("mkdir huge");
        std::fs::write(
            dir.join("crates/huge/AGENTS.md"),
            "# Huge DOX\n\n## Purpose\nMust survive oversized spill intact.\n",
        )
        .expect("huge agents");
        std::fs::write(
            dir.join("crates/huge/src/big.rs"),
            "pub const PAD: &str = \"0123456789abcdef\";\n".repeat(120),
        )
        .expect("big.rs");
        let mut tight_spill_ctx = tool_ctx.clone();
        tight_spill_ctx.max_output_bytes = 512;
        let spilled_read = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tight_spill_ctx,
                true,
                ToolCall {
                    id: "t17d".into(),
                    name: zenko_core::TOOL_READ_FILE.into(),
                    arguments: json!({ "path": "crates/huge/src/big.rs" }),
                },
                &cancel,
            )
            .await
            .expect("spilled_read");
        assert!(!spilled_read.output.is_error);
        assert!(spilled_read.output.content.contains("artifact:"));
        assert!(
            spilled_read
                .output
                .content
                .contains("<scoped_repo_rules path=\"crates/huge/AGENTS.md\">\n# Huge DOX\n\n## Purpose\nMust survive oversized spill intact.\n</scoped_repo_rules>"),
            "scoped rule must survive oversized spill verbatim: {}",
            spilled_read.output.content
        );

        // Verify restart durability: persisting ToolResult with `scoped_rules_injected` hydrates
        // `loaded_scoped_rules` after in-memory state is cleared (simulating daemon restart).
        let restart_sess = rt
            .store
            .create_session(
                "dox-restart",
                "cli",
                Some(&dir.display().to_string()),
                None,
                PermissionMode::Autonomous,
            )
            .expect("create restart_sess");
        rt.store
            .append_event(
                &restart_sess.id,
                None,
                EventRole::Tool,
                EventKind::ToolResult,
                json!({
                    "tool_call_id": "c_persisted",
                    "tool_name": zenko_core::TOOL_READ_FILE,
                    "content": "1:aaa| pub fn settle() -> bool { true }",
                    "is_error": false,
                    "scoped_rules_injected": ["crates/submod/AGENTS.md"],
                }),
                &UsageStats::default(),
            )
            .expect("append persisted scoped rule event");
        rt.clear_session_scoped_rules(&restart_sess.id);
        let hydrated_ctx = rt
            .inspect_session_context(&restart_sess.id)
            .await
            .expect("hydrated_ctx");
        assert_eq!(
            hydrated_ctx.scoped_rules_loaded,
            vec!["crates/submod/AGENTS.md"]
        );
        let post_restart_read = rt
            .execute_single_tool_call(
                &cfg_now,
                &restart_sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t17e".into(),
                    name: zenko_core::TOOL_READ_FILE.into(),
                    arguments: json!({ "path": "crates/submod/src/worker.rs" }),
                },
                &cancel,
            )
            .await
            .expect("post_restart_read");
        assert!(
            !post_restart_read.output.content.contains("<scoped_repo_rules"),
            "hydrated scoped rule must not re-inject after daemon restart"
        );

        // Verify native Project Brain (`project_brain`) end-to-end:
        // 1. Plan Mode policy blocks mutating `init` and permits read-only `list_pages`.
        // 2. Orchestrator classifier categorizes read-only vs mutating (direct & via `invoke_tool`).
        // 3. Autonomous `init` -> `create_page` -> `update_truth` (reversal) -> `impact` -> `search`.
        // 4. Fresh session Tier-2 snapshot emits `<project_brain_index>` & `project_brain_pages_count == 1`.
        // 5. Parallel Sovereign Orchestrator recon auto-surfaces `brain_hits`.
        assert_eq!(
            crate::orchestrator::classify_tool_category(
                zenko_core::TOOL_PROJECT_BRAIN,
                &json!({ "action": "search", "query": "ledger" }),
            ),
            crate::orchestrator::ToolCategory::ReadOnly
        );
        assert_eq!(
            crate::orchestrator::classify_tool_category(
                zenko_core::TOOL_PROJECT_BRAIN,
                &json!({ "action": "update_truth", "id": "payment-ledger", "truth": "v2" }),
            ),
            crate::orchestrator::ToolCategory::Mutation
        );
        assert_eq!(
            crate::orchestrator::classify_tool_category(
                zenko_core::TOOL_INVOKE_TOOL,
                &json!({
                    "tool": zenko_core::TOOL_PROJECT_BRAIN,
                    "args": { "action": "create_page", "id": "payment-ledger" }
                }),
            ),
            crate::orchestrator::ToolCategory::Mutation
        );

        let plan_brain_init = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18a".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({ "action": "init" }),
                },
                &cancel,
            )
            .await
            .expect("plan_brain_init");
        assert!(
            plan_brain_init.output.is_error,
            "Plan Mode must block mutating project_brain init: {}",
            plan_brain_init.output.content
        );

        let plan_brain_list = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Plan,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18b".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({ "action": "list_pages" }),
                },
                &cancel,
            )
            .await
            .expect("plan_brain_list");
        assert!(
            !plan_brain_list.output.is_error,
            "Plan Mode must permit read-only project_brain list_pages: {}",
            plan_brain_list.output.content
        );

        let auto_brain_init = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18c".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({ "action": "init" }),
                },
                &cancel,
            )
            .await
            .expect("auto_brain_init");
        assert!(!auto_brain_init.output.is_error);

        let auto_brain_create = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18d".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({
                        "action": "create_page",
                        "id": "payment-ledger",
                        "title": "Payment Ledger Storage",
                        "category": "decision",
                        "affects": ["billing-api"],
                        "compiled_truth": "Use SQLite WAL for single-node ledger durability.",
                        "summary": "Initial storage choice: SQLite WAL (zero external dependencies)",
                        "source": "ADR-01"
                    }),
                },
                &cancel,
            )
            .await
            .expect("auto_brain_create");
        assert!(
            !auto_brain_create.output.is_error,
            "create_page failed: {}",
            auto_brain_create.output.content
        );

        let auto_brain_rev = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18e".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({
                        "action": "update_truth",
                        "id": "payment-ledger",
                        "kind": "reversal",
                        "compiled_truth": "Use Postgres for multi-region ledger replication.",
                        "summary": "Reversed SQLite WAL in favor of Postgres for cross-region HA",
                        "source": "ADR-12",
                        "affects": ["billing-api"]
                    }),
                },
                &cancel,
            )
            .await
            .expect("auto_brain_rev");
        assert!(
            !auto_brain_rev.output.is_error,
            "update_truth failed: {}",
            auto_brain_rev.output.content
        );

        let auto_brain_impact = rt
            .execute_single_tool_call(
                &cfg_now,
                &sess.id,
                PermissionMode::Autonomous,
                &tool_ctx,
                true,
                ToolCall {
                    id: "t18f".into(),
                    name: zenko_core::TOOL_PROJECT_BRAIN.into(),
                    arguments: json!({ "action": "impact", "id": "payment-ledger" }),
                },
                &cancel,
            )
            .await
            .expect("auto_brain_impact");
        assert!(
            !auto_brain_impact.output.is_error,
            "impact failed: {}",
            auto_brain_impact.output.content
        );
        assert!(
            auto_brain_impact.output.content.contains("billing-api"),
            "impact BFS must surface affected node: {}",
            auto_brain_impact.output.content
        );

        let brain_sess = rt
            .store
            .create_session(
                "brain-fresh",
                "cli",
                Some(&dir.display().to_string()),
                None,
                PermissionMode::Autonomous,
            )
            .expect("create brain_sess");
        let brain_snap = rt
            .frozen_snapshot_for_session(&brain_sess.id, &dir, &cfg_now)
            .expect("brain_snap");
        assert_eq!(brain_snap.project_brain_pages_count, 1);
        assert!(
            brain_snap
                .tier2_prompt
                .contains("<project_brain_index")
                && brain_snap
                    .tier2_prompt
                    .contains("payment-ledger"),
            "fresh session Tier-2 must include <project_brain_index>: {}",
            brain_snap.tier2_prompt
        );
        let brain_ctx = rt
            .inspect_session_context(&brain_sess.id)
            .await
            .expect("brain_ctx");
        assert_eq!(brain_ctx.project_brain_pages_count, 1);

        let brain_recon = crate::orchestrator::recon::run_parallel_recon(
            dir.clone(),
            cfg_now.resolved_hub_dir(),
            brain_sess.id.clone(),
            "Revisit payment ledger SQLite WAL storage decision",
            rt.store.clone(),
            rt.jobs.clone(),
        )
        .await;
        assert!(
            brain_recon
                .brain_hits
                .iter()
                .any(|h| h.contains("payment-ledger")),
            "orchestrator parallel recon must auto-surface Project Brain hits: {:?}",
            brain_recon.brain_hits
        );
        let recon_block = crate::orchestrator::recon::format_recon_context(&brain_recon);
        assert!(
            recon_block.contains("project_brain_truth") && recon_block.contains("payment-ledger"),
            "formatted recon context must include Project Brain truth block: {recon_block}"
        );

        rt.shutdown_external_tools().await;
        let _ = std::fs::remove_dir_all(dir);
    }
