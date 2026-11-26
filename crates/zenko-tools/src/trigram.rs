//! Lazy In-Process `tgrep`-Inspired Trigram Inverted Index Accelerator (pure Rust, zero regex).
//!
//! Eliminates O(entire repo) linear scans on repeated `code_search` / `code_refactor` lookups by
//! mapping packed 3-byte ASCII-lowercased trigrams (`u32`) to sorted file-ID posting lists,
//! validating per-file `(mtime_ns, len)` incrementally, and intersecting posting lists by
//! ascending cardinality. Automatically stays zero-overhead on tiny repos (`< 64` files) and
//! safely falls back to full scan whenever a query lacks a safe 3-byte literal run.

use ignore::WalkBuilder;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use zenko_core::{
    DEFAULT_SEARCH_MAX_FILE_BYTES, DEFAULT_SEARCH_SKIP_DIRS, TRIGRAM_MAX_FILE_BYTES,
    TRIGRAM_MAX_INDEXED_FILES, TRIGRAM_MAX_WORKSPACES_CACHED, TRIGRAM_MIN_REPO_FILES,
    TRIGRAM_QUERY_MIN_RUN_BYTES,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrigramQueryPlan {
    Candidates(Vec<PathBuf>),
    FullScan,
}

#[derive(Debug, Clone)]
struct IndexedFileRecord {
    abs_path: PathBuf,
    mtime_ns: u64,
    len: u64,
    trigrams: Vec<u32>,
}

struct WorkspaceTrigramIndex {
    root: PathBuf,
    files: Vec<IndexedFileRecord>,
    path_to_id: HashMap<PathBuf, u32>,
    postings: HashMap<u32, Vec<u32>>,
    last_touch_seq: u64,
}

struct TrigramCacheState {
    seq: u64,
    workspaces: Vec<WorkspaceTrigramIndex>,
}

static TRIGRAM_CACHE: OnceLock<Mutex<TrigramCacheState>> = OnceLock::new();

fn pack_trigram(a: u8, b: u8, c: u8) -> u32 {
    ((a.to_ascii_lowercase() as u32) << 16)
        | ((b.to_ascii_lowercase() as u32) << 8)
        | (c.to_ascii_lowercase() as u32)
}

fn collect_trigrams_from_slice(bytes: &[u8], out: &mut Vec<u32>) {
    if bytes.len() < TRIGRAM_QUERY_MIN_RUN_BYTES {
        return;
    }
    for win in bytes.windows(3) {
        out.push(pack_trigram(win[0], win[1], win[2]));
    }
}

/// Extracts unique sorted ASCII-lowercased trigrams from a file's raw bytes.
/// Returns `None` for binary files (containing NUL byte) so binary blobs are skipped cleanly.
fn extract_file_trigrams(bytes: &[u8]) -> Option<Vec<u32>> {
    if bytes.contains(&0) {
        return None;
    }
    if bytes.len() < TRIGRAM_QUERY_MIN_RUN_BYTES {
        return Some(Vec::new());
    }
    let mut tris = Vec::with_capacity(bytes.len().min(8_192));
    collect_trigrams_from_slice(bytes, &mut tris);
    tris.sort_unstable();
    tris.dedup();
    Some(tris)
}

/// Deterministic zero-regex literal-run extractor for both fixed strings and regex queries.
/// Guarantees zero false negatives: any top-level alternation `|` or optional/repeating group
/// conservatively drops uncertain bytes or returns empty (triggering safe `FullScan`).
pub fn extract_query_trigrams(pattern: &str, fixed_strings: bool) -> Vec<u32> {
    let bytes = pattern.as_bytes();
    if bytes.len() < TRIGRAM_QUERY_MIN_RUN_BYTES {
        return Vec::new();
    }
    if fixed_strings {
        let mut tris = Vec::with_capacity(bytes.len());
        collect_trigrams_from_slice(bytes, &mut tris);
        tris.sort_unstable();
        tris.dedup();
        return tris;
    }

    // Pure byte-FSM extracting guaranteed required literal runs from a regex pattern.
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut i = 0usize;
    let mut paren_depth = 0usize;
    let mut in_char_class = false;

    while i < bytes.len() {
        let b = bytes[i];
        match (in_char_class, b) {
            (true, b'\\') => {
                i = (i + 2).min(bytes.len());
            }
            (true, b']') => {
                in_char_class = false;
                i += 1;
            }
            (true, _) => {
                i += 1;
            }
            (false, b'\\') => {
                match bytes.get(i + 1).copied() {
                    Some(esc) if matches!(esc, b'.' | b'+' | b'*' | b'?' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'|' | b'^' | b'$' | b'\\') => {
                        if paren_depth == 0 {
                            cur.push(esc);
                        }
                        i += 2;
                    }
                    _ => {
                        // Shorthand classes (\d, \w, \s, \b, etc.) break literal run
                        if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                            runs.push(std::mem::take(&mut cur));
                        } else {
                            cur.clear();
                        }
                        i = (i + 2).min(bytes.len());
                    }
                }
            }
            (false, b'[') => {
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                in_char_class = true;
                i += 1;
            }
            (false, b'(') => {
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                paren_depth += 1;
                i += 1;
            }
            (false, b')') => {
                paren_depth = paren_depth.saturating_sub(1);
                i += 1;
            }
            (false, b'|') if paren_depth == 0 => {
                // Top-level alternation means no single literal run is guaranteed across all branches.
                return Vec::new();
            }
            (false, b'?' | b'*') => {
                // Previous literal byte may occur 0 times -> pop it and flush run.
                cur.pop();
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                i += 1;
            }
            (false, b'+') => {
                // Preceding byte occurs >= 1 time (still valid for run ending at it, break after).
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                i += 1;
            }
            (false, b'{') => {
                // Conservative: `{0,n}` could drop previous char -> pop and skip until `}`.
                cur.pop();
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
            }
            (false, b'.' | b'^' | b'$' | b'|') => {
                if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
                    runs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                i += 1;
            }
            (false, lit) => {
                if paren_depth == 0 {
                    cur.push(lit);
                }
                i += 1;
            }
        }
    }
    if cur.len() >= TRIGRAM_QUERY_MIN_RUN_BYTES {
        runs.push(cur);
    }

    let mut tris = Vec::new();
    for run in runs {
        collect_trigrams_from_slice(&run, &mut tris);
    }
    tris.sort_unstable();
    tris.dedup();
    tris
}

fn intersect_sorted_u32(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn file_mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

struct WorkspaceFileScan {
    indexed_sigs: BTreeMap<PathBuf, (u64, u64)>,
    oversized_passthrough: Vec<PathBuf>,
}

fn collect_workspace_file_sigs(root: &Path) -> Option<WorkspaceFileScan> {
    let mut map = BTreeMap::new();
    let mut oversized = Vec::new();
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
        if map.len() >= TRIGRAM_MAX_INDEXED_FILES {
            return None;
        }
        let Ok(meta) = dent.metadata() else {
            continue;
        };
        if meta.len() > TRIGRAM_MAX_FILE_BYTES {
            oversized.push(dent.into_path());
            continue;
        }
        map.insert(dent.into_path(), (file_mtime_ns(&meta), meta.len()));
    }
    Some(WorkspaceFileScan {
        indexed_sigs: map,
        oversized_passthrough: oversized,
    })
}

impl WorkspaceTrigramIndex {
    fn rebuild_postings(&mut self) {
        self.postings.clear();
        for (idx, rec) in self.files.iter().enumerate() {
            let fid = idx as u32;
            for &tri in &rec.trigrams {
                self.postings.entry(tri).or_default().push(fid);
            }
        }
    }

    fn sync_with_disk(&mut self, current: &BTreeMap<PathBuf, (u64, u64)>) {
        let mut changed = false;
        let mut next_files: Vec<IndexedFileRecord> = Vec::with_capacity(current.len());
        let mut next_map: HashMap<PathBuf, u32> = HashMap::with_capacity(current.len());

        for (path, &(mtime_ns, len)) in current {
            let reused = self
                .path_to_id
                .get(path)
                .and_then(|&id| self.files.get(id as usize))
                .filter(|rec| rec.mtime_ns == mtime_ns && rec.len == len)
                .cloned();
            let rec = match reused {
                Some(r) => r,
                None => {
                    changed = true;
                    let tris = std::fs::read(path)
                        .ok()
                        .and_then(|b| extract_file_trigrams(&b))
                        .unwrap_or_default();
                    IndexedFileRecord {
                        abs_path: path.clone(),
                        mtime_ns,
                        len,
                        trigrams: tris,
                    }
                }
            };
            let new_id = next_files.len() as u32;
            next_map.insert(path.clone(), new_id);
            next_files.push(rec);
        }

        if next_files.len() != self.files.len() {
            changed = true;
        }
        if changed || self.postings.is_empty() {
            self.files = next_files;
            self.path_to_id = next_map;
            self.rebuild_postings();
        }
    }

    fn lookup_candidates(&self, trigrams: &[u32]) -> Vec<PathBuf> {
        let mut lists: Vec<&[u32]> = Vec::with_capacity(trigrams.len());
        for tri in trigrams {
            let Some(pl) = self.postings.get(tri) else {
                return Vec::new();
            };
            lists.push(pl.as_slice());
        }
        if lists.is_empty() {
            return self.files.iter().map(|f| f.abs_path.clone()).collect();
        }
        lists.sort_by_key(|l| l.len());
        let mut acc: Vec<u32> = lists[0].to_vec();
        for list in &lists[1..] {
            acc = intersect_sorted_u32(&acc, list);
            if acc.is_empty() {
                break;
            }
        }
        acc.into_iter()
            .filter_map(|id| self.files.get(id as usize).map(|f| f.abs_path.clone()))
            .collect()
    }
}

/// Queries the bounded-RAM lazy workspace trigram index for candidate files matching `pattern`.
/// Automatically bypasses indexing on small subtrees (`< TRIGRAM_MIN_REPO_FILES`) or giant
/// corpora (`> TRIGRAM_MAX_INDEXED_FILES`) and falls back to `FullScan` when no 3-byte literal exists.
pub fn query_trigram_candidates(
    root: &Path,
    pattern: &str,
    fixed_strings: bool,
) -> TrigramQueryPlan {
    query_trigram_candidates_with_min_files(root, pattern, fixed_strings, TRIGRAM_MIN_REPO_FILES)
}

pub(crate) fn query_trigram_candidates_with_min_files(
    root: &Path,
    pattern: &str,
    fixed_strings: bool,
    min_files: usize,
) -> TrigramQueryPlan {
    if !root.is_dir() {
        return TrigramQueryPlan::FullScan;
    }
    let trigrams = extract_query_trigrams(pattern, fixed_strings);
    if trigrams.is_empty() {
        return TrigramQueryPlan::FullScan;
    }
    let Some(scan) = collect_workspace_file_sigs(root) else {
        return TrigramQueryPlan::FullScan;
    };
    if scan.indexed_sigs.len() < min_files {
        return TrigramQueryPlan::FullScan;
    }

    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cache = TRIGRAM_CACHE.get_or_init(|| {
        Mutex::new(TrigramCacheState {
            seq: 0,
            workspaces: Vec::new(),
        })
    });
    let Ok(mut guard) = cache.lock() else {
        return TrigramQueryPlan::FullScan;
    };
    guard.seq = guard.seq.wrapping_add(1);
    let seq = guard.seq;

    let idx_pos = match guard.workspaces.iter().position(|w| w.root == canon_root) {
        Some(pos) => pos,
        None => {
            if guard.workspaces.len() >= TRIGRAM_MAX_WORKSPACES_CACHED
                && let Some((evict_idx, _)) = guard
                    .workspaces
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, w)| w.last_touch_seq)
            {
                guard.workspaces.remove(evict_idx);
            }
            guard.workspaces.push(WorkspaceTrigramIndex {
                root: canon_root,
                files: Vec::new(),
                path_to_id: HashMap::new(),
                postings: HashMap::new(),
                last_touch_seq: seq,
            });
            guard.workspaces.len() - 1
        }
    };

    let ws_idx = &mut guard.workspaces[idx_pos];
    ws_idx.last_touch_seq = seq;
    ws_idx.sync_with_disk(&scan.indexed_sigs);
    let mut cands = ws_idx.lookup_candidates(&trigrams);
    cands.extend(scan.oversized_passthrough);
    TrigramQueryPlan::Candidates(cands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trigrams_safely_from_fixed_and_regex_patterns() {
        assert!(!extract_query_trigrams("handle_auth", true).is_empty());
        assert!(extract_query_trigrams("ab", true).is_empty());
        // Alternation at top level safely falls back to empty (no false negatives)
        assert!(extract_query_trigrams("foo|bar", false).is_empty());
        // Required literal prefix before regex group yields valid trigrams
        let re_tris = extract_query_trigrams(r"fn\s+compute_tax\(", false);
        assert!(!re_tris.is_empty());
        assert!(re_tris.contains(&pack_trigram(b't', b'a', b'x')));
    }

    #[test]
    fn trigram_index_filters_candidates_and_refreshes_on_edit() {
        let dir = std::env::temp_dir().join(format!("zenko_tgrep_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("alpha.rs"), "pub fn settle_invoice() {}\n").expect("a");
        std::fs::write(dir.join("beta.rs"), "pub fn render_button() {}\n").expect("b");
        std::fs::write(dir.join("gamma.rs"), "pub fn parse_config() {}\n").expect("c");

        let plan = query_trigram_candidates_with_min_files(&dir, "settle_invoice", true, 1);
        match plan {
            TrigramQueryPlan::Candidates(cands) => {
                assert_eq!(cands.len(), 1);
                assert!(cands[0].ends_with("alpha.rs"));
            }
            TrigramQueryPlan::FullScan => panic!("expected Candidates"),
        }

        // Edit beta.rs so it now also matches; incremental sync must surface both files!
        std::fs::write(
            dir.join("beta.rs"),
            "pub fn render_button() { settle_invoice(); }\n",
        )
        .expect("b edit");
        let plan2 = query_trigram_candidates_with_min_files(&dir, "settle_invoice", true, 1);
        match plan2 {
            TrigramQueryPlan::Candidates(cands) => {
                assert_eq!(cands.len(), 2);
            }
            TrigramQueryPlan::FullScan => panic!("expected Candidates after edit"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}
