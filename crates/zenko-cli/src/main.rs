mod cli;
mod commands;
mod daemon;
mod debug;
mod ipc;
mod mcp;
mod service;
mod setup;

use anyhow::{Context, Result};
use clap::Parser;
use cli::*;
use commands::{AskParams, LogsParams};
use ipc::OutputMode;
use mimalloc::MiMalloc;
use std::time::Duration;
use zenko_core::{
    ApprovalDecision, DEFAULT_SHUTDOWN_GRACE_SECS, DEFAULT_TOKIO_BLOCKING_THREADS,
    DEFAULT_TOKIO_WORKERS, EXIT_OK, ZenkoConfig,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .config
        .unwrap_or_else(ZenkoConfig::default_config_path);
    let json_out = cli.json;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(DEFAULT_TOKIO_WORKERS)
        .max_blocking_threads(DEFAULT_TOKIO_BLOCKING_THREADS)
        .enable_all()
        .build()
        .context("failed to initialize tokio runtime")?;

    let exit_code: Result<i32> = rt.block_on(async move {
        match cli.command {
            Commands::Setup => {
                setup::run_interactive_setup(&config_path).await?;
                Ok(EXIT_OK)
            }
            Commands::Web { port, bind, info } => {
                match info {
                    true => commands::run_web_info(&config_path, json_out)?,
                    false => daemon::run_web_daemon(config_path, port, bind).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Status => commands::run_status(&config_path, json_out).await,
            Commands::Ask {
                prompt,
                session,
                workspace,
                model,
                mode,
                timeout,
                standalone,
                wait_approval,
                stream_json,
            } => {
                let output = match (stream_json, json_out) {
                    (true, _) => OutputMode::StreamJson,
                    (false, true) => OutputMode::Json,
                    (false, false) => OutputMode::Text,
                };
                commands::run_ask(
                    &config_path,
                    AskParams {
                        prompt,
                        session,
                        workspace,
                        model,
                        mode,
                        timeout_secs: timeout,
                        standalone,
                        wait_approval,
                        output,
                    },
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Chat {
                session,
                workspace,
                model,
                mode,
                standalone,
            } => {
                commands::run_chat(&config_path, session, workspace, model, mode, standalone)
                    .await?;
                Ok(EXIT_OK)
            }
            Commands::Attach { session_id, replay } => {
                commands::run_attach(&config_path, &session_id, replay, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Background { session_id } => {
                commands::run_session_background(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Sessions { action } => {
                match action {
                    SessionsCmd::List { limit } => {
                        commands::run_sessions_list(&config_path, limit, json_out).await?
                    }
                    SessionsCmd::Search {
                        query,
                        since,
                        channel,
                        workspace,
                        role,
                        limit,
                    } => {
                        commands::run_sessions_search(
                            &config_path,
                            &query,
                            since.as_deref(),
                            channel.as_deref(),
                            &workspace,
                            role.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    SessionsCmd::Show { id } => {
                        commands::run_session_show(&config_path, &id, json_out).await?
                    }
                    SessionsCmd::Clear { id } => {
                        commands::run_session_clear(&config_path, &id, json_out).await?
                    }
                    SessionsCmd::Delete { id } => {
                        commands::run_session_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Approvals { action } => {
                match action {
                    ApprovalsCmd::List => commands::run_approvals_list(&config_path, json_out).await?,
                    ApprovalsCmd::Approve { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::Approve,
                            json_out,
                        )
                        .await?
                    }
                    ApprovalsCmd::Always { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::AlwaysAllow,
                            json_out,
                        )
                        .await?
                    }
                    ApprovalsCmd::Deny { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::Deny,
                            json_out,
                        )
                        .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Memory { action } => {
                match action {
                    MemoryCmd::Search {
                        query,
                        scope,
                        limit,
                    } => {
                        commands::run_memory_search(&config_path, &query, &scope, limit, json_out)
                            .await?
                    }
                    MemoryCmd::Add {
                        title,
                        content,
                        kind,
                        scope,
                    } => {
                        commands::run_memory_add(
                            &config_path,
                            &title,
                            &content,
                            &kind,
                            &scope,
                            json_out,
                        )
                        .await?
                    }
                    MemoryCmd::Update {
                        id,
                        title,
                        content,
                        kind,
                        scope,
                    } => {
                        commands::run_memory_update(
                            &config_path,
                            &id,
                            title.as_deref(),
                            content.as_deref(),
                            kind.as_deref(),
                            scope.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    MemoryCmd::Delete { id } => {
                        commands::run_memory_delete(&config_path, &id, json_out).await?
                    }
                    MemoryCmd::Dream { force } => {
                        commands::run_memory_dream(&config_path, force, json_out).await?
                    }
                    MemoryCmd::History { doc } => {
                        commands::run_memory_history(&config_path, doc.as_deref(), json_out).await?
                    }
                    MemoryCmd::Rollback { snapshot_id } => {
                        commands::run_memory_rollback(&config_path, &snapshot_id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Cron { action } => {
                match action {
                    CronCmd::List => commands::run_cron_list(&config_path, json_out).await?,
                    CronCmd::Add {
                        name,
                        expr,
                        prompt,
                        script,
                        kind,
                    } => {
                        commands::run_cron_add(
                            &config_path,
                            &name,
                            &expr,
                            prompt.as_deref(),
                            script.as_deref(),
                            &kind,
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Edit {
                        id,
                        name,
                        expr,
                        prompt,
                        script,
                    } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            name.as_deref(),
                            expr.as_deref(),
                            prompt.as_deref(),
                            script.as_deref(),
                            None,
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Pause { id } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            None,
                            None,
                            None,
                            None,
                            Some(false),
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Resume { id } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            None,
                            None,
                            None,
                            None,
                            Some(true),
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Run { id } => {
                        commands::run_cron_run_now(&config_path, &id, json_out).await?
                    }
                    CronCmd::Delete { id } => {
                        commands::run_cron_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Tasks { action } => {
                match action {
                    JobsCmd::List {
                        session,
                        kind,
                        limit,
                    } => {
                        commands::run_jobs_list(
                            &config_path,
                            session.as_deref(),
                            kind.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Start {
                        kind,
                        label,
                        workspace,
                        command,
                    } => {
                        commands::run_jobs_start(
                            &config_path,
                            &command,
                            kind.as_deref(),
                            label.as_deref(),
                            workspace,
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Read {
                        id,
                        offset,
                        max_bytes,
                    } => {
                        commands::run_jobs_read(
                            &config_path,
                            &id,
                            offset,
                            max_bytes.unwrap_or_else(commands::default_job_read_bytes),
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Input { id, text } => {
                        commands::run_jobs_input(&config_path, &id, &text, json_out).await?
                    }
                    JobsCmd::Update { id, label, kind } => {
                        commands::run_jobs_update(
                            &config_path,
                            &id,
                            label.as_deref(),
                            kind.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Restart { id } => {
                        commands::run_jobs_restart(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Wait { id, timeout } => {
                        commands::run_jobs_wait(&config_path, &id, timeout, json_out).await?
                    }
                    JobsCmd::Follow { id } => {
                        commands::run_jobs_follow(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Stop { id } => {
                        commands::run_jobs_stop(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Delete { id } => {
                        commands::run_jobs_delete(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Prune { kind } => {
                        commands::run_jobs_prune(&config_path, kind.as_deref(), json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Agents { action } => {
                match action {
                    AgentsCmd::List { parent, limit } => {
                        commands::run_agents_list(&config_path, parent.as_deref(), limit, json_out)
                            .await?
                    }
                    AgentsCmd::Show { id } => {
                        commands::run_agents_show(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Spawn {
                        role,
                        goal,
                        parent,
                        workspace,
                        model,
                        bg,
                        fork_context,
                        include_uncommitted,
                        max_duration,
                        max_cost,
                    } => {
                        commands::run_agents_spawn(
                            &config_path,
                            &role,
                            &goal.join(" "),
                            parent.as_deref(),
                            workspace.as_deref(),
                            model.as_deref(),
                            bg,
                            fork_context,
                            include_uncommitted,
                            max_duration,
                            max_cost,
                            json_out,
                        )
                        .await?
                    }
                    AgentsCmd::Wait { id, timeout } => {
                        commands::run_agents_wait(&config_path, &id, timeout, json_out).await?
                    }
                    AgentsCmd::Message {
                        id,
                        message,
                        no_resume,
                        interrupt,
                    } => {
                        commands::run_agents_message(
                            &config_path,
                            &id,
                            &message.join(" "),
                            !no_resume,
                            interrupt,
                            json_out,
                        )
                        .await?
                    }
                    AgentsCmd::Diff { id } => {
                        commands::run_agents_diff(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Merge { id, keep } => {
                        commands::run_agents_merge(&config_path, &id, !keep, json_out).await?
                    }
                    AgentsCmd::Discard { id } => {
                        commands::run_agents_discard(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Prune => commands::run_agents_prune(&config_path, json_out).await?,
                    AgentsCmd::Kill { id, keep_worktree } => {
                        commands::run_agents_kill(&config_path, &id, !keep_worktree, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Hub { action } => {
                match action {
                    HubCmd::List { category, limit } => {
                        commands::run_hub_list(&config_path, category.as_deref(), limit, json_out)
                            .await?
                    }
                    HubCmd::Path => commands::run_hub_path(&config_path, json_out)?,
                }
                Ok(EXIT_OK)
            }
            Commands::Queue { action } => {
                match action {
                    QueueCmd::List { session_id } => {
                        commands::run_queue_list(&config_path, &session_id, json_out).await?
                    }
                    QueueCmd::Cancel {
                        session_id,
                        queue_id,
                    } => {
                        commands::run_queue_cancel(&config_path, &session_id, &queue_id, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Clarify { action } => {
                match action {
                    ClarifyCmd::List { session } => {
                        commands::run_clarifications_list(
                            &config_path,
                            session.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    ClarifyCmd::Answer {
                        id,
                        option_id,
                        custom,
                    } => {
                        commands::run_clarification_answer(
                            &config_path,
                            &id,
                            &option_id,
                            custom.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Steer {
                session_id,
                message,
            } => {
                commands::run_session_steer(&config_path, &session_id, &message, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Logs {
                limit,
                category,
                channel,
                level,
                session,
                search,
                follow,
            } => {
                commands::run_logs(
                    &config_path,
                    LogsParams {
                        limit,
                        category,
                        channel,
                        level,
                        session,
                        search,
                        follow,
                        json_out,
                    },
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Trace { session_id } => {
                debug::run_trace(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Context { session_id } => {
                commands::run_session_context(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Undo {
                session_id,
                checkpoint,
                mode,
                files,
                force,
                no_summary,
            } => {
                commands::run_undo(
                    &config_path,
                    &session_id,
                    checkpoint.as_deref(),
                    &mode,
                    files,
                    force,
                    !no_summary,
                    json_out,
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Checkpoints { action } => {
                match action {
                    CheckpointsCmd::List { session, limit } => {
                        commands::run_checkpoints_list(
                            &config_path,
                            session.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    CheckpointsCmd::Diff { checkpoint_id } => {
                        commands::run_checkpoints_diff(&config_path, &checkpoint_id, json_out)
                            .await?
                    }
                    CheckpointsCmd::Rewind {
                        checkpoint_id,
                        mode,
                        files,
                        force,
                        no_summary,
                    } => {
                        commands::run_checkpoints_rewind(
                            &config_path,
                            &checkpoint_id,
                            &mode,
                            files,
                            force,
                            !no_summary,
                            json_out,
                        )
                        .await?
                    }
                    CheckpointsCmd::Prune => {
                        commands::run_checkpoints_prune(&config_path, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Dag { action } => {
                match action {
                    DagCmd::List { session_id } => {
                        commands::run_dag_list(&config_path, &session_id, false, json_out).await?
                    }
                    DagCmd::Ready { session_id } => {
                        commands::run_dag_list(&config_path, &session_id, true, json_out).await?
                    }
                    DagCmd::Create {
                        session_id,
                        title,
                        id,
                        depends_on,
                        verify,
                        priority,
                    } => {
                        commands::run_dag_create(
                            &config_path,
                            &session_id,
                            &title,
                            id.as_deref(),
                            depends_on,
                            verify.as_deref(),
                            priority,
                            json_out,
                        )
                        .await?
                    }
                    DagCmd::Update {
                        session_id,
                        task_id,
                        status,
                        owner,
                        evidence,
                        notes,
                    } => {
                        commands::run_dag_update(
                            &config_path,
                            &session_id,
                            &task_id,
                            status.as_deref(),
                            owner.as_deref(),
                            evidence.as_deref(),
                            notes.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    DagCmd::Claim {
                        session_id,
                        task_id,
                        owner,
                    } => {
                        commands::run_dag_claim(&config_path, &session_id, &task_id, &owner, json_out)
                            .await?
                    }
                    DagCmd::Delete {
                        session_id,
                        task_id,
                    } => {
                        commands::run_dag_delete(&config_path, &session_id, Some(&task_id), json_out)
                            .await?
                    }
                    DagCmd::Clear { session_id } => {
                        commands::run_dag_delete(&config_path, &session_id, None, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Plan { action } => {
                match action {
                    PlanCmd::Approve { session_id, mode } => {
                        commands::run_plan_approve(&config_path, &session_id, &mode, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Debug { action } => {
                match action {
                    DebugCmd::Tool {
                        name,
                        args,
                        workspace,
                    } => {
                        debug::run_debug_tool(
                            &config_path,
                            &name,
                            args.as_deref(),
                            workspace,
                            json_out,
                        )
                        .await?
                    }
                    DebugCmd::Policy { command, mode } => {
                        debug::run_debug_policy(&config_path, &command, mode.as_deref(), json_out)?
                    }
                    DebugCmd::Triage { prompt, mode } => {
                        debug::run_debug_triage(&config_path, &prompt, mode.as_deref(), json_out)?
                    }
                    DebugCmd::Prompt { workspace, full } => {
                        debug::run_debug_prompt(&config_path, workspace, full, json_out)?
                    }
                    DebugCmd::LogsDir => debug::run_debug_logs_dir(&config_path, json_out)?,
                    DebugCmd::Bundle { output, limit } => {
                        debug::run_debug_bundle(&config_path, output, limit, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Service { action } => {
                match action {
                    ServiceCmd::Unit => service::run_service_unit(&config_path, json_out)?,
                    ServiceCmd::Install { enable } => {
                        service::run_service_install(&config_path, enable, json_out)?
                    }
                    ServiceCmd::Status => service::run_service_status(json_out)?,
                }
                Ok(EXIT_OK)
            }
            Commands::Gateway { action } => {
                match action {
                    GatewayCmd::Pair { role } => {
                        commands::run_gateway_pair(&config_path, &role, json_out).await?
                    }
                    GatewayCmd::Peers => commands::run_gateway_peers(&config_path, json_out).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Doctor => {
                commands::run_doctor(&config_path, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Mcp => {
                mcp::run_mcp_stdio(config_path).await?;
                Ok(EXIT_OK)
            }
            Commands::McpClient { action } => {
                match action {
                    McpClientCmd::List => {
                        commands::run_mcp_client_list(&config_path, json_out).await?
                    }
                    McpClientCmd::Add(boxed) => {
                        let McpClientAddArgs {
                            name,
                            transport,
                            command,
                            args,
                            url,
                            auth_secret,
                            env_secrets,
                            enabled_tools,
                            disabled_tools,
                            risk,
                            parallel,
                            allow_private_url,
                            sync,
                        } = *boxed;
                        commands::run_mcp_client_add(
                            &config_path,
                            commands::McpClientAddParams {
                                name,
                                transport,
                                command,
                                args,
                                url,
                                auth_secret,
                                env_secrets,
                                enabled_tools,
                                disabled_tools,
                                risk,
                                parallel,
                                allow_private_url,
                                sync,
                            },
                            json_out,
                        )
                        .await?
                    }
                    McpClientCmd::Remove { name } => {
                        commands::run_mcp_client_remove(&config_path, &name, json_out).await?
                    }
                    McpClientCmd::Toggle { name, enabled } => {
                        commands::run_mcp_client_toggle(&config_path, &name, enabled, json_out)
                            .await?
                    }
                    McpClientCmd::Sync { name } => {
                        commands::run_mcp_client_sync(&config_path, &name, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Lsp { action } => {
                match action {
                    LspCmd::Status => commands::run_lsp_status(&config_path, json_out).await?,
                    LspCmd::Stop => commands::run_lsp_stop(&config_path, json_out).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Skills { action } => {
                match action {
                    SkillsCmd::List => commands::run_skills_list(&config_path, json_out).await?,
                    SkillsCmd::Show { slug, file } => {
                        commands::run_skill_show(&config_path, &slug, file.as_deref(), json_out)
                            .await?
                    }
                    SkillsCmd::Create {
                        slug,
                        name,
                        description,
                        content,
                    } => {
                        commands::run_skill_create(
                            &config_path,
                            &slug,
                            name.as_deref(),
                            &description,
                            &content,
                            json_out,
                        )
                        .await?
                    }
                    SkillsCmd::Toggle { slug, enabled } => {
                        commands::run_skill_toggle(&config_path, &slug, enabled, json_out).await?
                    }
                    SkillsCmd::Approve { slug } => {
                        commands::run_skill_approve(&config_path, &slug, json_out).await?
                    }
                    SkillsCmd::History { slug } => {
                        commands::run_skill_history(&config_path, slug.as_deref(), json_out).await?
                    }
                    SkillsCmd::Rollback { snapshot_id } => {
                        commands::run_skill_rollback(&config_path, &snapshot_id, json_out).await?
                    }
                    SkillsCmd::Delete { slug } => {
                        commands::run_skill_delete(&config_path, &slug, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Goals { action } => {
                match action {
                    GoalsCmd::List { limit } => {
                        commands::run_goals_list(&config_path, limit, json_out).await?
                    }
                    GoalsCmd::Show { id } => {
                        commands::run_goal_show(&config_path, &id, json_out).await?
                    }
                    GoalsCmd::Create {
                        name,
                        goal,
                        verify_cmd,
                        workspace,
                        interval,
                        max_iterations,
                        max_budget,
                        stalls,
                        auto_rewind,
                        model,
                    } => {
                        commands::run_goal_create(
                            &config_path,
                            zenko_core::GoalLoopCreateRequest {
                                name,
                                goal_text: goal,
                                verify_cmd,
                                workspace_dir: workspace.map(|p| p.display().to_string()),
                                session_id: None,
                                interval_secs: interval,
                                max_iterations,
                                max_budget_usd: max_budget,
                                max_consecutive_stalls: stalls,
                                checkpoint_each_iter: Some(true),
                                auto_rewind_on_stall: Some(auto_rewind),
                                model,
                                notify_channel: None,
                            },
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Pause { id } => {
                        commands::run_goal_set_status(
                            &config_path,
                            &id,
                            zenko_core::GoalLoopStatus::Paused,
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Resume { id } => {
                        commands::run_goal_set_status(
                            &config_path,
                            &id,
                            zenko_core::GoalLoopStatus::Active,
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Run { id } => {
                        commands::run_goal_run_now(&config_path, &id, json_out).await?
                    }
                    GoalsCmd::Delete { id } => {
                        commands::run_goal_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Triggers { action } => {
                match action {
                    TriggersCmd::List => {
                        commands::run_triggers_list(&config_path, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Export {
                session_id,
                all,
                format,
                since,
                channel,
                verified_only,
                min_turns,
                include_thinking,
                anonymize_paths,
                all_branches,
                output,
            } => {
                commands::run_export(
                    &config_path,
                    commands::ExportCliParams {
                        session_id,
                        all,
                        format,
                        since,
                        channel,
                        verified_only,
                        min_turns,
                        include_thinking,
                        anonymize_paths,
                        all_branches,
                        output,
                    },
                    json_out,
                )
                .await?;
                Ok(EXIT_OK)
            }
        }
    });

    rt.shutdown_timeout(Duration::from_secs(DEFAULT_SHUTDOWN_GRACE_SECS));
    match exit_code {
        Ok(EXIT_OK) => Ok(()),
        Ok(code) => std::process::exit(code),
        Err(e) => Err(e),
    }
}

