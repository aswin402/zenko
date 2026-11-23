//! Linux `systemd --user` service unit generator and installer for 24/7 operation.

use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use zenko_core::SYSTEMD_UNIT_NAME;

fn escape_systemd_quoted_arg(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '%' => out.push_str("%%"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

pub fn render_systemd_unit(config_path: &Path) -> Result<String> {
    let exe = std::env::current_exe().context("failed to resolve current zenko executable path")?;
    let cfg_abs = match config_path.is_absolute() {
        true => config_path.to_path_buf(),
        false => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(config_path),
    };
    let exe_escaped = escape_systemd_quoted_arg(&exe.display().to_string());
    let cfg_escaped = escape_systemd_quoted_arg(&cfg_abs.display().to_string());
    Ok(format!(
        "[Unit]\n\
Description=Zenko 24/7 Autonomous AI Agent Daemon\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
Type=simple\n\
Environment=\"MIMALLOC_PURGE_DELAY=10\" \"MIMALLOC_ALLOW_LARGE_OS_PAGES=0\" \"MIMALLOC_ARENA_EAGER_COMMIT=0\"\n\
ExecStart=\"{exe_escaped}\" --config \"{cfg_escaped}\" web\n\
Restart=on-failure\n\
RestartSec=3\n\
TimeoutStopSec=10\n\
MemoryMax=512M\n\
StandardOutput=journal\n\
StandardError=journal\n\
\n\
[Install]\n\
WantedBy=default.target\n"
    ))
}

fn user_unit_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT_NAME))
}

pub fn run_service_unit(config_path: &Path, json_out: bool) -> Result<()> {
    let unit = render_systemd_unit(config_path)?;
    let path = user_unit_path().ok();
    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "unit_name": SYSTEMD_UNIT_NAME,
                "target_path": path.as_ref().map(|p| p.display().to_string()),
                "content": unit,
            }))?
        ),
        false => print!("{unit}"),
    }
    Ok(())
}

pub fn run_service_install(config_path: &Path, enable_now: bool, json_out: bool) -> Result<()> {
    let unit = render_systemd_unit(config_path)?;
    let path = user_unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &unit)
        .with_context(|| format!("failed to write {}", path.display()))?;

    let reload_ok = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let enable_ok = match enable_now {
        true => Command::new("systemctl")
            .args(["--user", "enable", "--now", SYSTEMD_UNIT_NAME])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        false => false,
    };

    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "installed": true,
                "path": path.display().to_string(),
                "daemon_reload_ok": reload_ok,
                "enabled_and_started": enable_ok,
            }))?
        ),
        false => {
            println!("Installed user unit: {}", path.display());
            println!("  systemctl --user daemon-reload: {}", ok_or_skip(reload_ok));
            match enable_now {
                true => println!(
                    "  systemctl --user enable --now {SYSTEMD_UNIT_NAME}: {}",
                    ok_or_skip(enable_ok)
                ),
                false => println!(
                    "  Enable & start with: systemctl --user enable --now {SYSTEMD_UNIT_NAME}"
                ),
            }
        }
    }
    Ok(())
}

pub fn run_service_status(json_out: bool) -> Result<()> {
    let out = Command::new("systemctl")
        .args(["--user", "status", SYSTEMD_UNIT_NAME, "--no-pager"])
        .output();
    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            match json_out {
                true => println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "unit": SYSTEMD_UNIT_NAME,
                        "active": o.status.success(),
                        "stdout": stdout,
                        "stderr": stderr,
                    }))?
                ),
                false => {
                    if !stdout.is_empty() {
                        print!("{stdout}");
                    }
                    if !stderr.is_empty() {
                        eprint!("{stderr}");
                    }
                }
            }
        }
        Err(e) => match json_out {
            true => println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "unit": SYSTEMD_UNIT_NAME,
                    "active": false,
                    "error": e.to_string(),
                }))?
            ),
            false => println!("systemctl unavailable: {e}"),
        },
    }
    Ok(())
}

fn ok_or_skip(ok: bool) -> &'static str {
    match ok {
        true => "OK",
        false => "skipped (no active user systemd bus)",
    }
}

#[cfg(test)]
mod tests {
    use super::escape_systemd_quoted_arg;

    #[test]
    fn systemd_quoted_arg_escapes_specifiers_quotes_and_newlines() {
        let escaped = escape_systemd_quoted_arg("/opt/zenko %i/\"cfg\"\\a\nb\rc");
        assert_eq!(escaped, "/opt/zenko %%i/\\\"cfg\\\"\\\\a b c");
    }
}
