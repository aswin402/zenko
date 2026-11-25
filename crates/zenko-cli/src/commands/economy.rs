//! CLI subcommands for Zenko's native Token Economy, Missed-Savings Discovery (`discover`),
//! and SHA-256 Trust-Gated Custom Filters (`.zenko/filters.toml`).

use super::*;
use zenko_runtime::read_less::custom_filters::{
    project_filter_path, trust_project_filter_file, verify_filter_file_at,
};

fn resolve_ws_and_store(
    config_path: &Path,
    workspace: Option<PathBuf>,
) -> Result<(ZenkoConfig, PathBuf, String, SqliteStore)> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let ws = workspace.unwrap_or_else(|| cfg.resolved_workspace_dir());
    let repo_root = resolve_git_root(&ws);
    let scope = project_scope_for_workspace(&repo_root);
    let store = SqliteStore::open(cfg.resolved_data_dir().join("zenko.db"))?;
    Ok((cfg, ws, scope, store))
}

pub async fn run_economy_gain(
    config_path: &Path,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<()> {
    let (cfg, ws, scope, store) = resolve_ws_and_store(config_path, workspace)?;
    let report = zenko_runtime::economy::build_economy_report(
        &store,
        &cfg.economy,
        &scope,
        &cfg.resolved_data_dir(),
        &ws,
    )?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let s = &report.summary;
    println!("Zenko Token Economy ({}) · ZMode: {}", report.repo_scope, cfg.context.zmode.as_str());
    println!(
        "  Net Saved     : {} tok (~${:.4})",
        s.net_tokens_saved, s.estimated_usd_saved
    );
    println!(
        "  Breakdown     : Read-Less +{} · Dedup +{} ({} hits) · Tool-Prune +{} · Overhead -{}",
        s.read_less_tokens_saved,
        s.output_dedup_tokens_saved,
        s.output_dedup_hits,
        s.tool_prune_tokens_saved,
        s.overhead_tokens
    );
    println!(
        "  Health        : {} CCR recovery read(s) · {} raw bypass(es)",
        s.ccr_recoveries, s.raw_bypasses
    );
    if !report.by_command.is_empty() {
        println!("  By Command    :");
        for c in &report.by_command {
            println!(
                "    - {:<10} +{} tok ({} calls, {} raw)",
                c.command_family, c.tokens_saved, c.calls, c.raw_bypasses
            );
        }
    }
    let fs = &report.filter_status;
    let filter_state = match (fs.project_file_exists, fs.trusted) {
        (true, true) => format!("trusted ({} rule(s))", fs.rule_count),
        (true, false) => format!("UNTRUSTED ({} rule(s)) — run `zenko economy trust`", fs.rule_count),
        (false, _) => "none".to_string(),
    };
    println!(
        "  Custom Filters: project={filter_state} · user-global={} rule(s)",
        fs.user_rule_count
    );
    Ok(())
}

pub async fn run_economy_discover(
    config_path: &Path,
    workspace: Option<PathBuf>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let (_cfg, ws, _scope, store) = resolve_ws_and_store(config_path, workspace)?;
    let items = store.discover_missed_savings(Some(&ws), limit)?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("No large uncompressed shell/git command families found in recent sessions.");
        return Ok(());
    }

    println!(
        "Discovered {} missed-savings command opportunit(y/ies):",
        items.len()
    );
    for item in &items {
        println!(
            "  - {:<24} {:>3} call(s) · {:>8} B raw · ~{} missed tok",
            item.command_prefix, item.calls, item.total_bytes, item.estimated_missed_tokens
        );
    }
    println!("\nSuggested starter rule for `.zenko/filters.toml` (schema_version = 1):");
    println!("schema_version = 1\n");
    if let Some(top) = items.first() {
        print!("{}", top.suggested_filter_toml);
    }
    Ok(())
}

pub async fn run_economy_trust(
    config_path: &Path,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<()> {
    let (_cfg, ws, scope, store) = resolve_ws_and_store(config_path, workspace)?;
    let (sha256, report) = trust_project_filter_file(&store, &scope, &ws)
        .map_err(|msg| anyhow::anyhow!("{msg}"))?;

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "sha256": sha256,
                "verify": report,
            }))?
        );
        return Ok(());
    }

    println!(
        "Trusted {} ({} rule(s), {} inline test(s) passed)\n  sha256: {}",
        report.file_path, report.rule_count, report.tests_passed, sha256
    );
    Ok(())
}

pub async fn run_economy_untrust(
    config_path: &Path,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<()> {
    let (_cfg, _ws, scope, store) = resolve_ws_and_store(config_path, workspace)?;
    let removed = store.untrust_filter_file(&scope)?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "removed": removed }))?);
        return Ok(());
    }

    match removed {
        true => println!("Revoked custom filter trust for `{scope}`."),
        false => println!("No trusted project filter record existed for `{scope}`."),
    }
    Ok(())
}

pub async fn run_economy_verify_filters(
    config_path: &Path,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<i32> {
    let (_cfg, ws, _scope, _store) = resolve_ws_and_store(config_path, workspace)?;
    let path = project_filter_path(&ws);
    let report = verify_filter_file_at(&path);

    if json_out {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(match report.valid {
            true => EXIT_OK,
            false => 1,
        });
    }

    if !report.exists {
        println!("No project filter file found at {}.", report.file_path);
        return Ok(EXIT_OK);
    }

    println!(
        "Filter file {} · valid={} · {} rule(s) · {} test(s) passed, {} failed",
        report.file_path,
        report.valid,
        report.rule_count,
        report.tests_passed,
        report.tests_failed
    );
    for err in &report.errors {
        eprintln!("  error: {err}");
    }
    for fail in &report.test_failures {
        eprintln!(
            "  FAIL [{}::{}]: expected {:?}, got {:?}",
            fail.filter_name, fail.test_name, fail.expected, fail.actual
        );
    }
    Ok(match report.valid {
        true => EXIT_OK,
        false => 1,
    })
}
