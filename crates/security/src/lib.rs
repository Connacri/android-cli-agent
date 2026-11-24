//! Explicit human confirmation gates for destructive operations.
//!
//! Policy (see `docs/SECURITY.md`): destructive commands (`apps uninstall`,
//! `apps clear-data`) must never run without an explicit human validation.
//! `--yes` records an explicit pre-authorization for non-interactive use
//! (CI, agent harnesses where the human passed the flag themselves).

use std::io::{BufRead, Write};
use thiserror::Error;

/// Errors produced while gating a destructive operation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfirmationError {
    /// The human answered something other than yes.
    #[error("operation cancelled: confirmation declined")]
    Declined,
    #[error("confirmation io error: {0}")]
    Io(String),
}

/// Actions that always require explicit confirmation.
///
/// Keep this list in sync with `docs/COMMANDS.md`.
pub fn is_destructive(action: &str) -> bool {
    matches!(action, "apps uninstall" | "apps clear-data")
}

/// Core gate, fully testable: reads one answer from `reader`,
/// writes the prompt to `writer`.
///
/// Accepted answers: `y`, `yes` (case-insensitive). Everything else,
/// including an empty line, declines.
pub fn confirm_with<R: BufRead, W: Write>(
    action: &str,
    target: &str,
    assume_yes: bool,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), ConfirmationError> {
    if assume_yes {
        return Ok(());
    }
    write!(
        writer,
        "CONFIRM DESTRUCTIVE: {action} on '{target}'? [y/N] "
    )
    .and_then(|()| writer.flush())
    .map_err(|e| ConfirmationError::Io(e.to_string()))?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| ConfirmationError::Io(e.to_string()))?;

    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(()),
        _ => Err(ConfirmationError::Declined),
    }
}

/// Interactive gate using the real stdin/stderr.
pub fn require(action: &str, target: &str, assume_yes: bool) -> Result<(), ConfirmationError> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stderr = std::io::stderr();
    let mut writer = stderr.lock();
    confirm_with(action, target, assume_yes, &mut reader, &mut writer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        action: &str,
        target: &str,
        yes: bool,
        input: &str,
    ) -> (Result<(), ConfirmationError>, String) {
        let mut reader = input.as_bytes();
        let mut writer = Vec::new();
        let res = confirm_with(action, target, yes, &mut reader, &mut writer);
        (res, String::from_utf8(writer).unwrap())
    }

    #[test]
    fn assume_yes_skips_prompt() {
        let (res, prompt) = run("apps uninstall", "com.x", true, "");
        assert!(res.is_ok());
        assert!(prompt.is_empty(), "no prompt expected with --yes");
    }

    #[test]
    fn yes_and_y_accept() {
        assert!(run("apps uninstall", "com.x", false, "yes\n").0.is_ok());
        assert!(run("apps clear-data", "com.x", false, "Y\n").0.is_ok());
    }

    #[test]
    fn empty_or_no_declines() {
        assert_eq!(
            run("apps uninstall", "com.x", false, "\n").0,
            Err(ConfirmationError::Declined)
        );
        assert_eq!(
            run("apps uninstall", "com.x", false, "no\n").0,
            Err(ConfirmationError::Declined)
        );
        assert_eq!(
            run("apps uninstall", "com.x", false, "maybe\n").0,
            Err(ConfirmationError::Declined)
        );
    }

    #[test]
    fn prompt_names_action_and_target() {
        let (_, prompt) = run("apps uninstall", "com.example.app", false, "n\n");
        assert!(prompt.contains("apps uninstall"));
        assert!(prompt.contains("com.example.app"));
    }

    #[test]
    fn destructive_registry() {
        assert!(is_destructive("apps uninstall"));
        assert!(is_destructive("apps clear-data"));
        assert!(!is_destructive("apps list"));
        assert!(!is_destructive("apps install"));
        assert!(!is_destructive("devices list"));
    }
}
