use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};
use zenko_core::{
    JobSupervisor, LogChannel, LogDirection, LogStatusTone, SecretVault,
    VAULT_KEY_WEBUI_ADMIN, ZenkoConfig, emit_hono_line, find_provider,
};
use zenko_gateways::TelegramGateway;
use zenko_runtime::{
    AgentRuntime, LogBroadcastLayer, run_activity_logger, run_dream_worker,
    run_goal_loop_scheduler, run_workspace_watchers,
};

pub(crate) async fn run_web_daemon(config_path: PathBuf, port: Option<u16>, bind: Option<String>) -> Result<()> {
    let mut cfg = ZenkoConfig::load_or_default(&config_path)?;
    if let Some(p) = port {
        cfg.webui.port = p;
    }
    if let Some(b) = bind {
        cfg.webui.bind = b;
    }

    let (runtime, log_buf) = AgentRuntime::build(config_path, cfg.clone())?;

    let tracing_level = match cfg.general.log_level.as_str() {
        "debug" | "trace" => cfg.general.log_level.as_str(),
        _ => "warn",
    };
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{tracing_level},hyper=warn,reqwest=warn")));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).compact())
        .with(LogBroadcastLayer::new(log_buf))
        .init();

    if cfg.webui.require_auth && !runtime.vault.has(VAULT_KEY_WEBUI_ADMIN)? {
        let tok = SecretVault::generate_token(16);
        runtime.vault.set(VAULT_KEY_WEBUI_ADMIN, &tok)?;
        // Print generated admin token ONLY to operator stderr on first boot; never write plaintext
        // credentials into SQLite logs, JSONL files, in-memory ring buffers, or SSE broadcasts.
        eprintln!("  [vault] Initial WebUI admin token generated (saved to vault.enc): {tok}");
        let entry = emit_hono_line(
            LogDirection::Warn,
            LogChannel::System,
            "initial WebUI admin token generated and saved to vault.enc (printed once to stderr)",
            Some(("vault", LogStatusTone::Warn)),
            Some("saved"),
        );
        runtime.logs.push(entry);
    }

    let cancel = CancellationToken::new();
    let tg_reload = Arc::new(Notify::new());

    let act_rt = runtime.clone();
    let act_cancel = cancel.clone();
    tokio::spawn(async move {
        run_activity_logger(act_rt, act_cancel).await;
    });

    let cron_rt = runtime.clone();
    let cron_cancel = cancel.clone();
    tokio::spawn(async move {
        cron_rt.run_cron_scheduler(cron_cancel).await;
    });

    let wake_rt = runtime.clone();
    let wake_cancel = cancel.clone();
    tokio::spawn(async move {
        wake_rt.run_job_wakeup_loop(wake_cancel).await;
    });

    let dream_rt = runtime.clone();
    let dream_cancel = cancel.clone();
    tokio::spawn(async move {
        run_dream_worker(dream_rt, dream_cancel).await;
    });

    let goal_rt = runtime.clone();
    let goal_cancel = cancel.clone();
    tokio::spawn(async move {
        run_goal_loop_scheduler(goal_rt, goal_cancel).await;
    });

    let watch_rt = runtime.clone();
    let watch_cancel = cancel.clone();
    tokio::spawn(async move {
        run_workspace_watchers(watch_rt, watch_cancel).await;
    });

    let reaper_rt = runtime.clone();
    let reaper_cancel = cancel.clone();
    tokio::spawn(async move {
        reaper_rt.run_external_tool_reaper(reaper_cancel).await;
    });

    // Non-blocking startup sweep removing crash-orphaned git worktrees under ~/.zenko/worktrees/
    let prune_rt = runtime.clone();
    tokio::spawn(async move {
        if let Ok(rep) = prune_rt.subagents.prune_orphan_worktrees(&prune_rt).await
            && rep.removed_count > 0 {
                let entry = emit_hono_line(
                    LogDirection::Work,
                    LogChannel::System,
                    &format!("pruned {} orphan subagent worktree(s)", rep.removed_count),
                    Some(("worktree", LogStatusTone::Ok)),
                    Some("boot"),
                );
                prune_rt.logs.push(entry);
            }
    });

    let ipc_rt = runtime.clone();
    let ipc_cancel = cancel.clone();
    tokio::spawn(async move {
        ipc_rt.run_ipc_server(ipc_cancel).await;
    });

    let tg_gw = Arc::new(TelegramGateway::new(runtime.clone(), tg_reload.clone()));
    let tg_cancel = cancel.clone();
    tokio::spawn(async move {
        tg_gw.run(tg_cancel).await;
    });

    let sig_cancel = cancel.clone();
    let sig_runtime = runtime.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = signal(SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    if let Some(ref mut s) = sigterm {
                        s.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = emit_hono_line(
            LogDirection::Warn,
            LogChannel::System,
            "shutdown signal received",
            Some(("stopping", LogStatusTone::Warn)),
            None,
        );
        sig_runtime.cancel_all_sessions();
        sig_cancel.cancel();

        if tokio::signal::ctrl_c().await.is_ok() {
            std::process::exit(130);
        }
    });

    let provider_desc = find_provider(&cfg.provider.kind);
    let rss_mb = zenko_runtime::current_rss_bytes() as f64 / (1024.0 * 1024.0);
    let stream_meta = match cfg.provider.enable_streaming {
        true => "stream on",
        false => "stream off",
    };
    let (tg_status, tg_tone, tg_meta) = match cfg.telegram.enabled {
        true => ("enabled", LogStatusTone::Info, "polling"),
        false => ("idle", LogStatusTone::Neutral, "run `zenko setup`"),
    };

    for entry in [
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            &format!("webui    http://{}:{}", cfg.webui.bind, cfg.webui.port),
            Some(("ready", LogStatusTone::Ok)),
            Some(&format!("{rss_mb:.1} MB RSS")),
        ),
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            &format!(
                "provider {} · {}",
                provider_desc.label,
                match cfg.provider.lead_model.is_empty() {
                    true => "auto",
                    false => cfg.provider.lead_model.as_str(),
                }
            ),
            Some(("ready", LogStatusTone::Ok)),
            Some(stream_meta),
        ),
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            "telegram gateway",
            Some((tg_status, tg_tone)),
            Some(tg_meta),
        ),
    ] {
        runtime.logs.push(entry);
    }

    let flush_logs = runtime.logs.clone();
    let stop_jobs = runtime.jobs.clone();
    let stop_runtime = runtime.clone();
    let serve_res = match cfg.webui.enabled {
        true => zenko_webui::serve(runtime, tg_reload, cancel.clone()).await,
        false => {
            cancel.cancelled().await;
            Ok(())
        }
    };

    // Always cancel background tasks (cron/telegram/goal loops/IPC) and flush state even if webui bind failed
    cancel.cancel();
    stop_runtime.subagents.kill_all(&stop_runtime).await;
    stop_runtime.shutdown_external_tools().await;
    stop_jobs.stop_all().await;
    let stopped_entry = emit_hono_line(
        LogDirection::Out,
        LogChannel::System,
        "zenko web",
        Some(("stopped", LogStatusTone::Neutral)),
        None,
    );
    flush_logs.push(stopped_entry);
    flush_logs.flush();
    serve_res?;
    Ok(())
}
