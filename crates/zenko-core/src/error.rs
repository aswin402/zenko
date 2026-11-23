use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZenkoError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("approval required: {0}")]
    ApprovalRequired(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    #[error("vault error: {0}")]
    Vault(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("cancelled")]
    Cancelled,

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ZenkoError>;
