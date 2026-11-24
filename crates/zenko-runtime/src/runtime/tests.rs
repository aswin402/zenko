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

        let _ = std::fs::remove_dir_all(dir);
    }
