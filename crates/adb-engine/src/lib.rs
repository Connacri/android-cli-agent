//! Thin async wrapper around the official Android `adb` binary
//! (SDK Platform-Tools). This crate never re-implements ADB: it locates the
//! executable, spawns it with a hard timeout, captures raw bytes (needed for
//! `screencap`), classifies classic device errors and parses the two textual
//! protocols we rely on (`adb devices -l`, `getprop`, `adb version`).

use output::Coded;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Default per-command timeout. ADB is local; anything longer than this is a
/// hang (daemon restart, unauthorized prompt, dead transport).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Environment variable overriding adb discovery (used by tests and air-gapped
/// setups). Takes precedence over `ANDROID_HOME`/`ANDROID_SDK_ROOT`/`PATH`.
pub const ADB_PATH_ENV: &str = "ACA_ADB_PATH";

/// Errors produced while talking to adb.
#[derive(Debug, Error)]
pub enum AdbError {
    #[error(
        "adb executable not found (searched {ADB_PATH_ENV}, ANDROID_HOME, ANDROID_SDK_ROOT, PATH); \
         install Android SDK Platform-Tools"
    )]
    NotFound,
    #[error("failed to spawn adb at {path}: {source}")]
    Spawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("adb io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("adb timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },
    #[error("adb exited with code {status}: {stderr}")]
    Failed { status: i32, stderr: String },
    #[error("device '{0}' not found")]
    DeviceNotFound(String),
    #[error("device '{0}' is offline")]
    DeviceOffline(String),
    #[error("device '{0}' is unauthorized: accept the USB debugging prompt on the phone")]
    Unauthorized(String),
}

impl Coded for AdbError {
    fn code(&self) -> &'static str {
        match self {
            AdbError::NotFound => "E_ADB_NOT_FOUND",
            AdbError::Spawn { .. } => "E_ADB_SPAWN",
            AdbError::Io(_) => "E_IO",
            AdbError::Timeout { .. } => "E_ADB_TIMEOUT",
            AdbError::Failed { .. } => "E_ADB_FAILED",
            AdbError::DeviceNotFound(_) => "E_DEVICE_NOT_FOUND",
            AdbError::DeviceOffline(_) => "E_DEVICE_OFFLINE",
            AdbError::Unauthorized(_) => "E_DEVICE_UNAUTHORIZED",
        }
    }
}

/// Raw result of one adb invocation. Bytes are kept verbatim because
/// `exec-out screencap -p` returns a PNG.
#[derive(Debug, Clone)]
pub struct AdbOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl AdbOutput {
    pub fn new(status: i32, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }

    pub fn success(&self) -> bool {
        self.status == 0
    }

    pub fn stdout_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    /// Combined textual context used for error classification.
    pub fn error_text(&self) -> String {
        format!("{}{}", self.stdout_str(), self.stderr_str())
    }
}

/// Transport hint inferred from the serial, before any property lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportHint {
    /// Serial contains `host:port` (adb over TCP).
    Wifi,
    /// Serial starts with `emulator-`.
    Emulator,
    /// Everything else: USB (or a docked/unknown physical transport).
    Usb,
    /// Serial could not be interpreted.
    Unknown,
}

impl TransportHint {
    pub fn from_serial(serial: &str) -> Self {
        if serial.starts_with("emulator-") {
            TransportHint::Emulator
        } else if let Some((host, port)) = serial.rsplit_once(':') {
            if !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
                TransportHint::Wifi
            } else {
                TransportHint::Unknown
            }
        } else {
            // No colon: a hardware serial reached over USB (or a dock).
            TransportHint::Usb
        }
    }
}

/// Connection state of one adb entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    Device,
    Offline,
    Unauthorized,
    Recovery,
    Sideload,
    Bootloader,
    /// `no permissions` (udev rules / driver problem on Linux).
    NoPermissions,
    /// Any future/unknown state string, preserved verbatim.
    Other(String),
}

impl DeviceState {
    fn parse(token: &str) -> Self {
        match token {
            "device" => DeviceState::Device,
            "offline" => DeviceState::Offline,
            "unauthorized" => DeviceState::Unauthorized,
            "recovery" => DeviceState::Recovery,
            "sideload" => DeviceState::Sideload,
            "bootloader" => DeviceState::Bootloader,
            other => DeviceState::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            DeviceState::Device => "device",
            DeviceState::Offline => "offline",
            DeviceState::Unauthorized => "unauthorized",
            DeviceState::Recovery => "recovery",
            DeviceState::Sideload => "sideload",
            DeviceState::Bootloader => "bootloader",
            DeviceState::NoPermissions => "no permissions",
            DeviceState::Other(s) => s.as_str(),
        }
    }
}

/// One line of `adb devices -l`, with its `key:value` extras preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub serial: String,
    pub state: DeviceState,
    pub transport: TransportHint,
    /// Extra columns such as `product:`, `model:`, `device:`, `transport_id:`.
    pub extras: BTreeMap<String, String>,
}

/// Parse the output of `adb devices -l`.
///
/// Tolerates daemon noise (`* daemon started successfully *`), the header line
/// and entries without extras.
pub fn parse_devices_output(text: &str) -> Vec<DeviceEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('*')
            || line.starts_with("List of devices")
        {
            continue;
        }
        let mut tokens = line.split_whitespace().peekable();
        let (Some(serial), Some(state_token)) = (tokens.next(), tokens.next()) else {
            continue;
        };
        // `no permissions (see ...)` spans two tokens; every other state is one.
        let state = if state_token == "no" && tokens.peek().copied() == Some("permissions") {
            tokens.next();
            DeviceState::NoPermissions
        } else {
            DeviceState::parse(state_token)
        };
        let mut extras = BTreeMap::new();
        for token in tokens {
            if let Some((k, v)) = token.split_once(':') {
                extras.insert(k.to_string(), v.to_string());
            }
        }
        entries.push(DeviceEntry {
            serial: serial.to_string(),
            state,
            transport: TransportHint::from_serial(serial),
            extras,
        });
    }
    entries
}

/// Parse `getprop` output: lines shaped `[key]: [value]`.
pub fn parse_getprop(text: &str) -> BTreeMap<String, String> {
    let mut props = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((key, rest)) = rest.split_once("]: [") else {
            continue;
        };
        let Some(value) = rest.strip_suffix(']') else {
            continue;
        };
        props.insert(key.to_string(), value.to_string());
    }
    props
}

/// Extract the platform-tools version from `adb version` output.
/// Example:
/// ```text
/// Android Debug Bridge version 1.0.41
/// Version 35.0.2-12147458
/// ```
/// The `Version` line (platform-tools build) wins over the protocol version.
pub fn parse_adb_version(text: &str) -> Option<String> {
    let mut bridge_version = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Version ") {
            return Some(rest.trim().to_string());
        }
        if bridge_version.is_none() {
            if let Some(rest) = line.strip_prefix("Android Debug Bridge version ") {
                bridge_version = Some(rest.trim().to_string());
            }
        }
    }
    bridge_version
}

/// Map a failed adb invocation to a typed error using the well-known
/// transport messages. Returns `AdbError::Failed` when nothing matches.
pub fn classify_failure(serial: &str, out: &AdbOutput) -> AdbError {
    let text = out.error_text().to_ascii_lowercase();
    if text.contains("unauthorized") {
        AdbError::Unauthorized(serial.to_string())
    } else if text.contains("device offline") || text.contains("' offline") {
        AdbError::DeviceOffline(serial.to_string())
    } else if text.contains("not found") {
        AdbError::DeviceNotFound(serial.to_string())
    } else {
        AdbError::Failed {
            status: out.status,
            stderr: out.error_text().trim().to_string(),
        }
    }
}

/// Backend abstraction: every adb interaction goes through this trait so that
/// device logic can be tested against a mock without a physical phone.
pub trait AdbBackend: Send + Sync {
    /// Run `adb <args...>` and wait for completion (with timeout).
    async fn run(&self, args: &[&str]) -> Result<AdbOutput, AdbError>;
}

/// Real backend spawning the official adb binary.
#[derive(Debug, Clone)]
pub struct Adb {
    path: PathBuf,
    timeout: Duration,
}

impl Adb {
    /// Locate adb: explicit path → `ACA_ADB_PATH` → `ANDROID_HOME` →
    /// `ANDROID_SDK_ROOT` → `PATH`.
    pub fn find(explicit: Option<&Path>) -> Result<PathBuf, AdbError> {
        if let Some(p) = explicit {
            return if p.is_file() {
                Ok(p.to_path_buf())
            } else {
                Err(AdbError::NotFound)
            };
        }
        if let Ok(p) = std::env::var(ADB_PATH_ENV) {
            let p = Path::new(&p);
            if p.is_file() {
                return Ok(p.to_path_buf());
            }
        }
        for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Ok(home) = std::env::var(var) {
                let candidate = Path::new(&home)
                    .join("platform-tools")
                    .join(exe_name());
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        search_path().ok_or(AdbError::NotFound)
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run adb and fail with a typed transport error when the exit code is
    /// non-zero and the output matches a known device condition.
    pub async fn run_checked(
        &self,
        args: &[&str],
        serial: Option<&str>,
    ) -> Result<AdbOutput, AdbError> {
        let out = self.run(args).await?;
        if out.success() {
            return Ok(out);
        }
        let serial = serial.unwrap_or("unknown");
        Err(classify_failure(serial, &out))
    }
}

impl AdbBackend for Adb {
    async fn run(&self, args: &[&str]) -> Result<AdbOutput, AdbError> {
        tracing::debug!(path = %self.path.display(), ?args, "spawn adb");
        let fut = tokio::process::Command::new(&self.path)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output();
        let out = match tokio::time::timeout(self.timeout, fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(if e.kind() == std::io::ErrorKind::NotFound {
                    AdbError::NotFound
                } else {
                    AdbError::Spawn {
                        path: self.path.clone(),
                        source: e,
                    }
                });
            }
            Err(_) => {
                return Err(AdbError::Timeout {
                    timeout_secs: self.timeout.as_secs(),
                });
            }
        };
        Ok(AdbOutput::new(
            out.status.code().unwrap_or(-1),
            out.stdout,
            out.stderr,
        ))
    }
}

#[cfg(windows)]
fn exe_name() -> &'static str {
    "adb.exe"
}

#[cfg(not(windows))]
fn exe_name() -> &'static str {
    "adb"
}

/// Scan `PATH` for the adb executable.
fn search_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(exe_name());
        candidate.is_file().then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICES_FIXTURE: &str = "List of devices attached\n\
R58M12ABCDE            device usb:1-1 product:beyond1lte model:SM_G973F device:beyond1 transport_id:1\n\
192.168.1.42:5555      device product:beyond1lte model:SM_G973F device:beyond1 transport_id:2\n\
emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:3\n\
0123456789ABCDEF       unauthorized transport_id:4\n\
192.168.1.99:5555      offline transport_id:5\n\
* daemon started successfully *\n";

    #[test]
    fn parses_devices_with_extras_and_states() {
        let entries = parse_devices_output(DEVICES_FIXTURE);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].serial, "R58M12ABCDE");
        assert_eq!(entries[0].state, DeviceState::Device);
        assert_eq!(entries[0].transport, TransportHint::Usb);
        assert_eq!(
            entries[0].extras.get("model").map(String::as_str),
            Some("SM_G973F")
        );
        assert_eq!(entries[1].transport, TransportHint::Wifi);
        assert_eq!(entries[1].state, DeviceState::Device);
        assert_eq!(entries[2].transport, TransportHint::Emulator);
        assert_eq!(entries[3].state, DeviceState::Unauthorized);
        assert_eq!(entries[4].state, DeviceState::Offline);
        // the `usb:` extra of the first entry must survive state parsing
        assert_eq!(entries[0].extras.get("usb").map(String::as_str), Some("1-1"));
        // daemon noise is not an entry
        assert!(!entries.iter().any(|e| e.serial.starts_with('*')));
    }

    #[test]
    fn parses_empty_device_list() {
        let entries = parse_devices_output("List of devices attached\n\n");
        assert!(entries.is_empty());
    }

    #[test]
    fn parses_no_permissions_entry() {
        let entries = parse_devices_output(
            "List of devices attached\nXYZ\tno permissions (user in plugdev group); see [http://developer.android.com/tools/device.html]\n",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state, DeviceState::NoPermissions);
    }

    #[test]
    fn transport_hint_rules() {
        assert_eq!(TransportHint::from_serial("192.168.1.42:5555"), TransportHint::Wifi);
        assert_eq!(TransportHint::from_serial("localhost:5037"), TransportHint::Wifi);
        assert_eq!(TransportHint::from_serial("R58M12ABCDE"), TransportHint::Usb);
        assert_eq!(TransportHint::from_serial("emulator-5554"), TransportHint::Emulator);
        // `host:port` requires a numeric port; `a:b` stays unknown
        assert_eq!(TransportHint::from_serial("a:b"), TransportHint::Unknown);
    }

    #[test]
    fn parses_getprop() {
        let fixture = "[ro.serialno]: [R58M12ABCDE]\n[ro.product.model]: [SM_G973F]\n[ro.build.version.release]: [14]\ngarbage line\n";
        let props = parse_getprop(fixture);
        assert_eq!(props.get("ro.serialno").map(String::as_str), Some("R58M12ABCDE"));
        assert_eq!(props.get("ro.product.model").map(String::as_str), Some("SM_G973F"));
        assert_eq!(
            props.get("ro.build.version.release").map(String::as_str),
            Some("14")
        );
        assert_eq!(props.len(), 3);
    }

    #[test]
    fn parses_adb_version() {
        let fixture = "Android Debug Bridge version 1.0.41\nVersion 35.0.2-12147458\nInstalled as C:\\platform-tools\\adb.exe\n";
        assert_eq!(parse_adb_version(fixture).as_deref(), Some("35.0.2-12147458"));
        assert_eq!(
            parse_adb_version("Android Debug Bridge version 1.0.39").as_deref(),
            Some("1.0.39")
        );
        assert_eq!(parse_adb_version(""), None);
    }

    #[test]
    fn classifies_transport_errors() {
        let mk = |stderr: &str| AdbOutput::new(1, Vec::new(), stderr.as_bytes().to_vec());
        assert!(matches!(
            classify_failure("X", &mk("adb: device 'X' not found")),
            AdbError::DeviceNotFound(_)
        ));
        assert!(matches!(
            classify_failure("X", &mk("error: device offline")),
            AdbError::DeviceOffline(_)
        ));
        assert!(matches!(
            classify_failure("X", &mk("adb: device unauthorized")),
            AdbError::Unauthorized(_)
        ));
        assert!(matches!(
            classify_failure("X", &mk("something else")),
            AdbError::Failed { .. }
        ));
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(AdbError::NotFound.code(), "E_ADB_NOT_FOUND");
        assert_eq!(
            AdbError::DeviceNotFound("x".into()).code(),
            "E_DEVICE_NOT_FOUND"
        );
        assert_eq!(
            AdbError::Unauthorized("x".into()).code(),
            "E_DEVICE_UNAUTHORIZED"
        );
        assert_eq!(
            AdbError::Timeout { timeout_secs: 1 }.code(),
            "E_ADB_TIMEOUT"
        );
    }

    #[test]
    fn find_explicit_path_wins() {
        // An existing file (this test file's own manifest) is accepted...
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(Adb::find(Some(&real)).is_ok());
        // ...a missing one is rejected.
        let missing = Path::new("definitely-not-adb-12345");
        assert!(matches!(Adb::find(Some(missing)), Err(AdbError::NotFound)));
    }

    #[test]
    fn find_uses_env_override() {
        // Serialised: env tests race under parallel test threads.
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        std::env::set_var(ADB_PATH_ENV, &real);
        let found = Adb::find(None);
        std::env::remove_var(ADB_PATH_ENV);
        match found {
            Ok(p) => assert_eq!(p, real),
            Err(e) => panic!("expected {real:?}, got error: {e}"),
        }
    }
}
