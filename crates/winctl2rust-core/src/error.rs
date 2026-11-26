use guap_mcp::McpToolError;
use thiserror::Error;

pub type WinctlResult<T> = Result<T, WinctlError>;

#[derive(Debug, Error)]
pub enum WinctlError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("parse error: {0}")]
    ParseError(String),

    #[error("unsupported on this platform: {0}")]
    UnsupportedOnPlatform(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("hardware unavailable: {0}")]
    HardwareUnavailable(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Win32 API error {code} ({code:#010x}) in {context}")]
    Win32 { code: u32, context: String },

    #[error("table error: {0}")]
    Table(#[from] guap_table::TableError),

    #[error("export error: {0}")]
    Export(#[from] guap_export::ExportError),
}

impl WinctlError {
    pub fn win32(code: u32, context: impl Into<String>) -> Self {
        Self::Win32 {
            code,
            context: context.into(),
        }
    }
}

impl From<WinctlError> for McpToolError {
    fn from(err: WinctlError) -> Self {
        match err {
            WinctlError::InvalidInput(msg) | WinctlError::ParseError(msg) => McpToolError::InvalidParams(msg),
            WinctlError::PolicyDenied(msg) => McpToolError::PolicyDenied(msg),
            WinctlError::Table(e) => McpToolError::Table(e),
            other => McpToolError::ExecutionFailed(other.to_string()),
        }
    }
}
