use super::*;

pub(super) struct RuntimeContextBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeContextBridge {
    pub(super) fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Option<Arc<AgentRuntime>> {
        self.runtime.get().and_then(Weak::upgrade)
    }
}

#[async_trait]
impl ContextBridge for RuntimeContextBridge {
    fn search_and_activate_deferred_tools(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ToolSpec> {
        self.activate_deferred_by_scope(session_id, None, false, &[], Some(query), limit)
    }

    fn activate_deferred_by_scope(
        &self,
        session_id: &str,
        toolset: Option<ToolSet>,
        activate_all: bool,
        names: &[String],
        query: Option<&str>,
        limit: usize,
    ) -> Vec<ToolSpec> {
        let Some(rt) = self.upgrade() else {
            return Vec::new();
        };
        let scope = rt.effective_session_scope(session_id);
        let demoted = rt.economy_demoted_tools_for_session(session_id);
        let matched = rt.tools.select_deferred_by_scope_with_demotions(
            toolset,
            activate_all,
            names,
            query,
            limit,
            &demoted,
            &scope.availability,
            |spec| scope.permits_spec(spec),
        );
        let selected_names: Vec<String> = matched.iter().map(|s| s.name.clone()).collect();
        let newly = rt.activate_deferred_tools(session_id, &selected_names);
        if !newly.is_empty() {
            rt.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_TOOL_ACTIVATED.into(),
                tier: zenko_core::ExecutionTier::Standard,
                detail: format!("Activated deferred tool(s): {}", newly.join(", ")),
                model: None,
            });
        }
        matched
    }

    fn list_permitted_toolsets(&self, session_id: &str) -> Vec<ToolSetSummary> {
        let Some(rt) = self.upgrade() else {
            return Vec::new();
        };
        let scope = rt.effective_session_scope(session_id);
        let demoted = rt.economy_demoted_tools_for_session(session_id);
        rt.tools
            .list_toolsets_with_demotions(&demoted, &scope.availability, |spec| {
                scope.permits_spec(spec)
            })
    }

    async fn inspect_session_context(&self, session_id: &str) -> Result<ContextBreakdown> {
        let Some(rt) = self.upgrade() else {
            return Err(ZenkoError::Config(
                "runtime context bridge unavailable".into(),
            ));
        };
        rt.inspect_session_context(session_id).await
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge executing `batch_tools` through the runtime's
/// full role scope, schema coercion, hooks, ExecPolicy, LSP diagnostics, and 3-state wave scheduler.
pub(super) struct RuntimeBatchBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeBatchBridge {
    pub(super) fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime batch bridge unavailable".into()))
    }
}

#[async_trait]
impl ToolBatchBridge for RuntimeBatchBridge {
    async fn invoke_single(
        &self,
        ctx: &ToolContext,
        tool: String,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let rt = self.upgrade()?;
        let cfg = rt.config();
        let scope = rt.effective_session_scope(&ctx.session_id);
        let coerced_args = rt.coerce_tool_call_args(&tool, args);
        if let Err(denied) = scope.permits_invocation(&tool, &coerced_args) {
            return Ok(ToolOutput::err(denied));
        }
        let exec = rt
            .execute_single_tool_call(
                &cfg,
                &ctx.session_id,
                scope.permission,
                ctx,
                ctx.headless_no_wait,
                ToolCall {
                    id: format!("invoke_{}", new_id()),
                    name: tool,
                    arguments: coerced_args,
                },
                &cancel,
            )
            .await?;
        Ok(exec.output)
    }

    async fn execute_batch(
        &self,
        ctx: &ToolContext,
        req: BatchToolRequest,
        cancel: CancellationToken,
    ) -> Result<Vec<BatchToolStepOutcome>> {
        let rt = self.upgrade()?;
        let cfg = rt.config();
        let scope = rt.effective_session_scope(&ctx.session_id);
        let total = req.calls.len();
        let sequential = req.stop_on_error || zenko_tools::batch::batch_uses_dataflow(&req.calls);

        if sequential {
            let mut outcomes = Vec::with_capacity(total);
            let mut bound_vars: HashMap<String, String> = HashMap::new();
            let mut halted = false;
            for (idx, item) in req.calls.into_iter().enumerate() {
                if cancel.is_cancelled() {
                    return Err(ZenkoError::Cancelled);
                }
                if halted {
                    outcomes.push(BatchToolStepOutcome {
                        index: idx,
                        tool: item.tool,
                        is_error: false,
                        skipped: true,
                        quiet: item.quiet,
                        bound_var: item.bind_as,
                        duration_ms: 0,
                        content: "Skipped because an earlier batch step failed (`stop_on_error: true`).".into(),
                        diff: None,
                        shell_command: None,
                        raw_output_preview: None,
                        read_less_tokens_saved: 0,
                        output_dedup_tokens_saved: 0,
                        shunt_tokens_saved: 0,
                        ccr_recovery: false,
                    });
                    continue;
                }
                let interpolated =
                    zenko_tools::batch::interpolate_args_templates(item.args, &bound_vars);
                let coerced_args = rt.coerce_tool_call_args(&item.tool, interpolated);
                let shell_command = match item.tool.as_str() == TOOL_SHELL_EXEC {
                    true => coerced_args
                        .get("command")
                        .and_then(Value::as_str)
                        .map(String::from),
                    false => None,
                };
                if let Err(denied) = scope.permits_invocation(&item.tool, &coerced_args) {
                    outcomes.push(BatchToolStepOutcome {
                        index: idx,
                        tool: item.tool,
                        is_error: true,
                        skipped: false,
                        quiet: false,
                        bound_var: None,
                        duration_ms: 0,
                        content: denied,
                        diff: None,
                        shell_command: None,
                        raw_output_preview: None,
                        read_less_tokens_saved: 0,
                        output_dedup_tokens_saved: 0,
                        shunt_tokens_saved: 0,
                        ccr_recovery: false,
                    });
                    if req.stop_on_error {
                        halted = true;
                    }
                    continue;
                }
                let exec = rt
                    .execute_single_tool_call(
                        &cfg,
                        &ctx.session_id,
                        scope.permission,
                        ctx,
                        ctx.headless_no_wait,
                        ToolCall {
                            id: format!("batch_{}_{}", idx, new_id()),
                            name: item.tool,
                            arguments: coerced_args,
                        },
                        &cancel,
                    )
                    .await?;
                if exec.output.is_error && req.stop_on_error {
                    halted = true;
                }
                let raw_output_preview = match exec.tool_name.as_str() == TOOL_SHELL_EXEC {
                    true => Some(
                        exec.output
                            .content
                            .chars()
                            .take(ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS)
                            .collect(),
                    ),
                    false => None,
                };
                let output_dedup_tokens_saved = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("output_dedup"))
                    .and_then(|od| od.get("tokens_saved"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let read_less_tokens_saved = match output_dedup_tokens_saved > 0 {
                    true => 0,
                    false => exec
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("read_less"))
                        .and_then(|rl| rl.get("tokens_saved"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                };
                let shunt_tokens_saved = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get(zenko_core::SHUNT_META_KEY))
                    .and_then(|s| s.get(zenko_core::SHUNT_META_FIELD_TOKENS_SAVED))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let ccr_recovery = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("ccr_recovery"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let projected = match exec.output.is_error {
                    true => exec.output.content,
                    false => zenko_tools::batch::extract_step_output(
                        &exec.output.content,
                        item.extract.as_deref(),
                    ),
                };
                if !exec.output.is_error
                    && let Some(ref var_name) = item.bind_as
                {
                    bound_vars.insert(var_name.clone(), projected.clone());
                }
                outcomes.push(BatchToolStepOutcome {
                    index: idx,
                    tool: exec.tool_name,
                    is_error: exec.output.is_error,
                    skipped: false,
                    quiet: item.quiet,
                    bound_var: item.bind_as,
                    duration_ms: exec.duration_ms,
                    content: projected,
                    diff: exec.output.diff,
                    shell_command,
                    raw_output_preview,
                    read_less_tokens_saved,
                    output_dedup_tokens_saved,
                    shunt_tokens_saved,
                    ccr_recovery,
                });
            }
            return Ok(outcomes);
        }

        // Wave-parallel mode (zero dataflow dependencies): split scope-denied items immediately,
        // run permitted calls through the deterministic 3-state wave scheduler.
        let mut allowed: Vec<(usize, ToolCall, Option<String>, bool)> = Vec::with_capacity(total);
        let mut slots: Vec<Option<BatchToolStepOutcome>> = vec![None; total];
        for (idx, item) in req.calls.into_iter().enumerate() {
            let coerced_args = rt.coerce_tool_call_args(&item.tool, item.args);
            let tc = ToolCall {
                id: format!("batch_{}_{}", idx, new_id()),
                name: item.tool,
                arguments: coerced_args,
            };
            if let Err(denied) = scope.permits_invocation(&tc.name, &tc.arguments) {
                slots[idx] = Some(BatchToolStepOutcome {
                    index: idx,
                    tool: tc.name,
                    is_error: true,
                    skipped: false,
                    quiet: false,
                    bound_var: None,
                    duration_ms: 0,
                    content: denied,
                    diff: None,
                    shell_command: None,
                    raw_output_preview: None,
                    read_less_tokens_saved: 0,
                    output_dedup_tokens_saved: 0,
                    shunt_tokens_saved: 0,
                    ccr_recovery: false,
                });
            } else {
                allowed.push((idx, tc, item.extract, item.quiet));
            }
        }

        if !allowed.is_empty() {
            let calls_only: Vec<ToolCall> =
                allowed.iter().map(|(_, tc, _, _)| tc.clone()).collect();
            let executed = execute_tool_waves(
                calls_only,
                MAX_PARALLEL_TOOLS,
                |tc| rt.classify_tool_call(&cfg, scope.permission, tc),
                |tc| {
                    let rt_c = rt.clone();
                    let cfg_c = cfg.clone();
                    let ctx_c = ctx.clone();
                    let cancel_c = cancel.clone();
                    let perm = scope.permission;
                    async move {
                        rt_c.execute_single_tool_call(
                            &cfg_c,
                            &ctx_c.session_id,
                            perm,
                            &ctx_c,
                            ctx_c.headless_no_wait,
                            tc,
                            &cancel_c,
                        )
                        .await
                    }
                },
            )
            .await?;

            for ((idx, tc, extract_opt, quiet), exec) in allowed.into_iter().zip(executed) {
                let shell_command = match exec.tool_name.as_str() == TOOL_SHELL_EXEC {
                    true => tc
                        .arguments
                        .get("command")
                        .and_then(Value::as_str)
                        .map(String::from),
                    false => None,
                };
                let raw_output_preview = match exec.tool_name.as_str() == TOOL_SHELL_EXEC {
                    true => Some(
                        exec.output
                            .content
                            .chars()
                            .take(ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS)
                            .collect(),
                    ),
                    false => None,
                };
                let output_dedup_tokens_saved = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("output_dedup"))
                    .and_then(|od| od.get("tokens_saved"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let read_less_tokens_saved = match output_dedup_tokens_saved > 0 {
                    true => 0,
                    false => exec
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("read_less"))
                        .and_then(|rl| rl.get("tokens_saved"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                };
                let shunt_tokens_saved = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get(zenko_core::SHUNT_META_KEY))
                    .and_then(|s| s.get(zenko_core::SHUNT_META_FIELD_TOKENS_SAVED))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let ccr_recovery = exec
                    .output
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("ccr_recovery"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let projected = match exec.output.is_error {
                    true => exec.output.content,
                    false => zenko_tools::batch::extract_step_output(
                        &exec.output.content,
                        extract_opt.as_deref(),
                    ),
                };
                slots[idx] = Some(BatchToolStepOutcome {
                    index: idx,
                    tool: exec.tool_name,
                    is_error: exec.output.is_error,
                    skipped: false,
                    quiet,
                    bound_var: None,
                    duration_ms: exec.duration_ms,
                    content: projected,
                    diff: exec.output.diff,
                    shell_command,
                    raw_output_preview,
                    read_less_tokens_saved,
                    output_dedup_tokens_saved,
                    shunt_tokens_saved,
                    ccr_recovery,
                });
            }
        }

        Ok(slots.into_iter().flatten().collect())
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting Phase 5 subagent tools (`spawn_agent`,
/// `agent_list`, `agent_message`, `agent_wait`, `agent_kill`) to `SubagentSupervisor`.
pub(super) struct RuntimeSubagentBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeSubagentBridge {
    pub(super) fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime subagent bridge unavailable".into()))
    }
}

#[async_trait]
impl SubagentBridge for RuntimeSubagentBridge {
    async fn spawn_agent(&self, req: SubagentSpawnRequest) -> Result<SubagentRecord> {
        let rt = self.upgrade()?;
        rt.subagents.spawn_agent(&rt, req).await
    }

    fn list_agents(
        &self,
        parent_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.store.list_subagents(parent_session_id, limit)
    }

    fn get_agent(&self, agent_id: &str) -> Result<Option<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.store.get_subagent(agent_id)
    }

    async fn send_message(
        &self,
        from_session_id: &str,
        to_session_id: &str,
        message: &str,
        opts: zenko_core::AgentMessageOptions,
    ) -> Result<zenko_core::AgentMailMessage> {
        let rt = self.upgrade()?;
        rt.subagents
            .send_message(&rt, from_session_id, to_session_id, message, opts)
            .await
    }

    async fn wait_agents(
        &self,
        parent_session_id: &str,
        agent_ids: &[String],
        wait_all: bool,
        timeout: Duration,
    ) -> Result<Vec<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.subagents
            .wait_agents(&rt, parent_session_id, agent_ids, wait_all, timeout)
            .await
    }

    async fn kill_agent(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        remove_worktree: bool,
    ) -> Result<Option<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.subagents
            .kill_agent(&rt, caller_session_id, agent_id, remove_worktree)
            .await
    }

    async fn inspect_worktree_diff(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<Option<WorktreeDiffReport>> {
        let rt = self.upgrade()?;
        rt.subagents
            .inspect_worktree_diff(&rt, caller_session_id, agent_id)
            .await
    }

    async fn merge_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        cleanup: bool,
    ) -> Result<WorktreeDiffReport> {
        let rt = self.upgrade()?;
        rt.subagents
            .merge_worktree(&rt, caller_session_id, agent_id, cleanup)
            .await
    }

    async fn discard_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<WorktreeDiffReport> {
        let rt = self.upgrade()?;
        rt.subagents
            .discard_worktree(&rt, caller_session_id, agent_id)
            .await
    }

    async fn prune_orphan_worktrees(&self) -> Result<WorktreePruneReport> {
        let rt = self.upgrade()?;
        rt.subagents.prune_orphan_worktrees(&rt).await
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting Phase 8 `goal_loop` tool to Goal Loop CRUD & execution.
pub(super) struct RuntimeGoalLoopBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeGoalLoopBridge {
    pub(super) fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime goal loop bridge unavailable".into()))
    }
}

#[async_trait]
impl GoalLoopBridge for RuntimeGoalLoopBridge {
    fn list_goals(&self, limit: usize) -> Result<Vec<GoalLoopRecord>> {
        let rt = self.upgrade()?;
        rt.store.list_goal_loops(limit)
    }

    fn get_goal(&self, id: &str) -> Result<Option<(GoalLoopRecord, Vec<GoalLoopRun>)>> {
        let rt = self.upgrade()?;
        match rt.store.get_goal_loop(id)? {
            Some(g) => {
                let runs = rt
                    .store
                    .list_goal_loop_runs(&g.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                    .unwrap_or_default();
                Ok(Some((g, runs)))
            }
            None => Ok(None),
        }
    }

    fn create_goal(&self, req: GoalLoopCreateRequest) -> Result<GoalLoopRecord> {
        let rt = self.upgrade()?;
        rt.create_goal_loop(req)
    }

    fn update_goal(&self, id: &str, patch: GoalLoopPatch) -> Result<Option<GoalLoopRecord>> {
        let rt = self.upgrade()?;
        rt.update_goal_loop(id, patch)
    }

    async fn trigger_goal_now(&self, id: &str) -> Result<GoalLoopRecord> {
        let rt = self.upgrade()?;
        crate::goals::execute_goal_iteration(&rt, id, CancellationToken::new()).await
    }

    fn delete_goal(&self, id: &str) -> Result<bool> {
        let rt = self.upgrade()?;
        rt.delete_goal_loop(id)
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting the deferred `token_economy` tool in
/// `zenko-tools` to `AgentRuntime` without circular dependencies.
pub(super) struct RuntimeEconomyBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeEconomyBridge {
    pub(super) fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime economy bridge unavailable".into()))
    }
}

impl EconomyBridge for RuntimeEconomyBridge {
    fn economy_report(&self, workspace_dir: &Path) -> Result<TokenEconomyReport> {
        let rt = self.upgrade()?;
        rt.economy_report(Some(workspace_dir))
    }

    fn add_micro_skill(
        &self,
        workspace_dir: &Path,
        fact: &str,
        failed_cmd: Option<&str>,
        fixed_cmd: Option<&str>,
    ) -> Result<RepoMicroSkill> {
        let rt = self.upgrade()?;
        rt.economy_add_micro_skill(Some(workspace_dir), fact, failed_cmd, fixed_cmd)
    }

    fn retire_micro_skill(&self, id: &str) -> Result<bool> {
        let rt = self.upgrade()?;
        rt.economy_retire_micro_skill(id)
    }

    fn apply_tool_prune(&self, workspace_dir: &Path, tool_name: &str) -> Result<bool> {
        let rt = self.upgrade()?;
        rt.economy_apply_tool_prune(Some(workspace_dir), tool_name)
    }

    fn restore_tool(&self, workspace_dir: &Path, tool_name: &str, pin: bool) -> Result<bool> {
        let rt = self.upgrade()?;
        rt.economy_restore_tool(Some(workspace_dir), tool_name, pin)
    }

    fn reaudit_pruned_tools(&self, workspace_dir: &Path) -> Result<usize> {
        let rt = self.upgrade()?;
        rt.economy_reaudit_tools(Some(workspace_dir))
    }

    fn session_minimalism_mode(&self, session_id: &str) -> CodeMinimalismMode {
        let Ok(rt) = self.upgrade() else {
            return CodeMinimalismMode::default();
        };
        let cfg = rt.config();
        let branch = rt.store.get_active_branch(session_id).unwrap_or_default();
        rt.effective_minimalism_mode(session_id, &cfg, &branch)
    }

    fn update_repo_debt_count(&self, workspace_dir: &Path, debt_count: usize) -> Result<()> {
        let rt = self.upgrade()?;
        let scope = rt.resolve_repo_scope(Some(workspace_dir));
        rt.store.update_repo_debt_count(&scope, debt_count)
    }

    fn discover_missed_savings(
        &self,
        workspace_dir: &Path,
        limit: usize,
    ) -> Result<Vec<zenko_core::MissedSavingsOpportunity>> {
        let rt = self.upgrade()?;
        rt.economy_discover_missed_savings(Some(workspace_dir), limit)
    }

    fn verify_custom_filters(
        &self,
        workspace_dir: &Path,
    ) -> Result<zenko_core::CustomFilterVerifyReport> {
        let rt = self.upgrade()?;
        rt.economy_verify_custom_filters(Some(workspace_dir))
    }
}

