//! Optional Appium UiAutomator2 engine **detection**.
//!
//! Appium is a complementary engine: classic operations (devices, apps, ui)
//! never require it. This crate only reports whether the Appium CLI is
//! available on the host and whether the UiAutomator2 server packages are
//! installed on a given device, so agents can decide whether to use it.

use adb_engine::{AdbBackend, AdbError};
use output::Coded;
use serde::Serialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Host-side Appium executable names, in discovery order (npm installs a
/// `.cmd` wrapper on Windows).
#[cfg(windows)]
const APPIUM_NAMES: &[&str] = &["appium.cmd", "appium.ps1", "appium"];
#[cfg(not(windows))]
const APPIUM_NAMES: &[&str] = &["appium"];

/// Package ids of the UiAutomator2 server on the device.
const UIA2_SERVER_PACKAGES: &[&str] = &[
    "io.appium.uiautomator2.server",
    "io.appium.uiautomator2.server.test",
];

/// Errors produced while detecting the optional engine.
#[derive(Debug, Error)]
pub enum AppiumError {
    #[error("adb error while checking appium state: {0}")]
    Adb(#[from] AdbError),
}

impl Coded for AppiumError {
    fn code(&self) -> &'static str {
        match self {
            AppiumError::Adb(e) => e.code(),
        }
    }
}

/// Result of `appium status`: purely informational, never fatal.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AppiumStatus {
    /// Appium CLI found on the host `PATH`.
    pub cli_available: bool,
    /// Resolved path of the Appium executable, when found.
    pub cli_path: Option<String>,
    /// UiAutomator2 server packages present on the device
    /// (`None` when the device could not be queried).
    pub uiautomator2_installed: Option<bool>,
    /// Human-readable guidance for agents/humans.
    pub hint: String,
}

impl AppiumStatus {
    fn hint_for(cli: bool, server: Option<bool>) -> String {
        match (cli, server) {
            (true, Some(true)) => "appium detected: optional engine ready".into(),
            (true, Some(false)) => {
                "appium detected on host but UiAutomator2 server is not installed on the device; \
                 run `appium driver install uiautomator2` and start a session to enable it"
                    .into()
            }
            (true, None) => "appium detected on host; device state unknown".into(),
            (false, _) => {
                "appium not found on PATH; classic android-cli operations remain fully available"
                    .into()
            }
        }
    }
}

/// Look for the Appium CLI on a `PATH`-like string (split with the platform
/// separator). Pure so it is testable without a real installation.
pub fn find_appium_in_path(path_value: &str) -> Option<PathBuf> {
    std::env::split_paths(path_value).find_map(|dir| {
        APPIUM_NAMES
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// True when any of the UiAutomator2 server packages appears in
/// `pm list packages` output.
pub fn parse_uiautomator2_installed(packages_output: &str) -> bool {
    packages_output.lines().any(|line| {
        let line = line.trim();
        UIA2_SERVER_PACKAGES
            .iter()
            .any(|pkg| line == format!("package:{pkg}"))
    })
}

/// Detection over any [`AdbBackend`]. All operations are best-effort: a
/// failing device query degrades to `None`, never to an error the CLI would
/// surface as a failure.
pub struct AppiumEngine<B: AdbBackend> {
    backend: B,
}

impl<B: AdbBackend> AppiumEngine<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Detect Appium on the host (uses `PATH`; respects `ACA_APPIUM_PATH`
    /// as an explicit override) and, when `serial` is given, the
    /// UiAutomator2 server on that device.
    pub async fn status(&self, serial: Option<&str>) -> AppiumStatus {
        let cli_path = explicit_appium().or_else(|| {
            std::env::var_os("PATH")
                .and_then(|p| p.to_str().map(find_appium_in_path))
                .flatten()
        });
        let cli_available = cli_path.is_some();

        let uiautomator2_installed = match serial {
            None => None,
            Some(serial) => {
                let args = ["-s", serial, "shell", "pm", "list", "packages"];
                match self.backend.run(&args).await {
                    Ok(out) if out.success() => Some(parse_uiautomator2_installed(&out.stdout_str())),
                    Ok(_) | Err(_) => None,
                }
            }
        };

        AppiumStatus {
            cli_available,
            cli_path: cli_path.map(|p| p.display().to_string()),
            uiautomator2_installed,
            hint: AppiumStatus::hint_for(cli_available, uiautomator2_installed),
        }
    }
}

/// `ACA_APPIUM_PATH` override (mirrors `ACA_ADB_PATH`).
fn explicit_appium() -> Option<PathBuf> {
    let raw = std::env::var_os("ACA_APPIUM_PATH")?;
    let path = Path::new(&raw);
    path.is_file().then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb_engine::{AdbOutput, AdbError};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockAdb {
        replies: HashMap<String, String>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl MockAdb {
        fn new() -> Self {
            Self {
                replies: HashMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn on(mut self, key: &str, reply: &str) -> Self {
            self.replies.insert(key.to_string(), reply.to_string());
            self
        }
    }

    impl AdbBackend for MockAdb {
        fn run(
            &self,
            args: &[&str],
        ) -> impl std::future::Future<Output = Result<AdbOutput, AdbError>> + Send {
            async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| s.to_string()).collect());
                let key = match args {
                    ["-s", _, rest @ ..] => rest.join(" "),
                    _ => args.join(" "),
                };
                let reply = self.replies.get(&key).cloned().unwrap_or_default();
                Ok(AdbOutput::new(0, reply.into_bytes(), Vec::new()))
            }
        }
    }

    #[test]
    fn parses_uiautomator2_packages() {
        assert!(parse_uiautomator2_installed(
            "package:io.appium.uiautomator2.server\npackage:com.example\n"
        ));
        assert!(parse_uiautomator2_installed(
            "package:io.appium.uiautomator2.server.test\n"
        ));
        assert!(!parse_uiautomator2_installed("package:com.example\n"));
        assert!(!parse_uiautomator2_installed(""));
    }

    #[test]
    fn finds_appium_in_supplied_path() {
        let dir = std::env::temp_dir().join(format!("aca-appium-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(APPIUM_NAMES[0]);
        std::fs::write(&exe, b"stub").unwrap();
        let path_value = std::env::join_paths([&dir]).unwrap();
        let found = find_appium_in_path(&path_value.to_string_lossy());
        assert_eq!(found.as_deref(), Some(exe.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_nothing_on_empty_path() {
        let empty = std::env::join_paths(std::iter::empty::<PathBuf>()).unwrap();
        assert!(find_appium_in_path(&empty.to_string_lossy()).is_none());
    }

    #[tokio::test]
    async fn status_without_device_skips_device_query() {
        let adb = MockAdb::new();
        let engine = AppiumEngine::new(adb);
        let status = engine.status(None).await;
        assert_eq!(status.uiautomator2_installed, None);
        assert!(!status.hint.is_empty());
    }

    #[tokio::test]
    async fn status_reports_device_server_presence() {
        let adb = MockAdb::new().on(
            "shell pm list packages",
            "package:io.appium.uiautomator2.server\npackage:com.example\n",
        );
        let engine = AppiumEngine::new(adb);
        let status = engine.status(Some("SER")).await;
        assert_eq!(status.uiautomator2_installed, Some(true));

        let adb = MockAdb::new().on("shell pm list packages", "package:com.example\n");
        let engine = AppiumEngine::new(adb);
        let status = engine.status(Some("SER")).await;
        assert_eq!(status.uiautomator2_installed, Some(false));
    }

    #[tokio::test]
    async fn failing_device_query_degrades_to_unknown() {
        // Unknown command → empty reply is success; use a non-zero status to
        // simulate a device-side failure.
        struct Failing;
        impl AdbBackend for Failing {
            fn run(
                &self,
                _args: &[&str],
            ) -> impl std::future::Future<Output = Result<AdbOutput, AdbError>> + Send {
                async move { Ok(AdbOutput::new(1, Vec::new(), b"error: device offline".to_vec())) }
            }
        }
        let engine = AppiumEngine::new(Failing);
        let status = engine.status(Some("SER")).await;
        assert_eq!(status.uiautomator2_installed, None, "never fatal");
    }
}
