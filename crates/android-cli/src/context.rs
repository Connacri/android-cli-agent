//! Shared execution context: flags, error mapping and manager factories.

use adb_engine::{Adb, AdbError};
use app_manager::AppError;
use appium_engine::AppiumError;
use device_manager::{DeviceError, DeviceManager};
use output::{exit, Coded};
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;
use ui_engine::UiError;

/// A typed CLI failure: stable machine-readable code, human message and the
/// process exit code to return.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub exit: i32,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            code,
            message: message.into(),
            exit: exit_code,
        }
    }

    /// Map any `Coded` error to a CLI failure. A declined confirmation exits
    /// with 3, everything else with 1.
    pub fn from_coded<E: Coded + fmt::Display>(err: E) -> Self {
        let code = err.code();
        let exit_code = if code == "E_CONFIRM_DECLINED" {
            exit::CANCELLED
        } else {
            exit::ERROR
        };
        Self::new(code, err.to_string(), exit_code)
    }
}

impl From<AdbError> for CliError {
    fn from(err: AdbError) -> Self {
        Self::from_coded(err)
    }
}

impl From<DeviceError> for CliError {
    fn from(err: DeviceError) -> Self {
        Self::from_coded(err)
    }
}

impl From<AppError> for CliError {
    fn from(err: AppError) -> Self {
        Self::from_coded(err)
    }
}

impl From<UiError> for CliError {
    fn from(err: UiError) -> Self {
        Self::from_coded(err)
    }
}

impl From<AppiumError> for CliError {
    fn from(err: AppiumError) -> Self {
        Self::from_coded(err)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        Self::new("E_SERIALIZE", err.to_string(), exit::ERROR)
    }
}

/// Successful command result: JSON payload, human text and exit code.
pub struct Success {
    pub data: serde_json::Value,
    pub text: String,
    pub exit: i32,
}

impl Success {
    pub fn new<T: Serialize>(data: &T, text: impl Into<String>) -> Result<Self, CliError> {
        Ok(Self {
            data: serde_json::to_value(data)?,
            text: text.into(),
            exit: exit::OK,
        })
    }

    /// Success with an explicit exit code (used by `doctor`).
    pub fn with_exit<T: Serialize>(
        data: &T,
        text: impl Into<String>,
        exit_code: i32,
    ) -> Result<Self, CliError> {
        Ok(Self {
            data: serde_json::to_value(data)?,
            text: text.into(),
            exit: exit_code,
        })
    }
}

/// Immutable flags shared by every command.
pub struct Ctx {
    pub json: bool,
    pub yes: bool,
    pub selector: String,
    pub adb: Option<PathBuf>,
}

impl Ctx {
    pub fn new(json: bool, yes: bool, selector: String, adb: Option<PathBuf>) -> Self {
        Self {
            json,
            yes,
            selector,
            adb,
        }
    }

    /// Locate the adb executable (explicit flag > ACA_ADB_PATH > SDK > PATH).
    pub fn find_adb(&self) -> Result<PathBuf, CliError> {
        Ok(Adb::find(self.adb.as_deref())?)
    }

    /// Build the real adb backend.
    pub fn backend(&self) -> Result<Adb, CliError> {
        Ok(Adb::new(self.find_adb()?))
    }

    pub fn devices(&self) -> Result<DeviceManager<Adb>, CliError> {
        Ok(DeviceManager::new(self.backend()?))
    }

    /// Confirmation mode for destructive operations.
    pub fn confirmation(&self) -> app_manager::Confirmation {
        if self.yes {
            app_manager::Confirmation::AssumeYes
        } else {
            app_manager::Confirmation::Interactive
        }
    }
}
