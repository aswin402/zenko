    use super::*;

    #[test]
    fn session_event_tree_and_fts_search() {
        let dir = std::env::temp_dir().join(format!("zenko_mem_test_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let s = store
            .create_session("Test", "webui", None, None, PermissionMode::Balanced)
            .expect("create session");
        let e1 = store
            .append_event(
                &s.id,
                None,
                EventRole::User,
                EventKind::Message,
                serde_json::json!({"text": "hello"}),
                &UsageStats::default(),
            )
            .expect("e1");
        let e2 = store
            .append_event(
                &s.id,
                Some(&e1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "world"}),
                &UsageStats::default(),
            )
            .expect("e2");
        let _e2_alt = store
            .append_event(
                &s.id,
                Some(&e1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "alternate branch"}),
                &UsageStats::default(),
            )
            .expect("e2_alt");

        let branch = store.get_active_branch(&s.id).expect("branch");
        assert_eq!(branch.len(), 2);
        assert_eq!(branch[1].content_json["text"], "alternate branch");

        store.set_session_head(&s.id, Some(&e2.id)).expect("switch");
        let branch1 = store.get_active_branch(&s.id).expect("branch1");
        assert_eq!(branch1[1].content_json["text"], "world");

        store
            .add_memory("global", "fact", "Rust Toolchain", "Uses rustls and mimalloc", 1.0)
            .expect("add mem");
        let found = store.search_memory_fts("mimalloc", 5).expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Rust Toolchain");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scoped_hybrid_memory_dedup_session_search_and_dream_lease() {
        let dir = std::env::temp_dir().join(format!("zenko_p4_sql_{}", new_id()));
        let ws_a = dir.join("repo_a");
        let ws_b = dir.join("repo_b");
        std::fs::create_dir_all(ws_a.join("src")).expect("ws_a");
        std::fs::create_dir_all(&ws_b).expect("ws_b");
        std::fs::write(ws_a.join("src/auth.rs"), "pub fn verify() {}").expect("auth");

        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let scope_a = crate::project_scope_for_workspace(&ws_a);
        let scope_b = crate::project_scope_for_workspace(&ws_b);
        assert_ne!(scope_a, scope_b);

        // 1. SimHash + Jaccard deduplication merges near-identical facts in the same scope
        let ins1 = store
            .upsert_memory_dedup(
                &scope_a,
                "project",
                "Auth Middleware",
                "JWT verification lives in src/auth.rs and never uses OpenSSL",
                1.0,
                None,
                None,
                true,
            )
            .expect("ins1");
        assert!(!ins1.merged);
        let ins2 = store
            .upsert_memory_dedup(
                &scope_a,
                "project",
                "Auth Middleware",
                "JWT verification lives in src/auth.rs and never uses OpenSSL crates",
                1.1,
                None,
                None,
                true,
            )
            .expect("ins2");
        assert!(ins2.merged, "near-duplicate must merge instead of duplicating");
        assert_eq!(ins1.entry.id, ins2.entry.id);

        // 2. Workspace scoping isolates Repo A from Repo B + 2-tier AND->OR recalls multi-word queries
        store
            .upsert_memory_dedup(
                &scope_b,
                "project",
                "Repo B Secret",
                "Repo B uses GraphQL resolvers exclusively",
                1.0,
                None,
                None,
                true,
            )
            .expect("ins_b");
        let hits_a = store
            .search_memory_hybrid(
                "How does JWT authentication verification work with nonexistentword?",
                Some(&scope_a),
                true,
                Some(&ws_a),
                5,
            )
            .expect("hits_a");
        assert_eq!(hits_a.len(), 1);
        assert_eq!(hits_a[0].entry.id, ins1.entry.id);
        assert!(hits_a[0].stale_paths.is_empty());
        assert_eq!(hits_a[0].entry.access_count, 1);

        // Delete src/auth.rs -> live staleness verifier flags it immediately!
        std::fs::remove_file(ws_a.join("src/auth.rs")).expect("rm");
        let hits_stale = store
            .search_memory_hybrid("JWT auth", Some(&scope_a), true, Some(&ws_a), 5)
            .expect("hits_stale");
        assert_eq!(hits_stale[0].stale_paths, vec!["src/auth.rs"]);

        // 3. Cross-session FTS5 `session_search` + Hermes ±N context window expansion
        let s1 = store
            .create_session("Auth Refactor", "cli", None, None, PermissionMode::Balanced)
            .expect("s1");
        store
            .set_session_workspace(&s1.id, &ws_a.display().to_string())
            .expect("set ws");
        let ev1 = store
            .append_event(
                &s1.id,
                None,
                EventRole::User,
                EventKind::Message,
                serde_json::json!({"text": "Fix PASETO token rotation clock skew in auth adapter"}),
                &UsageStats::default(),
            )
            .expect("ev1");
        let _ev2 = store
            .append_event(
                &s1.id,
                Some(&ev1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "Added 30-second leeway to PASETO verifier."}),
                &UsageStats::default(),
            )
            .expect("ev2");
        let s_hits = store
            .search_sessions_fts(&SessionSearchFilter {
                query: "PASETO rotation skew".into(),
                workspace_dir: Some(ws_a.display().to_string()),
                ..Default::default()
            })
            .expect("s_hits");
        assert_eq!(s_hits.len(), 1);
        assert_eq!(s_hits[0].session_id, s1.id);

        let ctx_win = store
            .get_session_event_context(&s_hits[0].event_id, 2)
            .expect("ctx_win")
            .expect("some ctx");
        assert_eq!(ctx_win.events.len(), 2);
        assert!(ctx_win.events[0].is_anchor);
        assert!(ctx_win.events[1].text.contains("30-second leeway"));

        // 4. Codex Stage-1 rollout summary cache round-trip + Stage-0 hygiene stale salience demotion
        let rsum = SessionRolloutSummary {
            session_id: s1.id.clone(),
            scope: scope_a.clone(),
            digest: "### Session cached digest".into(),
            session_updated_at: "2026-11-22T00:00:00Z".into(),
            created_at: Utc::now(),
        };
        store.upsert_rollout_summary(&rsum).expect("upsert rollout");
        assert!(
            store
                .get_rollout_summary(&s1.id, "2026-11-22T00:00:00Z")
                .expect("get rollout")
                .is_some()
        );
        assert!(
            store
                .get_rollout_summary(&s1.id, "2026-11-22T01:00:00Z")
                .expect("stale rollout miss")
                .is_none()
        );
        let before_sal = store.get_memory(&ins1.entry.id).expect("get").expect("e").salience;
        let hyg = store.run_memory_hygiene(Some(&ws_a)).expect("hygiene");
        assert!(hyg.stale_flagged >= 1);
        let after_sal = store.get_memory(&ins1.entry.id).expect("get").expect("e").salience;
        assert!(after_sal < before_sal, "stale-path memory salience must demote");

        // 5. Transactional Dream SQLite lease blocks concurrent claim until released
        assert!(store.try_claim_dream_lease("owner_1", 120).expect("claim 1"));
        assert!(!store.try_claim_dream_lease("owner_2", 120).expect("claim 2 blocked"));
        store.release_dream_lease("owner_1").expect("release 1");
        assert!(store.try_claim_dream_lease("owner_2", 120).expect("claim 2 ok"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subagent_hierarchy_and_mailbox_persistence() {
        let dir = std::env::temp_dir().join(format!("zenko_p5_sql_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let parent = store
            .create_session("Lead", "webui", None, None, PermissionMode::Balanced)
            .expect("parent");
        let child = store
            .create_child_session(
                &parent.id,
                "architect",
                1,
                "Arch Child",
                "/tmp/repo",
                None,
                None,
                None,
                PermissionMode::Balanced,
            )
            .expect("child");
        let grandchild = store
            .create_child_session(
                &child.id,
                "explorer",
                2,
                "Explorer Leaf",
                "/tmp/repo",
                None,
                None,
                None,
                PermissionMode::Balanced,
            )
            .expect("grandchild");

        let now = Utc::now();
        store
            .upsert_subagent(&SubagentRecord {
                id: child.id.clone(),
                parent_session_id: parent.id.clone(),
                role: "architect".into(),
                depth: 1,
                goal: "Design auth".into(),
                status: SubagentStatus::Running,
                workspace_mode: SubagentWorkspaceMode::Inherit,
                workspace_dir: "/tmp/repo".into(),
                worktree_branch: None,
                worktree_base_commit: None,
                worktree_dirty_baseline: false,
                model: Some("test-model".into()),
                summary: None,
                output_schema: None,
                structured_output: None,
                diff_stat: None,
                changed_files: Vec::new(),
                tokens_used: 100,
                cost_usd: 0.002,
                tree_tokens: 0,
                tree_cost_usd: 0.0,
                descendant_count: 0,
                resume_count: 0,
                started_at: now,
                finished_at: None,
            })
            .expect("upsert child");
        store
            .upsert_subagent(&SubagentRecord {
                id: grandchild.id.clone(),
                parent_session_id: child.id.clone(),
                role: "explorer".into(),
                depth: 2,
                goal: "Scan auth files".into(),
                status: SubagentStatus::Completed,
                workspace_mode: SubagentWorkspaceMode::Inherit,
                workspace_dir: "/tmp/repo".into(),
                worktree_branch: None,
                worktree_base_commit: None,
                worktree_dirty_baseline: false,
                model: Some("fast-model".into()),
                summary: Some("Found src/auth.rs".into()),
                output_schema: None,
                structured_output: Some(serde_json::json!({"files": ["src/auth.rs"]})),
                diff_stat: None,
                changed_files: Vec::new(),
                tokens_used: 420,
                cost_usd: 0.001,
                tree_tokens: 0,
                tree_cost_usd: 0.0,
                descendant_count: 0,
                resume_count: 0,
                started_at: now,
                finished_at: Some(now),
            })
            .expect("upsert grandchild");

        let desc = store.list_descendant_session_ids(&parent.id).expect("desc");
        assert_eq!(desc.len(), 2);
        assert!(desc.contains(&child.id));
        assert!(desc.contains(&grandchild.id));

        let child_loaded = store.get_subagent(&child.id).expect("get child").expect("some");
        assert_eq!(child_loaded.tree_tokens, 520);
        assert_eq!(child_loaded.descendant_count, 1);
        assert!((child_loaded.tree_cost_usd - 0.003).abs() < 1e-9);

        let loaded = store.get_subagent(&grandchild.id).expect("get").expect("some");
        assert_eq!(loaded.structured_output.unwrap()["files"][0], "src/auth.rs");

        // Inter-agent mailbox atomic drain
        store
            .insert_agent_message(&parent.id, &child.id, "Focus on OAuth2 PKCE")
            .expect("mail");
        let drained = store.drain_unread_agent_messages(&child.id).expect("drain 1");
        assert_eq!(drained.len(), 1);
        assert!(drained[0].read_at.is_some());
        assert_eq!(drained[0].body, "Focus on OAuth2 PKCE");
        assert!(store.drain_unread_agent_messages(&child.id).expect("drain 2").is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_tasks_dag_atomic_claim_and_checkpoints_persistence() {
        let dir = std::env::temp_dir().join(format!("zenko_p6_sql_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let s = store
            .create_session("Phase 6 Test", "cli", None, None, PermissionMode::Plan)
            .expect("session");
        assert_eq!(s.permission_mode, PermissionMode::Plan);

        let now = Utc::now();
        let t1 = SessionTask {
            id: "t1".into(),
            session_id: s.id.clone(),
            branch_event_id: None,
            title: "Design schema".into(),
            description: "Write SQLite DDL".into(),
            active_label: Some("Designing schema".into()),
            status: SessionTaskStatus::Pending,
            depends_on: vec![],
            owner_agent: None,
            verification_cmd: Some("cargo test".into()),
            verification_evidence: None,
            notes: None,
            priority: 10,
            created_at: now,
            updated_at: now,
        };
        store.upsert_session_task(&t1).expect("upsert t1");

        // Atomic claim succeeds for first worker and rejects competing second worker
        let claimed = store
            .claim_session_task(&s.id, "t1", "coder_a")
            .expect("claim 1")
            .expect("claimed");
        assert_eq!(claimed.owner_agent.as_deref(), Some("coder_a"));
        assert_eq!(claimed.status, SessionTaskStatus::InProgress);
        assert!(
            store
                .claim_session_task(&s.id, "t1", "coder_b")
                .expect("claim 2")
                .is_none(),
            "competing worker must not steal claimed task"
        );

        // Workspace checkpoint insert, post-state update, and retention pruning
        let cp1 = WorkspaceCheckpoint {
            id: "cp_1".into(),
            session_id: s.id.clone(),
            turn_id: "turn_1".into(),
            event_id: None,
            workspace_dir: "/tmp/ws".into(),
            backend: CheckpointBackend::ShadowGit,
            pre_ref: "aaa111".into(),
            post_ref: None,
            trigger_tool: "write_file".into(),
            label: "Before write_file".into(),
            changed_files: vec![],
            is_safety_backup: false,
            created_at: now - Duration::seconds(10),
        };
        let cp2 = WorkspaceCheckpoint {
            id: "cp_2".into(),
            session_id: s.id.clone(),
            turn_id: "turn_2".into(),
            event_id: None,
            workspace_dir: "/tmp/ws".into(),
            backend: CheckpointBackend::ShadowGit,
            pre_ref: "bbb222".into(),
            post_ref: Some("ccc333".into()),
            trigger_tool: "shell_exec".into(),
            label: "Before shell_exec".into(),
            changed_files: vec!["src/main.rs".into()],
            is_safety_backup: false,
            created_at: now,
        };
        store.insert_checkpoint(&cp1).expect("insert cp1");
        store.insert_checkpoint(&cp2).expect("insert cp2");
        store
            .update_checkpoint_post_state("cp_1", "bbb222", &["README.md".into()])
            .expect("update post");
        let loaded = store.get_checkpoint("cp_1").expect("get").expect("some");
        assert_eq!(loaded.post_ref.as_deref(), Some("bbb222"));
        assert_eq!(loaded.changed_files, vec!["README.md"]);

        let pruned = store.prune_session_checkpoints(&s.id, 1).expect("prune");
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].id, "cp_1");
        assert_eq!(store.list_checkpoints(Some(&s.id), 10).expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn phase2_p1_regressions_pairing_scope_filter_ui_profile_and_secret_scrub() {
        let dir = std::env::temp_dir().join(format!("zenko_p1_mem_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open");

        // 1. Pairing code is strictly single-use
        let code = store
            .create_pairing_code("telegram", PeerRole::Owner, 15)
            .expect("create pairing");
        let first_peer = store
            .redeem_pairing_code("telegram", &code.code, "u1", None, None)
            .expect("first")
            .expect("first redeem succeeds");
        assert_eq!(first_peer.role, PeerRole::Owner);
        assert!(
            store
                .redeem_pairing_code("telegram", &code.code, "u2", None, None)
                .expect("second")
                .is_none()
        );

        // 2. Multi-project scope filter executes in SQL before LIMIT (no starvation)
        for i in 0..25 {
            store
                .add_memory("project:alpha", "fact", &format!("Alpha deploy {i}"), "rust deploy pipeline alpha", 0.9)
                .expect("alpha mem");
        }
        store
            .add_memory("project:beta", "fact", "Beta deploy", "rust deploy pipeline beta", 0.9)
            .expect("beta mem");
        let hits = store
            .search_memory_hybrid("rust deploy pipeline", Some("project:beta"), false, None, 5)
            .expect("hybrid beta");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.scope, "project:beta");

        // 3. update_memory_with_secrets scrubs vault secrets
        let secrets = vec!["super-secret-token-xyz987".to_string()];
        let updated = store
            .update_memory_with_secrets(
                &hits[0].entry.id,
                None,
                Some("deploy key is super-secret-token-xyz987"),
                None,
                None,
                None,
                &secrets,
            )
            .expect("update")
            .expect("some");
        assert!(!updated.content.contains("super-secret-token-xyz987"));

        // 4. activate_ui_profile with nonexistent id preserves existing active profile
        let active_before = store.get_active_ui_profile().expect("active");
        assert!(!store.activate_ui_profile("nonexistent_profile_id").expect("activate"));
        let active_after = store.get_active_ui_profile().expect("active after");
        assert_eq!(active_before.id, active_after.id);

        let _ = std::fs::remove_dir_all(dir);
    }
