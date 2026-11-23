pub mod render;
pub mod state;
pub mod theme;

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::collections::BTreeMap;
use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;
use zeroize::Zeroizing;
use zenko_core::{
    DEFAULT_PAIRING_TTL_MINS, DEFAULT_THEME_PRESET, GATEWAY_REGISTRY, LlmProvider, LogChannel,
    LogDirection, LogStatusTone, PROVIDER_REGISTRY, PeerRole, SecretVault, UiCustomization,
    UiProfile, VAULT_KEY_TELEGRAM, VAULT_KEY_WEBHOOK_SECRET, VAULT_KEY_WEBUI_ADMIN, ZenkoConfig,
    find_provider, format_hono_entry,
};
use zenko_gateways::verify_bot_token;
use zenko_memory::{SnapshotManager, SqliteStore};
use zenko_providers::UnifiedLlmProvider;
use zenko_runtime::record_standalone_log;

use crate::setup::render::draw_setup;
use crate::setup::state::{MachineOutcome, SetupMachine};
use crate::setup::theme::SetupTheme;

/// RAII terminal guard guaranteeing raw mode, paste mode, and alternate screen are restored on exit.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

pub async fn run_interactive_setup(config_path: &Path) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data_dir = cfg.resolved_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
    let store = SqliteStore::open(cfg.db_path())?;
    let snapshots = SnapshotManager::new(&data_dir, cfg.resolved_workspace_dir());
    snapshots.ensure_defaults()?;

    let mut existing_secrets = BTreeMap::new();
    for p in PROVIDER_REGISTRY {
        existing_secrets.insert(
            p.vault_secret_key.to_string(),
            vault.has(p.vault_secret_key).unwrap_or(false),
        );
    }
    for g in GATEWAY_REGISTRY {
        existing_secrets.insert(
            g.vault_secret_key.to_string(),
            vault.has(g.vault_secret_key).unwrap_or(false),
        );
    }

    let theme = store
        .get_active_ui_profile()
        .map(|p| SetupTheme::from_customization(&p.config))
        .unwrap_or_else(|_| SetupTheme::default_zenko());

    let mut machine = SetupMachine::new(cfg, existing_secrets);

    let outcome = {
        let mut guard = TerminalGuard::enter()?;
        run_event_loop(&mut guard.terminal, &mut machine, &theme)?
    };

    match outcome {
        MachineOutcome::CancelAndExit => {
            record_standalone_log(
                &data_dir,
                &store,
                &vault,
                format_hono_entry(
                    LogDirection::Warn,
                    LogChannel::Tui,
                    "setup wizard cancelled",
                    Some(("cancelled", LogStatusTone::Warn)),
                    None,
                ),
            );
            println!("Setup cancelled — no changes written.");
            Ok(())
        }
        MachineOutcome::SaveAndExit | MachineOutcome::Continue => {
            persist_and_verify(config_path, &data_dir, &vault, &store, machine).await
        }
    }
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    machine: &mut SetupMachine,
    theme: &SetupTheme,
) -> Result<MachineOutcome> {
    loop {
        terminal.draw(|frame| draw_setup(frame, machine, theme))?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }

        match event::read()? {
            Event::Paste(text) => machine.paste_text(&text),
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(MachineOutcome::CancelAndExit);
                }

                let outcome = match (machine.is_input_mode(), key.code) {
                    (true, KeyCode::Enter) => machine.confirm(),
                    (true, KeyCode::Esc) => machine.go_back(),
                    (true, KeyCode::Backspace) => {
                        machine.backspace();
                        MachineOutcome::Continue
                    }
                    (true, KeyCode::Char(c))
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        machine.push_char(c);
                        MachineOutcome::Continue
                    }
                    (false, KeyCode::Up | KeyCode::Char('k')) => {
                        machine.move_cursor(-1);
                        MachineOutcome::Continue
                    }
                    (false, KeyCode::Down | KeyCode::Char('j')) => {
                        machine.move_cursor(1);
                        MachineOutcome::Continue
                    }
                    (false, KeyCode::Enter | KeyCode::Char(' ')) => machine.confirm(),
                    (false, KeyCode::Esc | KeyCode::Left | KeyCode::Backspace) => machine.go_back(),
                    (false, KeyCode::Char('q')) => MachineOutcome::CancelAndExit,
                    _ => MachineOutcome::Continue,
                };

                if outcome != MachineOutcome::Continue {
                    return Ok(outcome);
                }
            }
            _ => {}
        }
    }
}

async fn persist_and_verify(
    config_path: &Path,
    data_dir: &Path,
    vault: &SecretVault,
    store: &SqliteStore,
    machine: SetupMachine,
) -> Result<()> {
    let mut cfg = machine.draft.config;
    for (k, v) in &machine.draft.pending_secrets {
        if !v.trim().is_empty() {
            vault.set(k, v.trim())?;
        }
    }

    // Ensure WebUI admin token exists when auth enabled
    let admin_token = match cfg.webui.require_auth {
        true => match vault.get(VAULT_KEY_WEBUI_ADMIN)? {
            Some(existing) if !existing.trim().is_empty() => Some(existing),
            _ => {
                let tok = SecretVault::generate_token(16);
                vault.set(VAULT_KEY_WEBUI_ADMIN, &tok)?;
                Some(Zeroizing::new(tok))
            }
        },
        false => None,
    };

    // Auto-generate webhook signing secret if webhooks enabled without one
    if cfg.webhooks.enabled && !vault.has(VAULT_KEY_WEBHOOK_SECRET).unwrap_or(false) {
        let secret = SecretVault::generate_token(24);
        vault.set(VAULT_KEY_WEBHOOK_SECRET, &secret)?;
    }

    // Ensure Obsidian Vermilion default UI profile exists
    if store.list_ui_profiles()?.is_empty() {
        store.save_ui_profile(&UiProfile {
            id: "default".into(),
            name: "Zenko Vermilion".into(),
            is_active: true,
            config: UiCustomization::preset(DEFAULT_THEME_PRESET),
            updated_at: Utc::now(),
        })?;
    }

    // Optional live connectivity check (non-blocking if offline)
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .build()?;
    let provider_desc = find_provider(&cfg.provider.kind);
    let provider_key = vault.get(provider_desc.vault_secret_key)?;
    let probe = UnifiedLlmProvider::for_provider(
        http.clone(),
        &cfg.provider.kind,
        cfg.provider.base_url.clone(),
        provider_key.clone(),
    );
    let provider_status = match provider_key {
        Some(_) => match probe.verify_key().await {
            Ok(()) => "verified".to_string(),
            Err(e) => format!("saved (verify skipped: {e})"),
        },
        None => match provider_desc.requires_api_key {
            true => "key pending".to_string(),
            false => "local endpoint".to_string(),
        },
    };

    // Auto-discover top live working model (free-first) when user left model blank
    if cfg.provider.lead_model.trim().is_empty()
        && let Ok(discovered) = probe.list_models().await
            && let Some(best) = discovered.first() {
                cfg.provider.lead_model = best.id.clone();
                if cfg.provider.fast_model.trim().is_empty() {
                    cfg.provider.fast_model = best.id.clone();
                    cfg.provider.compaction_model = best.id.clone();
                    cfg.provider.dream_model = best.id.clone();
                }
            }

    let mut pair_code: Option<String> = None;
    if cfg.telegram.enabled {
        if let Ok(Some(tok)) = vault.get(VAULT_KEY_TELEGRAM)
            && let Ok(info) = verify_bot_token(&http, &tok).await {
                cfg.telegram.bot_username = info.username;
            }
        let code = store.create_pairing_code("telegram", PeerRole::Owner, DEFAULT_PAIRING_TTL_MINS)?;
        pair_code = Some(code.code);
    }

    cfg.general.setup_completed = true;
    cfg.save(config_path)?;

    record_standalone_log(
        data_dir,
        store,
        vault,
        format_hono_entry(
            LogDirection::Out,
            LogChannel::Tui,
            &format!("setup saved ({} · {})", provider_desc.label, cfg.provider.lead_model),
            Some(("ok", LogStatusTone::Ok)),
            Some(&provider_status),
        )
        .with_payload(serde_json::json!({
            "provider": cfg.provider.kind,
            "model": cfg.provider.lead_model,
            "telegram_enabled": cfg.telegram.enabled,
            "webhooks_enabled": cfg.webhooks.enabled,
            "streaming": cfg.provider.enable_streaming,
        })),
    );

    println!("\n\x1b[38;2;234;88;41m▲\x1b[0m \x1b[1mZenko Setup Saved\x1b[0m");
    println!("  Provider:   {} ({}) — {provider_status}", provider_desc.label, cfg.provider.lead_model);
    println!("  Base URL:   {}", cfg.provider.base_url);
    println!(
        "  Streaming:  SSE={} · Cache={} · Sticky={} · TG Drafts={}",
        on_off(cfg.provider.enable_streaming),
        on_off(cfg.provider.enable_prompt_caching),
        on_off(cfg.provider.sticky_session_routing),
        on_off(cfg.telegram.stream_drafts),
    );
    println!(
        "  Gateways:   Telegram={} · Webhooks={}",
        on_off(cfg.telegram.enabled),
        on_off(cfg.webhooks.enabled),
    );
    println!("  WebUI:      http://{}:{}", cfg.webui.bind, cfg.webui.port);
    if let Some(tok) = admin_token {
        println!("  Admin Tok:  {}", tok.as_str());
    }
    if let Some(code) = pair_code {
        println!("  TG Pair:    Send \x1b[1m/pair {code}\x1b[0m to your Telegram bot");
    }
    println!("\nStart Zenko WebUI & 24/7 daemon with:\n  zenko web\n");
    Ok(())
}

fn on_off(v: bool) -> &'static str {
    match v {
        true => "ON",
        false => "OFF",
    }
}
