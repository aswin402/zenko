pub mod client;
pub mod dialect;
pub mod discovery;
pub mod sse;

pub use client::UnifiedLlmProvider;
pub use discovery::{CompactModelRegistry, ExternalModelMeta};

/// Backward-compatible alias for `UnifiedLlmProvider`.
pub type OpenRouterProvider = UnifiedLlmProvider;
