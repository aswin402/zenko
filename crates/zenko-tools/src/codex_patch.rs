//! Zero-regex, pure-Rust state-machine parser for OpenAI Codex's structured multi-file
//! `*** Begin Patch` / `*** Update File:` / `*** Add File:` / `*** Delete File:` / `*** Move to:`
//! patch grammar.

use zenko_core::{APPLY_PATCH_MAX_FILES, Result, ZenkoError};

const MARKER_BEGIN_PATCH: &str = "*** Begin Patch";
const MARKER_END_PATCH: &str = "*** End Patch";
const MARKER_ADD_FILE: &str = "*** Add File:";
const MARKER_DELETE_FILE: &str = "*** Delete File:";
const MARKER_UPDATE_FILE: &str = "*** Update File:";
const MARKER_MOVE_TO: &str = "*** Move to:";
const MARKER_END_OF_FILE: &str = "*** End of File";

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
    let Some(begin_idx) = lines.iter().position(|l| l.trim() == MARKER_BEGIN_PATCH) else {
        return Err(ZenkoError::Tool(format!(
            "Codex patch must start with `{MARKER_BEGIN_PATCH}`"
        )));
    };

    let mut ops: Vec<CodexPatchFileOp> = Vec::new();
    let mut section = ParserSection::Idle;

    for (offset, &raw_line) in lines[begin_idx + 1..].iter().enumerate() {
        let line_no = begin_idx + offset + 2;
        let trimmed = raw_line.trim_end_matches('\r');

        if trimmed.trim() == MARKER_END_PATCH {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            break;
        }

        if let Some(rest) = trimmed.strip_prefix(MARKER_ADD_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, MARKER_ADD_FILE, line_no)?;
            section = ParserSection::Add {
                path,
                lines: Vec::new(),
            };
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(MARKER_DELETE_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, MARKER_DELETE_FILE, line_no)?;
            ops.push(CodexPatchFileOp::Delete { path });
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(MARKER_UPDATE_FILE) {
            flush_section(std::mem::replace(&mut section, ParserSection::Idle), &mut ops)?;
            let path = parse_header_path(rest, MARKER_UPDATE_FILE, line_no)?;
            section = ParserSection::Update {
                path,
                move_to: None,
                hunks: Vec::new(),
                current_hunk: None,
            };
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(MARKER_MOVE_TO) {
            let dest = parse_header_path(rest, MARKER_MOVE_TO, line_no)?;
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
                        "Codex patch line {line_no}: `{MARKER_MOVE_TO}` must appear immediately after `{MARKER_UPDATE_FILE}`"
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
                if trimmed.trim() == MARKER_END_OF_FILE {
                    if let Some(h) = current_hunk.as_mut() {
                        h.is_eof = true;
                    }
                    continue;
                }
                if let Some(header_rest) = trimmed.strip_prefix("@@") {
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
}
