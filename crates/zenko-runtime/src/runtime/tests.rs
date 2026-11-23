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
