//! Pure-Rust, Zero-Regex Tokei-Inspired Deterministic FSM Code / Comment / Blank Counter.
//!
//! Driven strictly by `zenko_core::LANGUAGE_SYNTAX_TABLE`: tracks nested block comments
//! (Rust, Kotlin, Swift), string literals (ignoring comment markers inside strings),
//! and escaped quotes in a single linear byte scan.

use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use zenko_core::{
    CODE_STATS_MAX_FILES, DEFAULT_SEARCH_MAX_FILE_BYTES, DEFAULT_SEARCH_SKIP_DIRS,
    LanguageSyntaxSpec, find_language_syntax_by_ext,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FileLineStats {
    pub files: usize,
    pub code: usize,
    pub comments: usize,
    pub blanks: usize,
    pub total: usize,
}

impl FileLineStats {
    pub fn add_assign(&mut self, other: Self) {
        self.files += other.files;
        self.code += other.code;
        self.comments += other.comments;
        self.blanks += other.blanks;
        self.total += other.total;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageStatsEntry {
    pub language: String,
    pub files: usize,
    pub code: usize,
    pub comments: usize,
    pub blanks: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkspaceCodeStats {
    pub languages: Vec<LanguageStatsEntry>,
    pub totals: FileLineStats,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    BlockComment(usize),
    InString { quote: u8, escaped: bool },
}

fn quote_enabled(spec: &LanguageSyntaxSpec, byte: u8) -> bool {
    match byte {
        b'"' => spec.double_quote,
        b'\'' => spec.single_quote,
        b'`' => spec.backtick_quote,
        _ => false,
    }
}

fn starts_with_bytes(haystack: &[u8], pos: usize, needle: Option<&str>) -> Option<usize> {
    let n = needle?.as_bytes();
    match !n.is_empty() && haystack[pos..].starts_with(n) {
        true => Some(n.len()),
        false => None,
    }
}

/// Deterministic zero-regex byte FSM counting `code`, `comments`, and `blanks` for a single file.
pub fn count_file_stats(spec: &LanguageSyntaxSpec, content: &str) -> FileLineStats {
    let mut stats = FileLineStats {
        files: 1,
        ..FileLineStats::default()
    };
    let mut state = ScanState::Normal;

    for raw_line in content.lines() {
        stats.total += 1;
        let line = raw_line.trim_end_matches('\r');
        let bytes = line.as_bytes();
        let trimmed_empty = bytes.iter().all(|b| b.is_ascii_whitespace());

        if trimmed_empty {
            match state {
                ScanState::Normal => stats.blanks += 1,
                ScanState::BlockComment(_) => stats.comments += 1,
                ScanState::InString { .. } => stats.code += 1,
            }
            continue;
        }

        let mut i = 0usize;
        let mut saw_code = false;
        let mut saw_comment = false;

        while i < bytes.len() {
            let b = bytes[i];
            match state {
                ScanState::Normal => {
                    if b.is_ascii_whitespace() {
                        i += 1;
                        continue;
                    }
                    if starts_with_bytes(bytes, i, spec.line_comment).is_some() {
                        saw_comment = true;
                        break;
                    }
                    if let Some(len) = starts_with_bytes(bytes, i, spec.block_open) {
                        saw_comment = true;
                        state = ScanState::BlockComment(1);
                        i += len;
                        continue;
                    }
                    if quote_enabled(spec, b) {
                        saw_code = true;
                        state = ScanState::InString {
                            quote: b,
                            escaped: false,
                        };
                        i += 1;
                        continue;
                    }
                    saw_code = true;
                    i += 1;
                }
                ScanState::BlockComment(depth) => {
                    saw_comment = true;
                    if spec.nested_block_comments
                        && let Some(len) = starts_with_bytes(bytes, i, spec.block_open)
                    {
                        state = ScanState::BlockComment(depth.saturating_add(1));
                        i += len;
                        continue;
                    }
                    if let Some(len) = starts_with_bytes(bytes, i, spec.block_close) {
                        state = match depth.saturating_sub(1) {
                            0 => ScanState::Normal,
                            rem => ScanState::BlockComment(rem),
                        };
                        i += len;
                        continue;
                    }
                    i += 1;
                }
                ScanState::InString { quote, escaped } => {
                    saw_code = true;
                    state = match (escaped, b) {
                        (true, _) => ScanState::InString {
                            quote,
                            escaped: false,
                        },
                        (false, b'\\') => ScanState::InString {
                            quote,
                            escaped: true,
                        },
                        (false, ch) if ch == quote => ScanState::Normal,
                        (false, _) => ScanState::InString {
                            quote,
                            escaped: false,
                        },
                    };
                    i += 1;
                }
            }
        }

        // Single-line strings terminate cleanly on line end (backtick templates may span lines)
        if let ScanState::InString { quote, .. } = state
            && quote != b'`'
        {
            state = ScanState::Normal;
        }

        match (saw_code, saw_comment) {
            (true, _) => stats.code += 1,
            (false, true) => stats.comments += 1,
            (false, false) => stats.blanks += 1,
        }
    }

    stats
}

/// Scans a workspace root respecting `.gitignore` and `DEFAULT_SEARCH_SKIP_DIRS`,
/// returning exact per-language code/comment/blank totals sorted by code LOC descending.
pub fn scan_workspace_code_stats(root: &Path, max_files: usize) -> WorkspaceCodeStats {
    let cap = max_files.clamp(1, CODE_STATS_MAX_FILES);
    let mut by_lang: BTreeMap<&'static str, FileLineStats> = BTreeMap::new();
    let mut totals = FileLineStats::default();
    let mut seen_files = 0usize;
    let mut truncated = false;

    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(true)
        .require_git(false)
        .max_filesize(Some(DEFAULT_SEARCH_MAX_FILE_BYTES))
        .filter_entry(|dent| {
            if dent.depth() == 0 {
                return true;
            }
            let is_dir = dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            match is_dir {
                false => true,
                true => {
                    let name = dent.file_name().to_str().unwrap_or("");
                    !DEFAULT_SEARCH_SKIP_DIRS.contains(&name)
                }
            }
        });

    for dent in walker.build().flatten() {
        let is_file = dent.file_type().map(|ft| ft.is_file()).unwrap_or(false);
        if !is_file {
            continue;
        }
        let Some(ext) = dent.path().extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let Some(spec) = find_language_syntax_by_ext(ext) else {
            continue;
        };
        if seen_files >= cap {
            truncated = true;
            break;
        }
        let Ok(content) = std::fs::read_to_string(dent.path()) else {
            continue;
        };
        seen_files += 1;
        let file_stats = count_file_stats(spec, &content);
        by_lang.entry(spec.name).or_default().add_assign(file_stats);
        totals.add_assign(file_stats);
    }

    let mut languages: Vec<LanguageStatsEntry> = by_lang
        .into_iter()
        .map(|(lang, st)| LanguageStatsEntry {
            language: lang.to_string(),
            files: st.files,
            code: st.code,
            comments: st.comments,
            blanks: st.blanks,
            total: st.total,
        })
        .collect();
    languages.sort_by(|a, b| b.code.cmp(&a.code).then_with(|| a.language.cmp(&b.language)));

    WorkspaceCodeStats {
        languages,
        totals,
        truncated,
    }
}

fn format_compact_loc(n: usize) -> String {
    match n >= 1_000 {
        true => format!("{:.1}k", n as f64 / 1_000.0),
        false => n.to_string(),
    }
}

/// Renders a concise 1-line language/LOC summary for Sovereign Orchestrator Turn-1 recon.
pub fn format_recon_language_summary(stats: &WorkspaceCodeStats, top_n: usize) -> Option<String> {
    if stats.languages.is_empty() {
        return None;
    }
    let parts: Vec<String> = stats
        .languages
        .iter()
        .take(top_n.max(1))
        .map(|l| {
            format!(
                "{} {} code/{} cmt ({}f)",
                l.language,
                format_compact_loc(l.code),
                format_compact_loc(l.comments),
                l.files
            )
        })
        .collect();
    let more = match stats.languages.len() > top_n {
        true => format!(" +{} lang", stats.languages.len() - top_n),
        false => String::new(),
    };
    Some(format!("{}{more}", parts.join(", ")))
}

/// Formats a compact aligned per-language report for `code_refactor(action="stats")`.
pub fn format_code_stats_table(stats: &WorkspaceCodeStats) -> String {
    if stats.languages.is_empty() {
        return "No recognized source files found.".to_string();
    }
    let mut lines = vec![format!(
        "Language Stats ({} files · {} code / {} comments / {} blanks / {} total{}):",
        stats.totals.files,
        stats.totals.code,
        stats.totals.comments,
        stats.totals.blanks,
        stats.totals.total,
        match stats.truncated {
            true => ", truncated at file cap",
            false => "",
        }
    )];
    for entry in &stats.languages {
        let cmt_pct = match entry.code + entry.comments {
            0 => 0.0,
            denom => (entry.comments as f64 * 100.0) / denom as f64,
        };
        lines.push(format!(
            "- {:<12} {:>4} file(s) | {:>7} code | {:>6} cmt ({:>4.1}%) | {:>5} blank",
            entry.language, entry.files, entry.code, entry.comments, cmt_pct, entry.blanks
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokei_fsm_handles_nested_comments_and_comment_markers_in_strings() {
        let rust = find_language_syntax_by_ext("rs").expect("rust spec");
        let sample = r#"
// top line comment
/* outer comment
   /* nested inner comment */
   still outer comment
*/
pub fn demo() -> &'static str {
    let s = "/* not a comment */ // also not a comment";
    s // inline trailing comment still counts as code
}
"#;
        let st = count_file_stats(rust, sample);
        assert_eq!(st.code, 4, "expected 4 code lines, got {st:?}");
        assert_eq!(st.comments, 5, "expected 5 comment lines, got {st:?}");
        assert_eq!(st.blanks, 1, "expected 1 blank line, got {st:?}");
        assert_eq!(st.total, 10);
    }

    #[test]
    fn workspace_stats_aggregates_languages_and_formats_recon_line() {
        let dir = std::env::temp_dir().join(format!("zenko_tokei_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");
        std::fs::write(dir.join("src/lib.rs"), "// cmt\npub fn a() {}\n").expect("rs");
        std::fs::write(dir.join("src/app.ts"), "/* cmt */\nexport const x = 1;\n\n").expect("ts");

        let stats = scan_workspace_code_stats(&dir, 50);
        assert_eq!(stats.totals.files, 2);
        assert_eq!(stats.totals.code, 2);
        assert_eq!(stats.totals.comments, 2);
        let summary = format_recon_language_summary(&stats, 4).expect("summary");
        assert!(summary.contains("Rust"));
        assert!(summary.contains("TypeScript"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
