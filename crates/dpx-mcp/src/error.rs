//! Tool results (ADR-24, ADR-43): every result is a JSON object with `success`, given both as
//! structured content and, serialised, as the first text block. A failure is an `isError`
//! result `{"success": false, "code": "...", "message": "..."}` with a stable upper-case code;
//! command errors keep the `dpx-core` code (`ErrorCode::as_str`).

use dpx_core::CommandError;
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::{Map, Value};

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

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", message)
    }

    /// The error as the structured `{success: false, code, message}` object.
    pub fn to_json(&self) -> Value {
        serde_json::json!({"success": false, "code": self.code, "message": self.message})
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

/// The `isError` result for `e`.
pub fn error_result(e: &ToolError) -> CallToolResult {
    CallToolResult::structured_error(e.to_json())
}

/// `value` (an object) with `success: true` added.
pub fn success_value<T: Serialize>(value: &T) -> Value {
    let mut map = match serde_json::to_value(value).expect("serialisable") {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("value".into(), other);
            m
        }
    };
    map.insert("success".into(), Value::Bool(true));
    Value::Object(map)
}

/// A successful result: the value with `success: true` as structured content and as text.
pub fn success_result<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::structured(success_value(value))
}

/// A successful result with an image first (for clients that show the first block), then
/// the structured value as text.
pub fn image_result<T: Serialize>(png_base64: String, value: &T) -> CallToolResult {
    let v = success_value(value);
    let mut r = CallToolResult::structured(v.clone());
    r.content = vec![
        ContentBlock::image(png_base64, "image/png"),
        ContentBlock::text(v.to_string()),
    ];
    r
}

/// `Ok` as [`success_result`], `Err` as [`error_result`].
pub fn json_or_error<T: Serialize>(r: Result<T, ToolError>) -> CallToolResult {
    match r {
        Ok(v) => success_result(&v),
        Err(e) => error_result(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_carry_success_and_structured_content() {
        let ok = success_result(&serde_json::json!({"revision": 3}));
        assert_eq!(ok.is_error, Some(false));
        let s = ok.structured_content.clone().unwrap();
        assert_eq!(s["success"], true);
        assert_eq!(s["revision"], 3);
        let text: Value = serde_json::from_str(&ok.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(text, s);

        let err = error_result(&ToolError::new("FILE_EXISTS", "a.iff: exists"));
        assert_eq!(err.is_error, Some(true));
        let s = err.structured_content.unwrap();
        assert_eq!(
            s,
            serde_json::json!({"success": false, "code": "FILE_EXISTS", "message": "a.iff: exists"})
        );
    }
}
