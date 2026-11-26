//! Zero-regex, pure-Rust state-machine parsers for both OpenAI Codex's structured multi-file
//! `*** Begin Patch` grammar AND standard POSIX/Git unified diffs (`--- a/` / `+++ b/` / `@@`).

use zenko_core::{
    APPLY_PATCH_MAX_FILES, CODEX_PATCH_ADD_FILE, CODEX_PATCH_BEGIN, CODEX_PATCH_DELETE_FILE,
    CODEX_PATCH_END, CODEX_PATCH_END_OF_FILE, CODEX_PATCH_MOVE_TO, CODEX_PATCH_UPDATE_FILE, Result,
    UNIFIED_DIFF_DEV_NULL, UNIFIED_DIFF_GIT_META_PREFIXES, UNIFIED_DIFF_GIT_PREFIX,
    UNIFIED_DIFF_HUNK_MARKER, UNIFIED_DIFF_NEW_PREFIX, UNIFIED_DIFF_NO_NEWLINE,
    UNIFIED_DIFF_OLD_PREFIX, UNIFIED_DIFF_RENAME_FROM, UNIFIED_DIFF_RENAME_TO, ZenkoError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFormat {
    Codex,
    UnifiedDiff,
}

/// Deterministic zero-regex patch format detector (handles optional Markdown code fences).
pub fn detect_patch_format(raw: &str) -> Option<PatchFormat> {
    let lines = strip_optional_code_fences(raw);
    if lines.iter().any(|l| l.trim() == CODEX_PATCH_BEGIN) {
        return Some(PatchFormat::Codex);
    }
    let has_git_diff = lines.iter().any(|l| l.starts_with(UNIFIED_DIFF_GIT_PREFIX));
    let has_minus = lines.iter().any(|l| l.starts_with(UNIFIED_DIFF_OLD_PREFIX));
    let has_plus = lines.iter().any(|l| l.starts_with(UNIFIED_DIFF_NEW_PREFIX));
    match has_git_diff || (has_minus && has_plus) {
        true => Some(PatchFormat::UnifiedDiff),
        false => None,
    }
}

/// Auto-detects Codex vs standard POSIX/Git unified diff and parses into validated per-file ops.
pub fn parse_auto_patch(raw: &str) -> Result<(PatchFormat, Vec<CodexPatchFileOp>)> {
    match detect_patch_format(raw) {
        Some(PatchFormat::Codex) => Ok((PatchFormat::Codex, parse_codex_patch(raw)?)),
        Some(PatchFormat::UnifiedDiff) => {
            Ok((PatchFormat::UnifiedDiff, parse_unified_diff_patch(raw)?))
        }
        None => Err(ZenkoError::Tool(
            "Unrecognized patch format: supply either Codex (`*** Begin Patch ... *** End Patch`) or standard unified diff (`--- a/file` / `+++ b/file` / `@@ ... @@`).".into(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexUpdateHunk {
    pub context_header: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub is_eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexPatchFileOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<CodexUpdateHunk>,
    },
}

enum ParserSection {
    Idle,
    Add {
        path: String,
        lines: Vec<String>,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<CodexUpdateHunk>,
        current_hunk: Option<CodexUpdateHunk>,
    },
}

fn flush_section(section: ParserSection, out: &mut Vec<CodexPatchFileOp>) -> Result<()> {
    match section {
        ParserSection::Idle => Ok(()),
        ParserSection::Add { path, lines } => {
            let mut content = lines.join("\n");
            if !lines.is_empty() {
                content.push('\n');
            }
            out.push(CodexPatchFileOp::Add { path, content });
            Ok(())
        }
        ParserSection::Update {
            path,
            move_to,
            mut hunks,
            current_hunk,
        } => {
            if let Some(h) = current_hunk {
                hunks.push(h);
            }
            if hunks.is_empty() && move_to.is_none() {
                return Err(ZenkoError::Tool(format!(
                    "Codex patch `*** Update File: {path}` has neither `@@` hunks nor `*** Move to:`"
                )));
            }
            out.push(CodexPatchFileOp::Update {
                path,
                move_to,
                hunks,
            });
            Ok(())
        }
    }
}

fn strip_optional_code_fences(raw: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = raw.lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.first().is_some_and(|l| l.trim_start().starts_with("```")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.pop();
    }
    lines
}

fn parse_header_path(raw: &str, marker: &str, line_no: usize) -> Result<String> {
    let p = raw.trim();
    match p.is_empty() {
        true => Err(ZenkoError::Tool(format!(
            "Codex patch line {line_no}: `{marker}` missing file path"
        ))),
        false => Ok(p.to_string()),
    }
}

/// Parses a Codex multi-file patch payload into validated per-file operations.
pub fn parse_codex_patch(raw: &str) -> Result<Vec<CodexPatchFileOp>> {
    let lines = strip_optional_code_fences(raw);
    let Some(begin_idx) = lines.iter().position(|l| l.trim() == CODEX_PATCH_BEGIN) else {
        return Err(ZenkoError::Tool(format!(
            "Codex patch must start with `{CODEX_PATCH_BEGIN}`"
        )));
    };

    let mut ops: Vec<CodexPatchFileOp> = Vec::new();
    let mut section = ParserSection::Idle;

    for (offset, &raw_line) in lines[begin_idx + 1..].iter().enumerate() {
        let line_no = begin_idx + offset + 2;
        let trimmed = raw_line.trim_end_matches('\r');

        if trimmed.trim() == CODEX_PATCH_END {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            break;
        }

        if let Some(rest) = trimmed.strip_prefix(CODEX_PATCH_ADD_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, CODEX_PATCH_ADD_FILE, line_no)?;
            section = ParserSection::Add {
                path,
                lines: Vec::new(),
            };
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(CODEX_PATCH_DELETE_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, CODEX_PATCH_DELETE_FILE, line_no)?;
            ops.push(CodexPatchFileOp::Delete { path });
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(CODEX_PATCH_UPDATE_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, CODEX_PATCH_UPDATE_FILE, line_no)?;
            section = ParserSection::Update {
                path,
                move_to: None,
                hunks: Vec::new(),
                current_hunk: None,
            };
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(CODEX_PATCH_MOVE_TO) {
            let dest = parse_header_path(rest, CODEX_PATCH_MOVE_TO, line_no)?;
            match &mut section {
                ParserSection::Update {
                    move_to,
                    hunks,
                    current_hunk,
                    ..
                } if hunks.is_empty() && current_hunk.is_none() => {
                    *move_to = Some(dest);
                    continue;
                }
                _ => {
                    return Err(ZenkoError::Tool(format!(
                        "Codex patch line {line_no}: `{CODEX_PATCH_MOVE_TO}` must appear immediately after `{CODEX_PATCH_UPDATE_FILE}`"
                    )));
                }
            }
        }

        match &mut section {
            ParserSection::Idle => {
                if !trimmed.trim().is_empty() {
                    return Err(ZenkoError::Tool(format!(
                        "Codex patch line {line_no}: unexpected line outside file header: `{trimmed}`"
                    )));
                }
            }
            ParserSection::Add { lines, .. } => match trimmed.strip_prefix('+') {
                Some(content_line) => lines.push(content_line.to_string()),
                None if trimmed.is_empty() => {}
                None => {
                    return Err(ZenkoError::Tool(format!(
                        "Codex patch line {line_no}: lines inside `*** Add File` must start with `+`"
                    )));
                }
            },
            ParserSection::Update {
                hunks,
                current_hunk,
                ..
            } => {
                if trimmed.trim() == CODEX_PATCH_END_OF_FILE {
                    if let Some(h) = current_hunk.as_mut() {
                        h.is_eof = true;
                    }
                    continue;
                }
                if let Some(header_rest) = trimmed.strip_prefix(UNIFIED_DIFF_HUNK_MARKER) {
                    if let Some(prev) = current_hunk.take() {
                        hunks.push(prev);
                    }
                    let ctx = header_rest.trim();
                    let context_header = match ctx.is_empty() {
                        true => None,
                        false => Some(ctx.to_string()),
                    };
                    *current_hunk = Some(CodexUpdateHunk {
                        context_header,
                        old_lines: Vec::new(),
                        new_lines: Vec::new(),
                        is_eof: false,
                    });
                    continue;
                }
                let hunk = current_hunk.get_or_insert_with(|| CodexUpdateHunk {
                    context_header: None,
                    old_lines: Vec::new(),
                    new_lines: Vec::new(),
                    is_eof: false,
                });
                match trimmed.as_bytes().first().copied() {
                    Some(b' ') => {
                        let body = &trimmed[1..];
                        hunk.old_lines.push(body.to_string());
                        hunk.new_lines.push(body.to_string());
                    }
                    Some(b'-') => {
                        hunk.old_lines.push(trimmed[1..].to_string());
                    }
                    Some(b'+') => {
                        hunk.new_lines.push(trimmed[1..].to_string());
                    }
                    None => {
                        hunk.old_lines.push(String::new());
                        hunk.new_lines.push(String::new());
                    }
                    Some(other) => {
                        return Err(ZenkoError::Tool(format!(
                            "Codex patch line {line_no}: unexpected diff prefix `{}` (expected ` `, `-`, `+`, or `@@`)",
                            other as char
                        )));
                    }
                }
            }
        }
    }

    flush_section(section, &mut ops)?;

    if ops.is_empty() {
        return Err(ZenkoError::Tool(
            "Codex patch contained zero file operations".into(),
        ));
    }
    if ops.len() > APPLY_PATCH_MAX_FILES {
        return Err(ZenkoError::Tool(format!(
            "Codex patch touches {} files (exceeds max {APPLY_PATCH_MAX_FILES} per call)",
            ops.len()
        )));
    }
    Ok(ops)
}

fn clean_unified_diff_path(raw: &str, git_prefix: &str) -> String {
    let without_ts = raw.split('\t').next().unwrap_or(raw).trim();
    let unquoted = without_ts
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(without_ts);
    match unquoted == UNIFIED_DIFF_DEV_NULL {
        true => UNIFIED_DIFF_DEV_NULL.to_string(),
        false => unquoted
            .strip_prefix(git_prefix)
            .unwrap_or(unquoted)
            .to_string(),
    }
}

fn parse_unified_hunk_context(line: &str) -> Result<Option<String>> {
    let rest = line.strip_prefix(UNIFIED_DIFF_HUNK_MARKER).ok_or_else(|| {
        ZenkoError::Tool(format!("Unified diff invalid hunk header: `{line}`"))
    })?;
    let Some(close_idx) = rest.find(UNIFIED_DIFF_HUNK_MARKER) else {
        return Err(ZenkoError::Tool(format!(
            "Unified diff hunk header missing closing `{UNIFIED_DIFF_HUNK_MARKER}`: `{line}`"
        )));
    };
    let coords = rest[..close_idx].trim();
    if !coords.starts_with('-') || !coords.contains('+') {
        return Err(ZenkoError::Tool(format!(
            "Unified diff hunk header missing `-old +new` coordinates: `{line}`"
        )));
    }
    let ctx = rest[close_idx + UNIFIED_DIFF_HUNK_MARKER.len()..].trim();
    Ok(match ctx.is_empty() {
        true => None,
        false => Some(ctx.to_string()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnifiedDiffState {
    Header,
    Hunk,
}

enum UnifiedLineToken<'a> {
    GitHeader,
    OldFile(&'a str),
    NewFile(&'a str),
    RenameFrom(&'a str),
    RenameTo(&'a str),
    HunkHeader(&'a str),
    GitMeta,
    NoNewline,
    Body(&'a str),
}

fn classify_unified_line<'a>(
    state: UnifiedDiffState,
    line: &'a str,
    next_line: Option<&'a str>,
) -> UnifiedLineToken<'a> {
    if line.starts_with(UNIFIED_DIFF_GIT_PREFIX) {
        return UnifiedLineToken::GitHeader;
    }
    if line.starts_with("@@ ") || line.starts_with("@@-") {
        return UnifiedLineToken::HunkHeader(line);
    }
    match state {
        UnifiedDiffState::Header => line
            .strip_prefix(UNIFIED_DIFF_OLD_PREFIX)
            .map(UnifiedLineToken::OldFile)
            .or_else(|| {
                line.strip_prefix(UNIFIED_DIFF_NEW_PREFIX)
                    .map(UnifiedLineToken::NewFile)
            })
            .or_else(|| {
                line.strip_prefix(UNIFIED_DIFF_RENAME_FROM)
                    .map(UnifiedLineToken::RenameFrom)
            })
            .or_else(|| {
                line.strip_prefix(UNIFIED_DIFF_RENAME_TO)
                    .map(UnifiedLineToken::RenameTo)
            })
            .unwrap_or_else(|| {
                let is_meta = line.trim().is_empty()
                    || UNIFIED_DIFF_GIT_META_PREFIXES
                        .iter()
                        .any(|p| line.starts_with(p));
                match is_meta {
                    true => UnifiedLineToken::GitMeta,
                    false => UnifiedLineToken::Body(line),
                }
            }),
        UnifiedDiffState::Hunk => match line.strip_prefix(UNIFIED_DIFF_OLD_PREFIX) {
            Some(old_rest)
                if next_line.is_some_and(|n| n.starts_with(UNIFIED_DIFF_NEW_PREFIX)) =>
            {
                UnifiedLineToken::OldFile(old_rest)
            }
            _ => match line.starts_with(UNIFIED_DIFF_NO_NEWLINE) {
                true => UnifiedLineToken::NoNewline,
                false => UnifiedLineToken::Body(line),
            },
        },
    }
}

struct UnifiedFileBuilder {
    old_path: Option<String>,
    new_path: Option<String>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    hunks: Vec<CodexUpdateHunk>,
    current_hunk: Option<CodexUpdateHunk>,
    no_trailing_newline: bool,
}

impl UnifiedFileBuilder {
    fn new() -> Self {
        Self {
            old_path: None,
            new_path: None,
            rename_from: None,
            rename_to: None,
            hunks: Vec::new(),
            current_hunk: None,
            no_trailing_newline: false,
        }
    }

    fn is_empty(&self) -> bool {
        self.old_path.is_none()
            && self.new_path.is_none()
            && self.rename_from.is_none()
            && self.rename_to.is_none()
            && self.hunks.is_empty()
            && self.current_hunk.is_none()
    }

    fn flush(mut self, out: &mut Vec<CodexPatchFileOp>) -> Result<()> {
        if let Some(h) = self.current_hunk.take() {
            self.hunks.push(h);
        }
        if self.is_empty() {
            return Ok(());
        }
        let old_p = self.old_path.or(self.rename_from);
        let new_p = self.new_path.or(self.rename_to);
        match (old_p.as_deref(), new_p.as_deref()) {
            (Some(UNIFIED_DIFF_DEV_NULL) | None, Some(dest))
                if dest != UNIFIED_DIFF_DEV_NULL =>
            {
                let mut added: Vec<String> = Vec::new();
                for h in self.hunks {
                    added.extend(h.new_lines);
                }
                let mut content = added.join("\n");
                if !added.is_empty() && !self.no_trailing_newline {
                    content.push('\n');
                }
                out.push(CodexPatchFileOp::Add {
                    path: dest.to_string(),
                    content,
                });
                Ok(())
            }
            (Some(src), Some(UNIFIED_DIFF_DEV_NULL)) if src != UNIFIED_DIFF_DEV_NULL => {
                out.push(CodexPatchFileOp::Delete {
                    path: src.to_string(),
                });
                Ok(())
            }
            (Some(src), Some(dest))
                if src != UNIFIED_DIFF_DEV_NULL && dest != UNIFIED_DIFF_DEV_NULL =>
            {
                let move_to = match src != dest {
                    true => Some(dest.to_string()),
                    false => None,
                };
                if self.hunks.is_empty() && move_to.is_none() {
                    return Err(ZenkoError::Tool(format!(
                        "Unified diff for `{src}` contains no `{UNIFIED_DIFF_HUNK_MARKER}` hunks and no rename."
                    )));
                }
                out.push(CodexPatchFileOp::Update {
                    path: src.to_string(),
                    move_to,
                    hunks: self.hunks,
                });
                Ok(())
            }
            _ => Err(ZenkoError::Tool(
                "Unified diff file section missing valid `---` / `+++` paths.".into(),
            )),
        }
    }
}

/// Deterministic zero-regex finite state machine for standard POSIX / Git unified diffs.
pub fn parse_unified_diff_patch(raw: &str) -> Result<Vec<CodexPatchFileOp>> {
    let lines = strip_optional_code_fences(raw);
    let mut ops: Vec<CodexPatchFileOp> = Vec::new();
    let mut cur = UnifiedFileBuilder::new();
    let mut state = UnifiedDiffState::Header;

    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim_end_matches('\r');
        let next_line = lines.get(idx + 1).map(|l| l.trim_end_matches('\r'));
        let token = classify_unified_line(state, line, next_line);

        match (state, token) {
            (_, UnifiedLineToken::GitHeader) => {
                if !cur.is_empty() {
                    std::mem::replace(&mut cur, UnifiedFileBuilder::new()).flush(&mut ops)?;
                }
                state = UnifiedDiffState::Header;
            }
            (UnifiedDiffState::Header, UnifiedLineToken::OldFile(rest)) => {
                if cur.old_path.is_some() || cur.new_path.is_some() {
                    std::mem::replace(&mut cur, UnifiedFileBuilder::new()).flush(&mut ops)?;
                }
                cur.old_path = Some(clean_unified_diff_path(rest, "a/"));
            }
            (UnifiedDiffState::Hunk, UnifiedLineToken::OldFile(rest)) => {
                std::mem::replace(&mut cur, UnifiedFileBuilder::new()).flush(&mut ops)?;
                cur.old_path = Some(clean_unified_diff_path(rest, "a/"));
                state = UnifiedDiffState::Header;
            }
            (UnifiedDiffState::Header, UnifiedLineToken::NewFile(rest)) => {
                cur.new_path = Some(clean_unified_diff_path(rest, "b/"));
            }
            (UnifiedDiffState::Header, UnifiedLineToken::RenameFrom(rest)) => {
                cur.rename_from = Some(rest.trim().to_string());
            }
            (UnifiedDiffState::Header, UnifiedLineToken::RenameTo(rest)) => {
                cur.rename_to = Some(rest.trim().to_string());
            }
            (UnifiedDiffState::Header, UnifiedLineToken::GitMeta) => {}
            (_, UnifiedLineToken::HunkHeader(hdr)) => {
                if let Some(prev) = cur.current_hunk.take() {
                    cur.hunks.push(prev);
                }
                let context_header = parse_unified_hunk_context(hdr)?;
                cur.current_hunk = Some(CodexUpdateHunk {
                    context_header,
                    old_lines: Vec::new(),
                    new_lines: Vec::new(),
                    is_eof: false,
                });
                state = UnifiedDiffState::Hunk;
            }
            (UnifiedDiffState::Hunk, UnifiedLineToken::NoNewline) => {
                cur.no_trailing_newline = true;
            }
            (UnifiedDiffState::Hunk, UnifiedLineToken::Body(body_line)) => {
                let Some(hunk) = cur.current_hunk.as_mut() else {
                    return Err(ZenkoError::Tool(format!(
                        "Unified diff line {line_no}: diff body outside `{UNIFIED_DIFF_HUNK_MARKER}` hunk"
                    )));
                };
                match body_line.as_bytes().first().copied() {
                    Some(b' ') => {
                        let body = &body_line[1..];
                        hunk.old_lines.push(body.to_string());
                        hunk.new_lines.push(body.to_string());
                    }
                    Some(b'-') => hunk.old_lines.push(body_line[1..].to_string()),
                    Some(b'+') => hunk.new_lines.push(body_line[1..].to_string()),
                    None => {
                        hunk.old_lines.push(String::new());
                        hunk.new_lines.push(String::new());
                    }
                    Some(other) => {
                        return Err(ZenkoError::Tool(format!(
                            "Unified diff line {line_no}: unexpected line prefix `{}` inside hunk",
                            other as char
                        )));
                    }
                }
            }
            (UnifiedDiffState::Header, UnifiedLineToken::Body(bad)) => {
                return Err(ZenkoError::Tool(format!(
                    "Unified diff line {line_no}: unexpected header line `{bad}`"
                )));
            }
            _ => {
                return Err(ZenkoError::Tool(format!(
                    "Unified diff line {line_no}: unexpected token `{line}`"
                )));
            }
        }
    }

    cur.flush(&mut ops)?;

    if ops.is_empty() {
        return Err(ZenkoError::Tool(
            "Unified diff contained zero file operations".into(),
        ));
    }
    if ops.len() > APPLY_PATCH_MAX_FILES {
        return Err(ZenkoError::Tool(format!(
            "Unified diff touches {} files (exceeds max {APPLY_PATCH_MAX_FILES} per call)",
            ops.len()
        )));
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_file_codex_patch_add_update_move_delete() {
        let patch = r#"
```patch
*** Begin Patch
*** Add File: src/new_mod.rs
+pub fn hello() -> &'static str {
+    "world"
+}
*** Update File: src/old.rs
*** Move to: src/renamed.rs
@@ fn greet()
 fn greet() {
-    println!("old");
+    println!("new");
 }
*** End of File
*** Delete File: src/obsolete.rs
*** End Patch
```
"#;
        let ops = parse_codex_patch(patch).expect("valid patch");
        assert_eq!(ops.len(), 3);
        assert_eq!(
            ops[0],
            CodexPatchFileOp::Add {
                path: "src/new_mod.rs".into(),
                content: "pub fn hello() -> &'static str {\n    \"world\"\n}\n".into(),
            }
        );
        match &ops[1] {
            CodexPatchFileOp::Update {
                path,
                move_to,
                hunks,
            } => {
                assert_eq!(path, "src/old.rs");
                assert_eq!(move_to.as_deref(), Some("src/renamed.rs"));
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].context_header.as_deref(), Some("fn greet()"));
                assert!(hunks[0].is_eof);
                assert_eq!(
                    hunks[0].old_lines,
                    vec!["fn greet() {", "    println!(\"old\");", "}"]
                );
                assert_eq!(
                    hunks[0].new_lines,
                    vec!["fn greet() {", "    println!(\"new\");", "}"]
                );
            }
            other => panic!("expected update op, got {other:?}"),
        }
        assert_eq!(
            ops[2],
            CodexPatchFileOp::Delete {
                path: "src/obsolete.rs".into(),
            }
        );
    }

    #[test]
    fn rejects_malformed_codex_patch_cleanly() {
        assert!(parse_codex_patch("no header here").is_err());
        assert!(parse_codex_patch("*** Begin Patch\n*** End Patch").is_err());
        assert!(
            parse_codex_patch("*** Begin Patch\n*** Add File: a.txt\nmissing plus\n*** End Patch")
                .is_err()
        );
    }

    #[test]
    fn parses_standard_unified_diff_add_update_rename_and_delete() {
        let diff = r#"
```diff
diff --git a/src/new_file.rs b/src/new_file.rs
new file mode 100644
--- /dev/null
+++ b/src/new_file.rs
@@ -0,0 +1,2 @@
+pub fn added() -> u8 {
+    7
+}
diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 80%
rename from src/old_name.rs
rename to src/new_name.rs
--- a/src/old_name.rs
+++ b/src/new_name.rs
@@ -1,3 +1,3 @@ fn run()
 fn run() {
-    let x = 1;
+    let x = 2;
 }
diff --git a/src/dead.rs b/src/dead.rs
deleted file mode 100644
--- a/src/dead.rs
+++ /dev/null
@@ -1,1 +0,0 @@
-pub fn gone() {}
```
"#;
        let (fmt, ops) = parse_auto_patch(diff).expect("valid unified diff");
        assert_eq!(fmt, PatchFormat::UnifiedDiff);
        assert_eq!(ops.len(), 3);
        assert_eq!(
            ops[0],
            CodexPatchFileOp::Add {
                path: "src/new_file.rs".into(),
                content: "pub fn added() -> u8 {\n    7\n}\n".into(),
            }
        );
        match &ops[1] {
            CodexPatchFileOp::Update {
                path,
                move_to,
                hunks,
            } => {
                assert_eq!(path, "src/old_name.rs");
                assert_eq!(move_to.as_deref(), Some("src/new_name.rs"));
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].context_header.as_deref(), Some("fn run()"));
                assert_eq!(hunks[0].old_lines, vec!["fn run() {", "    let x = 1;", "}"]);
                assert_eq!(hunks[0].new_lines, vec!["fn run() {", "    let x = 2;", "}"]);
            }
            other => panic!("expected update op, got {other:?}"),
        }
        assert_eq!(
            ops[2],
            CodexPatchFileOp::Delete {
                path: "src/dead.rs".into(),
            }
        );
    }
}
