//! Tool errors: a stable upper-case code plus a message, reported as `isError` results whose
//! text reads `CODE: message`. Command errors keep the `dpx-core` code (`ErrorCode::as_str`).

use dpx_core::CommandError;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    pub code: String,
    pub message: String,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }
}

impl From<CommandError> for ToolError {
    fn from(e: CommandError) -> Self {
        Self::new(e.code.as_str(), e.message)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

pub fn error_result(e: &ToolError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(e.to_string())])
}

/// A successful result carrying the value as pretty JSON text, or the error as `isError`.
pub fn json_or_error<T: Serialize>(r: Result<T, ToolError>) -> CallToolResult {
    match r {
        Ok(v) => CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&v).expect("serialisable"),
        )]),
        Err(e) => error_result(&e),
    }
}
