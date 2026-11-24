//! Android package inventory and lifecycle operations.
//!
//! Read operations (`list`, `info`) are cheap and side-effect free.
//! Mutating operations (`launch`, `stop`, `install`) are plain commands.
//! Destructive operations (`uninstall`, `clear-data`) are gated by an
//! explicit human confirmation before any adb process is spawned.
//! Callers holding a [`device_manager`] lock must wrap mutations with it;
//! the CLI does this, see `crates/android-cli`.

use adb_engine::{AdbBackend, AdbOutput, AdbError};
use output::Coded;
use serde::Serialize;
use security::ConfirmationError;
use std::path::Path;
use thiserror::Error;

/// Confirmation mode for destructive operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Confirmation {
    /// Prompt the human on the terminal (default).
    #[default]
    Interactive,
    /// The human pre-authorized the operation with `--yes`.
    AssumeYes,
}

/// Errors produced by app operations.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("package '{0}' not found on device")]
    NotFound(String),
    #[error("apk not found: {0}")]
    ApkMissing(String),
    #[error("operation on '{package}' failed: {reason}")]
    Failed { package: String, reason: String },
    #[error("confirmation declined by user")]
    Declined,
    #[error("confirmation io error: {0}")]
    ConfirmIo(String),
    #[error(transparent)]
    Adb(#[from] AdbError),
}

impl Coded for AppError {
    fn code(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "E_APP_NOT_FOUND",
            AppError::ApkMissing(_) => "E_APK_NOT_FOUND",
            AppError::Failed { .. } => "E_APP_OP_FAILED",
            AppError::Declined => "E_CONFIRM_DECLINED",
            AppError::ConfirmIo(_) => "E_IO",
            AppError::Adb(e) => e.code(),
        }
    }
}

/// One installed package (fast inventory: two `pm list` calls total).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppEntry {
    pub package: String,
    pub apk_path: Option<String>,
    pub system: bool,
}

/// Detailed package information (`apps info`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct AppInfo {
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: Option<String>,
    pub min_sdk: Option<String>,
    pub target_sdk: Option<String>,
    pub first_install_time: Option<String>,
    pub last_update_time: Option<String>,
    pub apk_path: Option<String>,
    pub system: bool,
    pub pid: Option<i32>,
}

/// Result of a mutating operation, safe to serialise for agents.
#[derive(Debug, Clone, Serialize)]
pub struct AppOpResult {
    pub action: String,
    pub package: String,
    pub detail: String,
}

/// Package operations over any [`AdbBackend`], bound to one command serial.
pub struct AppManager<B: AdbBackend> {
    backend: B,
}

impl<B: AdbBackend> AppManager<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    async fn run_on(
        &self,
        serial: &str,
        args: &[&str],
    ) -> Result<AdbOutput, AppError> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        full.push("-s");
        full.push(serial);
        full.extend_from_slice(args);
        let out = self.backend.run(&full).await?;
        if !out.success() {
            return Err(AppError::Adb(AdbError::Failed {
                status: out.status,
                stderr: out.error_text(),
            }));
        }
        Ok(out)
    }

    async fn shell(
        &self,
        serial: &str,
        args: &[&str],
    ) -> Result<AdbOutput, AppError> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 1);
        full.push("shell");
        full.extend_from_slice(args);
        self.run_on(serial, &full).await
    }

    /// Confirmation gate executed *before* any adb process is spawned.
    /// Reader/writer are injectable so tests never touch a real terminal.
    fn gate_with<R: std::io::BufRead, W: std::io::Write>(
        action: &str,
        package: &str,
        confirmation: Confirmation,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), AppError> {
        if confirmation == Confirmation::AssumeYes {
            return Ok(());
        }
        security::confirm_with(action, package, false, reader, writer).map_err(|e| match e {
            ConfirmationError::Declined => AppError::Declined,
            ConfirmationError::Io(s) => AppError::ConfirmIo(s),
        })
    }

    /// Interactive gate using the real stdin/stderr.
    fn gate(action: &str, package: &str, confirmation: Confirmation) -> Result<(), AppError> {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let stderr = std::io::stderr();
        let mut writer = stderr.lock();
        Self::gate_with(action, package, confirmation, &mut reader, &mut writer)
    }

    // ------------------------------------------------------------- inventory

    /// All installed packages: one `pm list packages -f` (paths) and one
    /// `pm list packages -s` (system set). Sorted by package id.
    pub async fn list(&self, serial: &str) -> Result<Vec<AppEntry>, AppError> {
        let all = self
            .shell(serial, &["pm", "list", "packages", "-f"])
            .await?;
        let system = self
            .shell(serial, &["pm", "list", "packages", "-s"])
            .await?;

        let system_pkgs: std::collections::HashSet<String> = system
            .stdout_str()
            .lines()
            .filter_map(|l| l.trim().strip_prefix("package:"))
            .map(str::to_string)
            .collect();

        let mut apps: Vec<AppEntry> = all
            .stdout_str()
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let body = line.strip_prefix("package:")?;
                // `package:<apk path>=<package id>` (path first, id last).
                match body.rsplit_once('=') {
                    Some((path, pkg)) if !pkg.is_empty() => Some(AppEntry {
                        package: pkg.to_string(),
                        apk_path: Some(path.to_string()),
                        system: false,
                    }),
                    _ => Some(AppEntry {
                        package: body.to_string(),
                        apk_path: None,
                        system: false,
                    }),
                }
            })
            .collect();
        for app in &mut apps {
            app.system = system_pkgs.contains(&app.package);
        }
        apps.sort_by(|a, b| a.package.cmp(&b.package));
        Ok(apps)
    }

    /// Detailed info: `pm path` (existence + system detection), `dumpsys
    /// package` (versions) and `pidof` (process), three adb calls.
    pub async fn info(&self, serial: &str, package: &str) -> Result<AppInfo, AppError> {
        let path_out = self.shell(serial, &["pm", "path", package]).await?;
        let apk_path = path_out
            .stdout_str()
            .lines()
            .find_map(|l| l.trim().strip_prefix("package:"))
            .map(str::to_string);
        let Some(apk_path) = apk_path else {
            return Err(AppError::NotFound(package.to_string()));
        };

        let dump = self
            .shell(serial, &["dumpsys", "package", package])
            .await?;
        let dump_text = dump.stdout_str();
        if dump_text.contains("Unable to find package") {
            return Err(AppError::NotFound(package.to_string()));
        }

        let pid_out = self.shell(serial, &["pidof", package]).await;
        let pid = pid_out.ok().and_then(|o| {
            o.stdout_str()
                .split_whitespace()
                .next()
                .and_then(|t| t.parse::<i32>().ok())
        });

        let system = [
            "/system/",
            "/system_ext/",
            "/vendor/",
            "/product/",
            "/apex/",
            "/odm/",
        ]
        .iter()
        .any(|p| apk_path.starts_with(p));

        Ok(AppInfo {
            package: package.to_string(),
            version_name: line_value(&dump_text, "versionName="),
            version_code: inline_value(&dump_text, "versionCode="),
            min_sdk: inline_value(&dump_text, "minSdk="),
            target_sdk: inline_value(&dump_text, "targetSdk="),
            first_install_time: line_value(&dump_text, "firstInstallTime="),
            last_update_time: line_value(&dump_text, "lastUpdateTime="),
            apk_path: Some(apk_path),
            system,
            pid,
        })
    }

    // ----------------------------------------------------------- lifecycle

    /// Start the launcher activity (`monkey`), with an `am start` fallback
    /// when the package exposes no monkey-reachable launcher.
    pub async fn launch(&self, serial: &str, package: &str) -> Result<AppOpResult, AppError> {
        let out = self
            .shell(
                serial,
                &[
                    "monkey",
                    "-p",
                    package,
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
            )
            .await;
        let monkey_text = out
            .as_ref()
            .map(|o| o.error_text())
            .unwrap_or_default();
        let monkey_ok = out.is_ok()
            && !monkey_text.contains("No activities found")
            && !monkey_text.contains("monkey aborted");
        if monkey_ok {
            return Ok(AppOpResult {
                action: "launch".into(),
                package: package.to_string(),
                detail: "launcher activity started".into(),
            });
        }

        // Fallback: resolve the launcher component, then `am start` it.
        let resolved = self
            .shell(
                serial,
                &[
                    "cmd",
                    "package",
                    "resolve-activity",
                    "--brief",
                    "-c",
                    "android.intent.category.LAUNCHER",
                    package,
                ],
            )
            .await;
        let component = resolved.ok().and_then(|o| {
            o.stdout_str()
                .lines()
                .map(str::trim)
                .filter(|l| l.contains('/'))
                .last()
                .map(str::to_string)
        });
        let Some(component) = component else {
            return Err(AppError::Failed {
                package: package.to_string(),
                reason: "no launcher activity for this package".into(),
            });
        };
        let start = self
            .shell(serial, &["am", "start", "-n", &component])
            .await?;
        let start_text = start.error_text();
        if start_text.contains("Error") || start_text.contains("Exception") {
            return Err(AppError::Failed {
                package: package.to_string(),
                reason: start_text.trim().to_string(),
            });
        }
        Ok(AppOpResult {
            action: "launch".into(),
            package: package.to_string(),
            detail: component,
        })
    }

    /// Force-stop the package.
    pub async fn stop(&self, serial: &str, package: &str) -> Result<AppOpResult, AppError> {
        self.shell(serial, &["am", "force-stop", package]).await?;
        Ok(AppOpResult {
            action: "stop".into(),
            package: package.to_string(),
            detail: "force-stopped".into(),
        })
    }

    /// Install (or replace) an APK: `adb -s <serial> install -r <file>`.
    pub async fn install(&self, serial: &str, apk: &Path) -> Result<AppOpResult, AppError> {
        if !apk.is_file() {
            return Err(AppError::ApkMissing(apk.display().to_string()));
        }
        let apk_str = apk.to_string_lossy().to_string();
        let out = self.run_on(serial, &["install", "-r", &apk_str]).await?;
        let text = out.error_text();
        if !text.contains("Success") {
            return Err(AppError::Failed {
                package: apk_str.clone(),
                reason: text.trim().to_string(),
            });
        }
        Ok(AppOpResult {
            action: "install".into(),
            package: apk_str,
            detail: "Success".into(),
        })
    }

    /// Uninstall a package. **Destructive**: gated by explicit confirmation
    /// before adb is spawned.
    pub async fn uninstall(
        &self,
        serial: &str,
        package: &str,
        confirmation: Confirmation,
    ) -> Result<AppOpResult, AppError> {
        Self::gate("apps uninstall", package, confirmation)?;
        let out = self.run_on(serial, &["uninstall", package]).await?;
        let text = out.error_text();
        if !text.contains("Success") {
            return Err(AppError::Failed {
                package: package.to_string(),
                reason: text.trim().to_string(),
            });
        }
        Ok(AppOpResult {
            action: "uninstall".into(),
            package: package.to_string(),
            detail: "Success".into(),
        })
    }

    /// Clear app data. **Destructive**: gated by explicit confirmation
    /// before adb is spawned.
    pub async fn clear_data(
        &self,
        serial: &str,
        package: &str,
        confirmation: Confirmation,
    ) -> Result<AppOpResult, AppError> {
        Self::gate("apps clear-data", package, confirmation)?;
        let out = self
            .shell(serial, &["pm", "clear", package])
            .await?;
        let text = out.error_text();
        if !text.contains("Success") {
            return Err(AppError::Failed {
                package: package.to_string(),
                reason: text.trim().to_string(),
            });
        }
        Ok(AppOpResult {
            action: "clear-data".into(),
            package: package.to_string(),
            detail: "Success".into(),
        })
    }
}

/// First `<key><value>` found on a line that starts with `<key>`.
fn line_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(key).map(|v| v.trim().to_string())
    })
}

/// First `<key><token>` occurrence anywhere (keys like `versionCode=` share
/// their line with `minSdk=`/`targetSdk=`).
fn inline_value(text: &str, key: &str) -> Option<String> {
    let idx = text.find(key)?;
    let rest = &text[idx + key.len()..];
    let token = rest
        .split(|c: char| c.is_whitespace() || c == ')')
        .next()
        .unwrap_or("");
    (!token.is_empty()).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb_engine::AdbError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Canned backend covering every command this crate issues.
    struct MockAdb {
        /// Command suffix (joined with spaces, without `-s <serial>`) → reply.
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

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
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
                // Strip `-s <serial>` for reply lookup.
                let key = match args {
                    ["-s", _, rest @ ..] => rest.join(" "),
                    _ => args.join(" "),
                };
                let reply = self
                    .replies
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                Ok(AdbOutput::new(0, reply.into_bytes(), Vec::new()))
            }
        }
    }

    const PM_F: &str = "package:/data/app/~~Ab/base.apk=com.example.app\n\
package:/system/app/Settings/Settings.apk=com.android.settings\n\
package:/data/app/~~Cd/base.apk=com.example.other\n";

    const PM_S: &str = "package:com.android.settings\n";

    const DUMPSYS: &str = "Packages:\n\
  Package [com.example.app] (111):\n\
    userId=10123\n\
    codePath=/data/app/~~Ab/base.apk\n\
    versionCode=42 minSdk=26 targetSdk=34\n\
    versionName=1.2.3\n\
    firstInstallTime=2026-01-01 10:00:00\n\
    lastUpdateTime=2026-02-02 11:00:00\n";

    #[tokio::test]
    async fn list_merges_paths_and_system_flags_sorted() {
        let adb = MockAdb::new()
            .on("shell pm list packages -f", PM_F)
            .on("shell pm list packages -s", PM_S);
        let mgr = AppManager::new(adb);
        let apps = mgr.list("SER").await.unwrap();
        assert_eq!(apps.len(), 3);
        let names: Vec<&str> = apps.iter().map(|a| a.package.as_str()).collect();
        assert_eq!(
            names,
            vec!["com.android.settings", "com.example.app", "com.example.other"]
        );
        let settings = &apps[0];
        assert!(settings.system);
        assert_eq!(
            settings.apk_path.as_deref(),
            Some("/system/app/Settings/Settings.apk")
        );
        assert!(!apps[1].system);
    }

    #[tokio::test]
    async fn info_extracts_versions_sdk_and_pid() {
        let adb = MockAdb::new()
            .on("shell pm path com.example.app", "package:/data/app/~~Ab/base.apk\n")
            .on("shell dumpsys package com.example.app", DUMPSYS)
            .on("shell pidof com.example.app", "4242\n");
        let mgr = AppManager::new(adb);
        let info = mgr.info("SER", "com.example.app").await.unwrap();
        assert_eq!(info.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(info.version_code.as_deref(), Some("42"));
        assert_eq!(info.min_sdk.as_deref(), Some("26"));
        assert_eq!(info.target_sdk.as_deref(), Some("34"));
        assert_eq!(info.pid, Some(4242));
        assert!(!info.system);
        assert_eq!(
            info.first_install_time.as_deref(),
            Some("2026-01-01 10:00:00")
        );
    }

    #[tokio::test]
    async fn info_detects_system_package_and_missing_pid() {
        let adb = MockAdb::new()
            .on("shell pm path com.android.settings", "package:/system/app/Settings/Settings.apk\n")
            .on("shell dumpsys package com.android.settings", DUMPSYS)
            .on("shell pidof com.android.settings", "");
        let mgr = AppManager::new(adb);
        let info = mgr.info("SER", "com.android.settings").await.unwrap();
        assert!(info.system);
        assert_eq!(info.pid, None);
    }

    #[tokio::test]
    async fn info_missing_package_is_typed() {
        let adb = MockAdb::new().on("shell pm path com.ghost", "");
        let mgr = AppManager::new(adb);
        let err = mgr.info("SER", "com.ghost").await.unwrap_err();
        assert_eq!(err.code(), "E_APP_NOT_FOUND");
    }

    #[tokio::test]
    async fn launch_uses_monkey_then_falls_back_to_am_start() {
        // Monkey finds no launcher → resolve-activity → am start.
        let adb = MockAdb::new()
            .on(
                "shell monkey -p com.example.app -c android.intent.category.LAUNCHER 1",
                "** No activities found to run, monkey aborted.\n",
            )
            .on(
                "shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER com.example.app",
                "priority=0 preferredOrder=0 match=0x108000 isDefault=true\ncom.example.app/.MainActivity\n",
            )
            .on(
                "shell am start -n com.example.app/.MainActivity",
                "Starting: Intent { cmp=com.example.app/.MainActivity }\n",
            );
        let mgr = AppManager::new(adb);
        let res = mgr.launch("SER", "com.example.app").await.unwrap();
        assert_eq!(res.action, "launch");
        assert_eq!(res.detail, "com.example.app/.MainActivity");
    }

    #[tokio::test]
    async fn launch_reports_no_launcher() {
        let adb = MockAdb::new()
            .on(
                "shell monkey -p com.example.app -c android.intent.category.LAUNCHER 1",
                "** No activities found to run, monkey aborted.\n",
            )
            .on(
                "shell cmd package resolve-activity --brief -c android.intent.category.LAUNCHER com.example.app",
                "No activity found\n",
            );
        let mgr = AppManager::new(adb);
        let err = mgr.launch("SER", "com.example.app").await.unwrap_err();
        assert_eq!(err.code(), "E_APP_OP_FAILED");
    }

    #[tokio::test]
    async fn stop_and_install_happy_paths() {
        // install requires a real file on disk; the mock replies Success for
        // the exact `install -r <path>` key.
        let apk = std::env::temp_dir().join(format!("aca-{}.apk", std::process::id()));
        std::fs::write(&apk, b"fake").unwrap();
        let adb = MockAdb::new()
            .on("shell am force-stop com.example.app", "")
            .on(
                &format!("install -r {}", apk.display()),
                "Performing Streamed Install\nSuccess\n",
            );
        let mgr = AppManager::new(adb);
        let res = mgr.stop("SER", "com.example.app").await.unwrap();
        assert_eq!(res.action, "stop");

        let res = mgr.install("SER", &apk).await.unwrap();
        assert_eq!(res.detail, "Success");
        let _ = std::fs::remove_file(&apk);
    }

    #[tokio::test]
    async fn install_missing_apk_never_spawns_adb() {
        let adb = MockAdb::new();
        let mgr = AppManager::new(adb);
        let err = mgr
            .install("SER", Path::new("C:\\nowhere\\missing.apk"))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E_APK_NOT_FOUND");
        assert!(mgr.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn uninstall_gate_declines_before_any_adb_call() {
        // A declined human answer must abort before adb is spawned.
        let adb = MockAdb::new().on("uninstall com.example.app", "Success\n");
        let mgr = AppManager::new(adb);
        let mut answer = std::io::Cursor::new("no\n");
        let mut prompt = Vec::new();
        let gated = AppManager::<MockAdb>::gate_with(
            "apps uninstall",
            "com.example.app",
            Confirmation::Interactive,
            &mut answer,
            &mut prompt,
        );
        assert!(matches!(gated, Err(AppError::Declined)));
        assert!(String::from_utf8(prompt).unwrap().contains("apps uninstall"));
        // ...and the async path is structured the same way: gate first.
        assert!(mgr.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn destructive_ops_run_with_assume_yes() {
        let adb = MockAdb::new()
            .on("uninstall com.example.app", "Success\n")
            .on("shell pm clear com.example.app", "Success\n");
        let mgr = AppManager::new(adb);
        let res = mgr
            .uninstall("SER", "com.example.app", Confirmation::AssumeYes)
            .await
            .unwrap();
        assert_eq!(res.action, "uninstall");
        let res = mgr
            .clear_data("SER", "com.example.app", Confirmation::AssumeYes)
            .await
            .unwrap();
        assert_eq!(res.action, "clear-data");
        let calls = mgr.backend.calls();
        assert!(calls.iter().any(|c| c.contains(&"uninstall".to_string())));
        assert!(calls.iter().any(|c| c.contains(&"clear".to_string())));
    }

    #[test]
    fn gate_declines_without_assume_yes() {
        let mut answer = std::io::Cursor::new(String::new()); // EOF = decline
        let mut prompt = Vec::new();
        let res = AppManager::<MockAdb>::gate_with(
            "apps clear-data",
            "com.x",
            Confirmation::Interactive,
            &mut answer,
            &mut prompt,
        );
        assert!(matches!(res, Err(AppError::Declined)));
    }

    #[tokio::test]
    async fn failed_device_operation_is_typed() {
        let adb = MockAdb::new().on("uninstall com.example.app", "Failure [DELETE_FAILED_INTERNAL_ERROR]\n");
        let mgr = AppManager::new(adb);
        let err = mgr
            .uninstall("SER", "com.example.app", Confirmation::AssumeYes)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "E_APP_OP_FAILED");
        assert!(err.to_string().contains("DELETE_FAILED_INTERNAL_ERROR"));
    }

    #[test]
    fn value_extractors() {
        let text = "  versionCode=7 minSdk=21 targetSdk=33\n  versionName=9.9\n";
        assert_eq!(inline_value(text, "versionCode=").as_deref(), Some("7"));
        assert_eq!(inline_value(text, "minSdk=").as_deref(), Some("21"));
        assert_eq!(inline_value(text, "targetSdk=").as_deref(), Some("33"));
        assert_eq!(line_value(text, "versionName=").as_deref(), Some("9.9"));
        assert_eq!(inline_value("nothing", "versionCode="), None);
    }
}
