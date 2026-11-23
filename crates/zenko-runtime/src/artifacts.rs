//! Async Prime-RLM / Codex Head+Tail Artifact Spilling Engine.
//! Oversized tool outputs are streamed atomically (`0600`) to `~/.zenko/artifacts/<session_id>/<call_id>.txt`
//! and replaced in active context with a UTF-8-safe, line-aligned 35% Head + 65% Tail preview plus an
//! addressable `artifact:<session_id>/<call_id>.txt` handle readable via `read_file` / `code_search`.
//! Zero regex, zero `if/else` ladders, 100% async `tokio::fs` I/O.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use zenko_core::{
    ARTIFACT_HEAD_RATIO_DEN, ARTIFACT_HEAD_RATIO_NUM, ARTIFACT_MAX_AGE_SECS,
    ARTIFACT_MAX_PER_SESSION, ARTIFACT_PATH_PREFIX, ARTIFACTS_DIR_NAME, ArtifactSpillRecord,
    FILE_MODE_OWNER_ONLY, SHELL_MIN_OUTPUT_BYTES,
};

fn sanitize_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    match cleaned.is_empty() {
        true => zenko_core::new_id(),
        false => cleaned,
    }
}

/// Computes a UTF-8-safe, line-boundary-aligned Head (35%) + Tail (65%) preview so both the
/// command/file header AND bottom compiler errors / stack traces survive truncation cleanly.
pub fn format_head_tail_preview(content: &str, max_bytes: usize) -> (String, usize, usize) {
    let total_bytes = content.len();
    let total_lines = content.lines().count();
    let budget = max_bytes.max(SHELL_MIN_OUTPUT_BYTES);
    if total_bytes <= budget {
        return (content.to_string(), total_lines, 0);
    }

    // Reserve room for the middle omission banner
    let usable = budget.saturating_sub(180).max(256);
    let head_target = (usable * ARTIFACT_HEAD_RATIO_NUM) / ARTIFACT_HEAD_RATIO_DEN;
    let tail_target = usable.saturating_sub(head_target);

    let raw_head_end = content.floor_char_boundary(head_target.min(total_bytes));
    let head_end = content[..raw_head_end]
        .rfind('\n')
        .filter(|&nl| nl > raw_head_end / 2)
        .unwrap_or(raw_head_end);

    let raw_tail_start = content.ceil_char_boundary(total_bytes.saturating_sub(tail_target));
    let tail_start = match raw_tail_start < total_bytes {
        true => content[raw_tail_start..]
            .find('\n')
            .map(|off| raw_tail_start + off + 1)
            .filter(|&pos| pos < total_bytes && total_bytes - pos > tail_target / 2)
            .unwrap_or(raw_tail_start),
        false => total_bytes,
    };

    if head_end >= tail_start {
        return (content.to_string(), total_lines, 0);
    }

    let omitted_slice = &content[head_end..tail_start];
    let omitted_bytes = omitted_slice.len();
    let omitted_lines = omitted_slice.lines().count();
    let head = content[..head_end].trim_end_matches('\n');
    let tail = content[tail_start..].trim_start_matches('\n');

    let preview = format!(
        "{head}\n\n... [{omitted_lines} lines / {omitted_bytes} bytes omitted from middle] ...\n\n{tail}"
    );
    (preview, total_lines, omitted_bytes)
}

/// Spills `content` asynchronously to `<data_dir>/artifacts/<session_id>/<call_id>.txt` when its
/// byte length exceeds `max_output_bytes`, returning a Head+Tail preview and inspectable handle.
pub async fn spill_tool_output_if_oversized(
    data_dir: &Path,
    session_id: &str,
    call_id: &str,
    tool_name: &str,
    content: String,
    max_output_bytes: usize,
) -> (String, Option<ArtifactSpillRecord>) {
    let limit = max_output_bytes.max(SHELL_MIN_OUTPUT_BYTES);
    if content.len() <= limit {
        return (content, None);
    }

    let safe_sid = sanitize_segment(session_id);
    let safe_cid = sanitize_segment(call_id);
    let filename = format!("{safe_cid}.txt");
    let rel_handle = format!("{ARTIFACT_PATH_PREFIX}{safe_sid}/{filename}");
    let session_dir = data_dir.join(ARTIFACTS_DIR_NAME).join(&safe_sid);
    let abs_path = session_dir.join(&filename);
    let total_bytes = content.len();
    let (preview, total_lines, _) = format_head_tail_preview(&content, limit);

    let write_ok = write_owner_only_async(&session_dir, &abs_path, content.as_bytes()).await;
    match write_ok {
        Ok(()) => {
            let summary = format!(
                "{preview}\n\n[spilled full `{tool_name}` output ({total_lines} lines, {total_bytes} bytes) -> `{rel_handle}`; inspect targeted slices with `read_file` or search with `code_search`]"
            );
            let record = ArtifactSpillRecord {
                handle: rel_handle,
                abs_path: abs_path.display().to_string(),
                total_bytes,
                total_lines,
                preview: summary.clone(),
            };
            (summary, Some(record))
        }
        Err(_) => {
            let fallback = format!(
                "{preview}\n\n[output condensed from {total_lines} lines / {total_bytes} bytes]"
            );
            (fallback, None)
        }
    }
}

async fn write_owner_only_async(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(
            &tmp,
            std::fs::Permissions::from_mode(FILE_MODE_OWNER_ONLY),
        )
        .await;
    }
    tokio::fs::rename(&tmp, path).await
}

/// Counts spilled artifact files currently stored for `session_id` (async, zero blocking).
pub async fn count_session_artifacts(data_dir: &Path, session_id: &str) -> usize {
    let dir = data_dir
        .join(ARTIFACTS_DIR_NAME)
        .join(sanitize_segment(session_id));
    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
        return 0;
    };
    let mut count = 0usize;
    while let Ok(Some(ent)) = rd.next_entry().await {
        if ent.path().extension().and_then(|e| e.to_str()) == Some("txt") {
            count += 1;
        }
    }
    count
}

/// Background async hygiene pruning expired artifacts (`> ARTIFACT_MAX_AGE_SECS`) and bounding
/// per-session artifact files to `ARTIFACT_MAX_PER_SESSION`.
pub async fn prune_old_artifacts(data_dir: &Path) {
    let root = data_dir.join(ARTIFACTS_DIR_NAME);
    let Ok(mut sessions) = tokio::fs::read_dir(&root).await else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(ARTIFACT_MAX_AGE_SECS))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    while let Ok(Some(sess_ent)) = sessions.next_entry().await {
        let sdir = sess_ent.path();
        if !sdir.is_dir() {
            continue;
        }
        let Ok(mut files) = tokio::fs::read_dir(&sdir).await else {
            continue;
        };
        let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
        while let Ok(Some(f_ent)) = files.next_entry().await {
            let p = f_ent.path();
            if !p.is_file() {
                continue;
            }
            let modified = f_ent
                .metadata()
                .await
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            match modified < cutoff {
                true => {
                    let _ = tokio::fs::remove_file(&p).await;
                }
                false => kept.push((modified, p)),
            }
        }
        if kept.len() > ARTIFACT_MAX_PER_SESSION {
            kept.sort_by_key(|(m, _)| *m);
            let drop_count = kept.len() - ARTIFACT_MAX_PER_SESSION;
            for (_, old_path) in kept.into_iter().take(drop_count) {
                let _ = tokio::fs::remove_file(&old_path).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spills_oversized_output_atomically_preserving_head_and_bottom_error_tail() {
        let base = std::env::temp_dir().join(format!("zenko_art_{}", zenko_core::new_id()));
        let mut raw = String::from("HEADER: cargo test --workspace\n");
        for i in 0..400 {
            raw.push_str(&format!("running verbose middle log line {i} — café UTF-8\n"));
        }
        raw.push_str("error[E0308]: mismatched types at src/lib.rs:42\nSUMMARY: 1 failed\n");

        let (out, rec) = spill_tool_output_if_oversized(
            &base,
            "sess_1",
            "call_99",
            "shell_exec",
            raw.clone(),
            2048,
        )
        .await;
        let spilled = rec.expect("must spill oversized output");
        assert_eq!(spilled.handle, "artifact:sess_1/call_99.txt");
        assert!(out.contains("HEADER: cargo test --workspace"), "head preserved");
        assert!(
            out.contains("error[E0308]: mismatched types at src/lib.rs:42"),
            "tail compiler error preserved"
        );
        assert!(out.contains("artifact:sess_1/call_99.txt"));
        assert_eq!(
            tokio::fs::read_to_string(&spilled.abs_path)
                .await
                .expect("read spilled"),
            raw
        );
        assert_eq!(count_session_artifacts(&base, "sess_1").await, 1);

        let _ = tokio::fs::remove_dir_all(&base).await;
    }
}
