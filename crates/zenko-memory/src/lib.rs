pub mod algorithms;
pub mod compress;
pub mod snapshot;
pub mod sqlite;

pub use algorithms::{
    FtsQueryPlan, build_fts5_queries, build_fts5_query_plan, compute_simhash64,
    extract_memory_file_paths, extract_searchable_event_text, filter_query_terms,
    find_missing_workspace_paths, hamming_distance, hybrid_memory_score, is_generic_code_term,
    parse_since_window, project_scope_for_workspace, relaxed_term_overlap, resolve_git_root,
    sanitize_memory_text, sanitize_memory_text_with_secrets, shingle_jaccard, tokenize_terms,
};
pub use compress::{MarkdownCompressionOutcome, compress_markdown_doc};
pub use snapshot::{SkillDoc, SnapshotManager, WorkspaceSnapshot, estimate_tokens};
pub use sqlite::{SqliteDashboardMetrics, SqliteStore};
