//! Uniform output envelope for `android-cli-agent`.
//!
//! Every `--json` invocation prints exactly one document:
//! `{"ok":true,"data":...}` on success or
//! `{"ok":false,"error":{"code":"E_*","message":"..."}}` on failure.
//! Error codes are stable, machine-readable identifiers for agents.

use serde::Serialize;
use std::io::Write;

/// Output format selected by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

impl Format {
    pub fn from_flag(json: bool) -> Self {
        if json {
            Format::Json
        } else {
            Format::Text
        }
    }
}

/// Body of an error envelope.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

/// Error envelope: `{"ok":false,"error":{...}}`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ErrorEnvelope {
    pub ok: bool,
    pub error: ErrorBody,
}

impl ErrorEnvelope {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}

/// Success envelope: `{"ok":true,"data":...}`.
#[derive(Debug, Serialize)]
pub struct SuccessEnvelope<T: Serialize> {
    pub ok: bool,
    pub data: T,
}

impl<T: Serialize> SuccessEnvelope<T> {
    pub fn new(data: T) -> Self {
        Self { ok: true, data }
    }
}

/// Process exit codes shared by the whole CLI.
pub mod exit {
    /// Success.
    pub const OK: i32 = 0;
    /// Runtime failure (device, app, ADB, IO...).
    pub const ERROR: i32 = 1;
    /// Invalid usage (also used by clap).
    pub const USAGE: i32 = 2;
    /// Human declined a destructive confirmation.
    pub const CANCELLED: i32 = 3;
}

/// Errors raised while serialising or printing output.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("output io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json serialisation error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Print one success envelope as JSON (single line, trailing newline).
pub fn print_json_success<T: Serialize>(data: &T) -> Result<(), OutputError> {
    let envelope = SuccessEnvelope::new(data);
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(&envelope)?)?;
    Ok(())
}

/// Print one error envelope as JSON (single line, trailing newline).
pub fn print_json_error(code: &str, message: &str) -> Result<(), OutputError> {
    let envelope = ErrorEnvelope::new(code, message);
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(&envelope)?)?;
    Ok(())
}

/// Trait for error types that expose a stable machine-readable code.
pub trait Coded {
    fn code(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_shape() {
        let json =
            serde_json::to_string(&SuccessEnvelope::new(serde_json::json!({"n": 1}))).unwrap();
        assert_eq!(json, r#"{"ok":true,"data":{"n":1}}"#);
    }

    #[test]
    fn error_envelope_shape() {
        let json =
            serde_json::to_string(&ErrorEnvelope::new("E_DEVICE_NOT_FOUND", "nope")).unwrap();
        assert_eq!(
            json,
            r#"{"ok":false,"error":{"code":"E_DEVICE_NOT_FOUND","message":"nope"}}"#
        );
    }

    #[test]
    fn format_from_flag() {
        assert_eq!(Format::from_flag(true), Format::Json);
        assert_eq!(Format::from_flag(false), Format::Text);
    }

    #[test]
    fn exit_codes_are_distinct() {
        let codes = [exit::OK, exit::ERROR, exit::USAGE, exit::CANCELLED];
        let mut seen = std::collections::HashSet::new();
        for c in codes {
            assert!(seen.insert(c), "duplicate exit code {c}");
        }
    }
}
