//! Logical device identity, transport priority and Wi-Fi management.
//!
//! The same phone can appear twice in `adb devices` (USB serial + `ip:port`).
//! This crate merges those entries into one **logical device** keyed on stable
//! Android properties, enforces the transport priority USB > Wi-Fi > offline,
//! refuses ambiguous selections, remembers saved Wi-Fi endpoints for automatic
//! reconnection, and serialises mutations per logical device.

use adb_engine::{
    parse_devices_output, parse_getprop, AdbBackend, AdbError, DeviceState, TransportHint,
};
use output::Coded;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

/// Transport priority ranks (lower is better): USB > Wi-Fi > emulator.
pub const RANK_USB: u8 = 1;
pub const RANK_WIFI: u8 = 2;
pub const RANK_EMULATOR: u8 = 3;

/// Errors produced by device management.
#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("no Android device detected: adb reports no entries")]
    NoDevice,
    #[error("device '{0}' not found (use `android-cli devices list` to see candidates)")]
    NotFound(String),
    #[error(
        "ambiguous device identity ({candidates}); \
         pass --device <serial> to choose explicitly"
    )]
    Ambiguous { candidates: String },
    #[error("device '{0}' is unauthorized: accept the USB debugging prompt on the phone")]
    Unauthorized(String),
    #[error("device '{0}' is offline")]
    Offline(String),
    #[error("wi-fi connect to {endpoint} failed: {reason}")]
    ConnectFailed { endpoint: String, reason: String },
    #[error("wi-fi pairing with {endpoint} failed: {reason}")]
    PairFailed { endpoint: String, reason: String },
    #[error("wi-fi disconnect from {endpoint} failed: {reason}")]
    DisconnectFailed { endpoint: String, reason: String },
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error("state file {path}: {source}")]
    State {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Coded for DeviceError {
    fn code(&self) -> &'static str {
        match self {
            DeviceError::NoDevice => "E_NO_DEVICE",
            DeviceError::NotFound(_) => "E_DEVICE_NOT_FOUND",
            DeviceError::Ambiguous { .. } => "E_DEVICE_AMBIGUOUS",
            DeviceError::Unauthorized(_) => "E_DEVICE_UNAUTHORIZED",
            DeviceError::Offline(_) => "E_DEVICE_OFFLINE",
            DeviceError::ConnectFailed { .. } => "E_WIFI_CONNECT_FAILED",
            DeviceError::PairFailed { .. } => "E_WIFI_PAIR_FAILED",
            DeviceError::DisconnectFailed { .. } => "E_WIFI_DISCONNECT_FAILED",
            DeviceError::Adb(e) => e.code(),
            DeviceError::State { .. } => "E_STATE_IO",
        }
    }
}

/// Transport of one adb entry, ranked USB > Wi-Fi > emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Usb,
    Wifi,
    Emulator,
}

impl Transport {
    pub fn rank(self) -> u8 {
        match self {
            Transport::Usb => RANK_USB,
            Transport::Wifi => RANK_WIFI,
            Transport::Emulator => RANK_EMULATOR,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Usb => "usb",
            Transport::Wifi => "wifi",
            Transport::Emulator => "emulator",
        }
    }

    fn from_hint(hint: TransportHint) -> Self {
        match hint {
            TransportHint::Wifi => Transport::Wifi,
            TransportHint::Emulator => Transport::Emulator,
            _ => Transport::Usb,
        }
    }
}

/// One live adb entry belonging to a logical device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransportEntry {
    pub serial: String,
    pub transport: Transport,
    /// Raw adb state (`device`, `offline`, `unauthorized`, ...).
    pub state: String,
}

impl TransportEntry {
    pub fn is_live(&self) -> bool {
        self.state == "device"
    }
}

/// Stable properties read from `getprop` (subset kept for identity + info).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceProps {
    pub hardware_serial: Option<String>,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    pub brand: Option<String>,
    pub product: Option<String>,
    pub device_name: Option<String>,
    pub android_version: Option<String>,
    pub sdk: Option<String>,
    pub build_fingerprint: Option<String>,
    pub security_patch: Option<String>,
    pub abis: Option<String>,
}

impl DeviceProps {
    fn from_map(props: &BTreeMap<String, String>) -> Self {
        let get = |k: &str| props.get(k).cloned().filter(|v| !v.is_empty());
        Self {
            hardware_serial: get("ro.serialno")
                .or_else(|| get("ro.boot.serialno"))
                .or_else(|| get("ro.boot.serial")),
            model: get("ro.product.model"),
            manufacturer: get("ro.product.manufacturer"),
            brand: get("ro.product.brand"),
            product: get("ro.product.name"),
            device_name: get("ro.product.device"),
            android_version: get("ro.build.version.release"),
            sdk: get("ro.build.version.sdk"),
            build_fingerprint: get("ro.build.fingerprint"),
            security_patch: get("ro.build.version.security_patch"),
            abis: get("ro.product.cpu.abilist"),
        }
    }

    /// Fallback identity when `ro.serialno` is unavailable: several stable
    /// properties combined. Not as strong as a hardware serial, which is why
    /// groups keyed on it are marked ambiguous.
    fn fingerprint(&self) -> Option<String> {
        let model = self.model.as_deref()?;
        let manufacturer = self.manufacturer.as_deref().unwrap_or("unknown");
        let product = self.product.as_deref().unwrap_or("unknown");
        Some(format!("{manufacturer}|{model}|{product}",))
    }
}

/// A phone as the human/agent sees it: one identity, possibly several
/// transports.
#[derive(Debug, Clone, Serialize)]
pub struct LogicalDevice {
    /// Stable logical id: hardware serial when known, else a sanitized
    /// property fingerprint, else the raw serial.
    pub id: String,
    pub hardware_serial: Option<String>,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    pub android_version: Option<String>,
    /// Identity fingerprint (manufacturer|model|product), when derivable.
    pub fingerprint: Option<String>,
    /// `true` when identity comes from a hardware serial or live getprop.
    pub identity_verified: bool,
    /// `true` when identity relies only on fallback properties and could not
    /// exclude a same-model twin.
    pub identity_ambiguous: bool,
    /// Best available state across entries (`device` wins).
    pub state: String,
    /// Serial used for `adb -s` when nothing is pinned: best transport.
    pub best_serial: Option<String>,
    pub best_transport: Option<Transport>,
    /// Wi-Fi host:port if any entry is Wi-Fi.
    pub ip: Option<String>,
    /// All entries, best transport first.
    pub entries: Vec<TransportEntry>,
    pub props: DeviceProps,
}

impl LogicalDevice {
    fn transport_rank(&self) -> u8 {
        self.best_transport.map(|t| t.rank()).unwrap_or(99)
    }

    /// Candidate serials usable for commands, best first.
    pub fn live_serials(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.is_live())
            .map(|e| e.serial.as_str())
            .collect()
    }
}

/// Result of resolving a `--device` selector.
#[derive(Debug, Clone, Serialize)]
pub struct SelectedDevice {
    #[serde(flatten)]
    pub device: LogicalDevice,
    /// Serial pinned for commands (explicit selector wins over best transport).
    pub command_serial: String,
    pub command_transport: Transport,
}

/// Persisted state: Wi-Fi endpoints previously connected by the user, used for
/// automatic reconnection. A flat JSON file, no database.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub wifi_endpoints: Vec<String>,
}

/// Default state directory: `%USERPROFILE%/.android-cli-agent` (or `$HOME`),
/// overridable with `ANDROID_CLI_AGENT_HOME`.
pub fn default_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANDROID_CLI_AGENT_HOME") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".android-cli-agent")
}

pub fn default_state_path() -> PathBuf {
    default_state_dir().join("state.json")
}

/// Device manager over any [`AdbBackend`] (real adb or a test mock).
pub struct DeviceManager<B: AdbBackend> {
    backend: B,
    state_path: PathBuf,
    locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl<B: AdbBackend> DeviceManager<B> {
    pub fn new(backend: B) -> Self {
        Self::with_state_path(backend, default_state_path())
    }

    pub fn with_state_path(backend: B, state_path: PathBuf) -> Self {
        Self {
            backend,
            state_path,
            locks: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    // ------------------------------------------------------------------ state

    pub fn load_state(&self) -> PersistedState {
        match std::fs::read_to_string(&self.state_path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => PersistedState::default(),
        }
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), DeviceError> {
        let path = &self.state_path;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DeviceError::State {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(state).map_err(|e| DeviceError::State {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        })?;
        std::fs::write(&tmp, json).map_err(|source| DeviceError::State {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, path).map_err(|source| DeviceError::State {
            path: path.to_path_buf(),
            source,
        })
    }

    fn remember_endpoint(&self, endpoint: &str) -> Result<(), DeviceError> {
        let mut state = self.load_state();
        if !state.wifi_endpoints.iter().any(|e| e == endpoint) {
            state.wifi_endpoints.push(endpoint.to_string());
            self.save_state(&state)?;
        }
        Ok(())
    }

    fn forget_endpoint(&self, endpoint: &str) -> Result<(), DeviceError> {
        let mut state = self.load_state();
        let before = state.wifi_endpoints.len();
        state.wifi_endpoints.retain(|e| e != endpoint);
        if state.wifi_endpoints.len() != before {
            self.save_state(&state)?;
        }
        Ok(())
    }

    // ----------------------------------------------------------------- locks

    /// Acquire the per-logical-device mutation guard. Two mutations on the
    /// same logical device never run concurrently, whatever transports they
    /// use; different logical devices proceed in parallel.
    pub async fn lock_device(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let arc = {
            let mut map = self.locks.lock().await;
            Arc::clone(
                map.entry(id.to_string())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        arc.lock_owned().await
    }

    // --------------------------------------------------------------- collect

    /// Read `adb devices -l`, fetch stable properties for live entries, group
    /// them into logical devices and apply transport priority.
    ///
    /// When `reconnect` is true, saved Wi-Fi endpoints missing from the live
    /// list are reconnected once (best effort).
    pub async fn collect(&self, reconnect: bool) -> Result<Vec<LogicalDevice>, DeviceError> {
        let raw = self.backend.run(&["devices", "-l"]).await?;
        if !raw.success() {
            return Err(AdbError::Failed {
                status: raw.status,
                stderr: raw.error_text(),
            }
            .into());
        }
        let mut entries = parse_devices_output(&raw.stdout_str());

        if reconnect {
            let saved = self.load_state().wifi_endpoints;
            let missing: Vec<String> = saved
                .iter()
                .filter(|ep| !entries.iter().any(|e| &e.serial == *ep))
                .cloned()
                .collect();
            if !missing.is_empty() {
                tracing::info!(?missing, "reconnecting saved wi-fi endpoints");
                for ep in &missing {
                    let _ = self.backend.run(&["connect", ep]).await;
                }
                let raw = self.backend.run(&["devices", "-l"]).await?;
                if raw.success() {
                    entries = parse_devices_output(&raw.stdout_str());
                }
            }
        }

        // Fetch stable properties for every live entry (best effort: a device
        // dropping mid-collect degrades to "no props" instead of failing).
        let mut props_by_serial: HashMap<String, DeviceProps> = HashMap::new();
        for entry in &entries {
            if entry.state != DeviceState::Device {
                continue;
            }
            let serial = entry.serial.clone();
            let args = ["-s", serial.as_str(), "shell", "getprop"];
            match self.backend.run(&args).await {
                Ok(out) if out.success() => {
                    let map = parse_getprop(&out.stdout_str());
                    let mut props = DeviceProps::from_map(&map);
                    // `adb devices -l` still shows model when getprop fails.
                    if props.model.is_none() {
                        props.model = entry.extras.get("model").cloned();
                    }
                    props_by_serial.insert(serial, props);
                }
                Ok(out) => {
                    tracing::warn!(
                        serial,
                        status = out.status,
                        "getprop failed; identity degraded"
                    );
                }
                Err(e) => tracing::warn!(serial, error = %e, "getprop error; identity degraded"),
            }
        }

        Ok(group_logical_devices(entries, &props_by_serial))
    }

    // --------------------------------------------------------------- resolve

    /// Resolve a selector: `auto` (transport priority + ambiguity guard),
    /// a logical id, a hardware serial, or any live adb serial.
    pub async fn resolve(&self, selector: &str) -> Result<SelectedDevice, DeviceError> {
        let devices = self.collect(true).await?;
        self.resolve_within(&devices, selector)
    }

    /// Pure selection logic (testable without a backend).
    pub fn resolve_within(
        &self,
        devices: &[LogicalDevice],
        selector: &str,
    ) -> Result<SelectedDevice, DeviceError> {
        if selector == "auto" || selector.is_empty() {
            return self.auto_select(devices);
        }
        // Exact match on logical id or any serial (entry or hardware).
        let found = devices.iter().find(|d| {
            d.id == selector
                || d.hardware_serial.as_deref() == Some(selector)
                || d.entries.iter().any(|e| e.serial == selector)
        });
        let Some(device) = found else {
            if devices.is_empty() {
                return Err(DeviceError::NoDevice);
            }
            return Err(DeviceError::NotFound(selector.to_string()));
        };
        // Explicit serial pins the transport; otherwise priority applies.
        let pinned = device.entries.iter().find(|e| e.serial == selector);
        let (serial, transport) = match pinned {
            Some(e) if e.is_live() => (e.serial.clone(), e.transport),
            Some(e) => {
                return Err(match e.state.as_str() {
                    "unauthorized" => DeviceError::Unauthorized(e.serial.clone()),
                    _ => DeviceError::Offline(e.serial.clone()),
                });
            }
            None => {
                let (s, t) = best_live(device).ok_or_else(|| device_error_for(device))?;
                (s.to_string(), t)
            }
        };
        Ok(SelectedDevice {
            device: device.clone(),
            command_serial: serial,
            command_transport: transport,
        })
    }

    fn auto_select(&self, devices: &[LogicalDevice]) -> Result<SelectedDevice, DeviceError> {
        let live: Vec<&LogicalDevice> = devices
            .iter()
            .filter(|d| !d.live_serials().is_empty())
            .collect();
        if live.is_empty() {
            // Report the most informative failure: unauthorized > offline > none.
            if let Some(d) = devices.iter().find(|d| d.state == "unauthorized") {
                return Err(DeviceError::Unauthorized(d.id.clone()));
            }
            if let Some(d) = devices.iter().find(|d| d.state == "offline") {
                return Err(DeviceError::Offline(d.id.clone()));
            }
            return Err(DeviceError::NoDevice);
        }
        // Ambiguity guard: several logical devices sharing one fingerprint and
        // relying on fallback identity cannot be told apart automatically.
        let mut by_fp: HashMap<&str, Vec<&LogicalDevice>> = HashMap::new();
        for d in &live {
            if d.identity_ambiguous {
                if let Some(fp) = d.fingerprint.as_deref() {
                    by_fp.entry(fp).or_default().push(d);
                }
            }
        }
        if let Some((_, group)) = by_fp.iter().find(|(_, g)| g.len() > 1) {
            let candidates = group
                .iter()
                .map(|d| {
                    d.entries
                        .iter()
                        .map(|e| e.serial.clone())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(DeviceError::Ambiguous { candidates });
        }

        let mut sorted: Vec<&&LogicalDevice> = live.iter().collect();
        sorted.sort_by_key(|d| (d.transport_rank(), d.id.clone()));
        let best = *sorted[0];
        let (serial, transport) = best_live(best).ok_or(DeviceError::NoDevice)?;
        Ok(SelectedDevice {
            device: (*best).clone(),
            command_serial: serial.to_string(),
            command_transport: transport,
        })
    }

    // ------------------------------------------------------------- transport

    /// `adb pair IP:PORT [CODE]` (Android 11+ on-screen pairing code).
    pub async fn pair(&self, endpoint: &str, code: Option<&str>) -> Result<(), DeviceError> {
        validate_endpoint(endpoint)?;
        let out = match code {
            Some(code) => self.backend.run(&["pair", endpoint, code]).await?,
            None => self.backend.run(&["pair", endpoint]).await?,
        };
        let text = out.error_text();
        if text.contains("Successfully paired") {
            tracing::info!(endpoint, "paired");
            Ok(())
        } else {
            Err(DeviceError::PairFailed {
                endpoint: endpoint.to_string(),
                reason: text.trim().to_string(),
            })
        }
    }

    /// `adb connect IP:PORT`, then persist the endpoint for auto-reconnect.
    pub async fn connect(&self, endpoint: &str) -> Result<(), DeviceError> {
        validate_endpoint(endpoint)?;
        let out = self.backend.run(&["connect", endpoint]).await?;
        let text = out.error_text().to_ascii_lowercase();
        if text.contains("failed") || text.contains("cannot connect") {
            return Err(DeviceError::ConnectFailed {
                endpoint: endpoint.to_string(),
                reason: out.error_text().trim().to_string(),
            });
        }
        if text.contains("connected") {
            self.remember_endpoint(endpoint)?;
            Ok(())
        } else {
            Err(DeviceError::ConnectFailed {
                endpoint: endpoint.to_string(),
                reason: out.error_text().trim().to_string(),
            })
        }
    }

    /// `adb disconnect` for an endpoint, or for every Wi-Fi entry of a logical
    /// device selector.
    pub async fn disconnect(&self, target: &str) -> Result<Vec<String>, DeviceError> {
        let endpoints: Vec<String> = if target.contains(':') {
            vec![target.to_string()]
        } else {
            // Logical selector: disconnect all its Wi-Fi entries.
            let devices = self.collect(false).await?;
            let device = self
                .resolve_within(&devices, target)
                .map(|s| s.device)
                .map_err(|_| DeviceError::NotFound(target.to_string()))?;
            device
                .entries
                .iter()
                .filter(|e| e.transport == Transport::Wifi)
                .map(|e| e.serial.clone())
                .collect()
        };
        let mut done = Vec::new();
        for ep in &endpoints {
            let out = self.backend.run(&["disconnect", ep]).await?;
            let text = out.error_text().to_ascii_lowercase();
            if text.contains("disconnected") || text.contains("already disconnected") {
                self.forget_endpoint(ep)?;
                done.push(ep.clone());
            } else {
                return Err(DeviceError::DisconnectFailed {
                    endpoint: ep.clone(),
                    reason: out.error_text().trim().to_string(),
                });
            }
        }
        Ok(done)
    }
}

/// Validate `HOST:PORT` before handing it to adb.
fn validate_endpoint(endpoint: &str) -> Result<(), DeviceError> {
    let bad = || DeviceError::ConnectFailed {
        endpoint: endpoint.to_string(),
        reason: "invalid endpoint, expected IP:PORT".to_string(),
    };
    let (host, port) = endpoint.rsplit_once(':').ok_or_else(bad)?;
    if host.is_empty() || port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    Ok(())
}

/// Best live (state=device) entry by transport rank: `Some((serial, transport))`.
fn best_live(device: &LogicalDevice) -> Option<(&str, Transport)> {
    device
        .entries
        .iter()
        .filter(|e| e.is_live())
        .min_by_key(|e| e.transport.rank())
        .map(|e| (e.serial.as_str(), e.transport))
}

/// Why auto-selection failed for a device with no live entry.
fn device_error_for(device: &LogicalDevice) -> DeviceError {
    if device.state == "unauthorized" {
        DeviceError::Unauthorized(device.id.clone())
    } else if device.state == "offline" {
        DeviceError::Offline(device.id.clone())
    } else {
        DeviceError::NoDevice
    }
}

/// Entry keys used for grouping.
enum GroupKey {
    /// Hardware serial: strong identity, USB + Wi-Fi merge safely.
    Hardware(String),
    /// Fallback fingerprint: merge only across different transport kinds.
    Fingerprint(String),
    /// No properties at all (offline/unauthorized): identity unverifiable,
    /// keep the entry alone and never auto-bind it to another transport.
    Serial(String),
}

fn entry_key(serial: &str, transport: Transport, props: Option<&DeviceProps>) -> GroupKey {
    let _ = transport;
    if let Some(hw) = props.and_then(|p| p.hardware_serial.as_ref()) {
        if !hw.is_empty() {
            return GroupKey::Hardware(hw.clone());
        }
    }
    if let Some(fp) = props.and_then(|p| p.fingerprint()) {
        return GroupKey::Fingerprint(fp);
    }
    GroupKey::Serial(serial.to_string())
}

/// Group raw entries + fetched properties into logical devices.
///
/// Rules (see docs/ARCHITECTURE.md):
/// - Same hardware serial over USB and Wi-Fi → one logical device.
/// - Same fallback fingerprint: one USB entry + Wi-Fi entries → merged,
///   flagged `identity_ambiguous` (props only, twins not excluded).
/// - Two USB entries sharing a fingerprint → two phones: keep separate,
///   flag both ambiguous (auto-selection will then refuse).
/// - Entries without properties (offline/unauthorized) stay alone,
///   `identity_verified = false`.
pub fn group_logical_devices(
    entries: Vec<adb_engine::DeviceEntry>,
    props_by_serial: &HashMap<String, DeviceProps>,
) -> Vec<LogicalDevice> {
    struct Group {
        key_id: String,
        hardware_serial: Option<String>,
        fingerprint: Option<String>,
        entries: Vec<adb_engine::DeviceEntry>,
        props: DeviceProps,
        verified: bool,
        ambiguous: bool,
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut by_serial: HashMap<String, usize> = HashMap::new();

    // Pass 1: strong keys (hardware serial), then serial-only keys.
    for entry in entries {
        let props = props_by_serial.get(&entry.serial);
        let live = entry.state == DeviceState::Device;
        match entry_key(&entry.serial, Transport::from_hint(entry.transport), props) {
            GroupKey::Hardware(hw) => {
                if let Some(&idx) = by_serial.get(&hw) {
                    groups[idx].entries.push(entry);
                    groups[idx].verified |= live;
                } else {
                    by_serial.insert(hw.clone(), groups.len());
                    groups.push(Group {
                        key_id: hw.clone(),
                        hardware_serial: Some(hw),
                        fingerprint: props.and_then(|p| p.fingerprint()),
                        entries: vec![entry],
                        props: props.cloned().unwrap_or_default(),
                        verified: live,
                        ambiguous: false,
                    });
                }
            }
            GroupKey::Fingerprint(fp) => {
                // Defer: fingerprint merging depends on transport kinds.
                groups.push(Group {
                    key_id: fp,
                    hardware_serial: None,
                    fingerprint: Some(props.and_then(|p| p.fingerprint()).unwrap_or_default()),
                    entries: vec![entry],
                    props: props.cloned().unwrap_or_default(),
                    verified: live,
                    ambiguous: true,
                });
            }
            GroupKey::Serial(serial) => {
                groups.push(Group {
                    key_id: serial.clone(),
                    hardware_serial: None,
                    fingerprint: None,
                    entries: vec![entry],
                    props: props.cloned().unwrap_or_default(),
                    verified: false,
                    ambiguous: false,
                });
            }
        }
    }

    // Pass 2: fingerprint groups of the same identity merge when at most one
    // is a non-Wi-Fi (physical) entry — that is the USB + Wi-Fi case of one
    // phone. Several physical entries with the same fingerprint are distinct
    // phones: keep them apart, all flagged ambiguous.
    let mut merged: Vec<Group> = Vec::new();
    let fp_indices: Vec<usize> = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.hardware_serial.is_none() && g.fingerprint.is_some())
        .map(|(i, _)| i)
        .collect();
    let mut consumed = vec![false; groups.len()];
    for &i in &fp_indices {
        if consumed[i] {
            continue;
        }
        let fp = groups[i].fingerprint.clone().unwrap_or_default();
        let mut members: Vec<usize> = vec![i];
        for &j in &fp_indices {
            if j != i && !consumed[j] && groups[j].fingerprint.clone().unwrap_or_default() == fp {
                members.push(j);
            }
        }
        let physical = members
            .iter()
            .filter(|&&m| {
                groups[m]
                    .entries
                    .iter()
                    .any(|e| Transport::from_hint(e.transport) != Transport::Wifi)
            })
            .count();
        for &m in &members {
            consumed[m] = true;
        }
        if physical <= 1 {
            // Merge all members into the first one.
            let first = members[0];
            let mut target = Group {
                key_id: fp.clone(),
                hardware_serial: None,
                fingerprint: Some(fp),
                entries: Vec::new(),
                props: DeviceProps::default(),
                verified: false,
                ambiguous: true,
            };
            for m in &members {
                let g = &groups[*m];
                target.entries.extend(g.entries.iter().cloned());
                target.verified |= g.verified;
                if target.props.model.is_none() {
                    target.props = g.props.clone();
                }
            }
            // Merged group is ambiguous only if identity is props-only AND a
            // twin could exist; merging one USB + Wi-Fi is the common case and
            // stays selectable, so keep the flag but not blocking.
            target.ambiguous = true;
            consumed[first] = true;
            merged.push(target);
        } else {
            // Real collision: distinct physical phones. Keep them separate.
            for m in &members {
                let mut g = std::mem::replace(
                    &mut groups[*m],
                    Group {
                        key_id: String::new(),
                        hardware_serial: None,
                        fingerprint: None,
                        entries: Vec::new(),
                        props: DeviceProps::default(),
                        verified: false,
                        ambiguous: false,
                    },
                );
                g.ambiguous = true;
                merged.push(g);
            }
        }
    }
    // Keep non-fingerprint groups (hardware / serial) in the list.
    for (i, g) in groups.into_iter().enumerate() {
        if !consumed[i] {
            merged.push(g);
        }
    }
    let groups = merged;

    // Build logical devices.
    let mut devices: Vec<LogicalDevice> = groups
        .into_iter()
        .map(|g| {
            let mut entries: Vec<TransportEntry> = g
                .entries
                .iter()
                .map(|e| TransportEntry {
                    serial: e.serial.clone(),
                    transport: Transport::from_hint(e.transport),
                    state: e.state.as_str().to_string(),
                })
                .collect();
            entries.sort_by_key(|e| e.transport.rank());
            let best = entries.iter().find(|e| e.is_live());
            let state = entries
                .iter()
                .min_by_key(|e| match e.state.as_str() {
                    "device" => 0u8,
                    "unauthorized" => 1,
                    "recovery" => 2,
                    "sideload" => 2,
                    "offline" => 3,
                    _ => 4,
                })
                .map(|e| e.state.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let ip = entries
                .iter()
                .find(|e| e.transport == Transport::Wifi)
                .map(|e| e.serial.clone());
            LogicalDevice {
                id: g.key_id,
                hardware_serial: g.hardware_serial,
                model: g.props.model.clone(),
                manufacturer: g.props.manufacturer.clone(),
                android_version: g.props.android_version.clone(),
                fingerprint: g.fingerprint.filter(|f| !f.is_empty()),
                identity_verified: g.verified,
                identity_ambiguous: g.ambiguous,
                state,
                best_serial: best.map(|e| e.serial.clone()),
                best_transport: best.map(|e| e.transport),
                ip,
                entries,
                props: g.props,
            }
        })
        .collect();

    // Deterministic output order + unique ids.
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    let mut seen: HashMap<String, u32> = HashMap::new();
    for d in &mut devices {
        let count = seen.entry(d.id.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            let suffix = d
                .entries
                .first()
                .map(|e| format!("-{}", sanitize(&e.serial)))
                .unwrap_or_default();
            d.id = format!("{}{suffix}", d.id);
        }
    }
    devices
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb_engine::{AdbOutput, DeviceEntry};
    use std::sync::Mutex;

    /// Canned adb backend: no phone needed to test grouping and transport.
    struct MockAdb {
        devices: String,
        props: HashMap<String, String>,
        connect_reply: String,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl MockAdb {
        fn new(devices: &str) -> Self {
            Self {
                devices: devices.to_string(),
                props: HashMap::new(),
                connect_reply: String::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_props(mut self, serial: &str, props: &str) -> Self {
            self.props.insert(serial.to_string(), props.to_string());
            self
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl AdbBackend for MockAdb {
        #[allow(clippy::manual_async_fn)]
        fn run(
            &self,
            args: &[&str],
        ) -> impl std::future::Future<Output = Result<AdbOutput, AdbError>> + Send {
            async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| s.to_string()).collect());
                let stdout = match args {
                    ["devices", "-l"] => self.devices.clone(),
                    ["-s", serial, "shell", "getprop"] => {
                        self.props.get(*serial).cloned().unwrap_or_default()
                    }
                    ["connect", ep] => {
                        if self.connect_reply.is_empty() {
                            format!("failed to connect to {ep}: connection refused")
                        } else {
                            self.connect_reply.clone()
                        }
                    }
                    ["disconnect", ep] => format!("disconnected {ep}"),
                    ["pair", ep, _code] => format!("Successfully paired with {ep} [guid=0000]"),
                    ["pair", _ep] => "error: expected 'adb pair <addr> [<code>]'".to_string(),
                    _ => String::new(),
                };
                Ok(AdbOutput::new(0, stdout.into_bytes(), Vec::new()))
            }
        }
    }

    fn tmp_state(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("aca-dm-{name}-{}.json", std::process::id()))
    }

    fn props(serial: &str, model: &str, manufacturer: &str) -> String {
        format!(
            "[ro.serialno]: [{serial}]\n[ro.product.model]: [{model}]\n\
             [ro.product.manufacturer]: [{manufacturer}]\n[ro.product.name]: [{model}]\n\
             [ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n"
        )
    }

    const TWO_TRANSPORTS: &str = "List of devices attached\n\
R58M12ABCDE\tdevice usb:1-1 product:beyond1lte model:SM_G973F device:beyond1\n\
192.168.1.42:5555\tdevice product:beyond1lte model:SM_G973F device:beyond1\n";

    #[tokio::test]
    async fn merges_usb_and_wifi_of_one_phone() {
        let adb = MockAdb::new(TWO_TRANSPORTS)
            .with_props("R58M12ABCDE", &props("R58M12ABCDE", "SM_G973F", "samsung"))
            .with_props(
                "192.168.1.42:5555",
                &props("R58M12ABCDE", "SM_G973F", "samsung"),
            );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("merge"));
        let devices = mgr.collect(false).await.unwrap();
        assert_eq!(devices.len(), 1, "same phone must be one logical device");
        let d = &devices[0];
        assert_eq!(d.id, "R58M12ABCDE");
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.best_transport, Some(Transport::Usb), "USB wins");
        assert_eq!(d.best_serial.as_deref(), Some("R58M12ABCDE"));
        assert!(d.identity_verified);
        assert_eq!(d.ip.as_deref(), Some("192.168.1.42:5555"));
    }

    #[tokio::test]
    async fn auto_select_prefers_usb_then_wifi() {
        let adb = MockAdb::new(TWO_TRANSPORTS)
            .with_props("R58M12ABCDE", &props("R58M12ABCDE", "SM_G973F", "samsung"))
            .with_props(
                "192.168.1.42:5555",
                &props("R58M12ABCDE", "SM_G973F", "samsung"),
            );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("auto"));
        let sel = mgr.resolve("auto").await.unwrap();
        assert_eq!(sel.command_serial, "R58M12ABCDE");
        assert_eq!(sel.command_transport, Transport::Usb);

        // USB gone: same logical device, Wi-Fi taken after identity check.
        let adb = MockAdb::new(
            "List of devices attached\n192.168.1.42:5555\tdevice product:beyond1lte model:SM_G973F device:beyond1\n",
        )
        .with_props("192.168.1.42:5555", &props("R58M12ABCDE", "SM_G973F", "samsung"));
        let mgr = DeviceManager::with_state_path(adb, tmp_state("auto-wifi"));
        let sel = mgr.resolve("auto").await.unwrap();
        assert_eq!(sel.command_transport, Transport::Wifi);
        assert_eq!(sel.device.hardware_serial.as_deref(), Some("R58M12ABCDE"));
    }

    #[tokio::test]
    async fn explicit_serial_pins_transport() {
        let adb = MockAdb::new(TWO_TRANSPORTS)
            .with_props("R58M12ABCDE", &props("R58M12ABCDE", "SM_G973F", "samsung"))
            .with_props(
                "192.168.1.42:5555",
                &props("R58M12ABCDE", "SM_G973F", "samsung"),
            );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("pin"));
        let sel = mgr.resolve("192.168.1.42:5555").await.unwrap();
        assert_eq!(sel.command_serial, "192.168.1.42:5555");
        assert_eq!(sel.command_transport, Transport::Wifi);
        assert_eq!(sel.device.id, "R58M12ABCDE", "still the same logical phone");
    }

    #[tokio::test]
    async fn unauthorized_and_offline_are_reported_not_selected() {
        let adb = MockAdb::new(
            "List of devices attached\nABCDEF\tunauthorized\n192.168.1.9:5555\toffline\n",
        );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("badstates"));
        let err = mgr.resolve("auto").await.unwrap_err();
        assert_eq!(err.code(), "E_DEVICE_UNAUTHORIZED");
    }

    #[tokio::test]
    async fn no_device_at_all_is_reported() {
        let adb = MockAdb::new("List of devices attached\n");
        let mgr = DeviceManager::with_state_path(adb, tmp_state("nodev"));
        assert!(matches!(
            mgr.resolve("auto").await.unwrap_err(),
            DeviceError::NoDevice
        ));
        let devices = mgr.collect(false).await.unwrap();
        assert!(devices.is_empty(), "empty list is a valid inventory");
    }

    #[tokio::test]
    async fn same_model_twins_stay_separate_and_block_auto() {
        // Two physical phones, identical model, no readable hardware serial.
        let devices_out = "List of devices attached\n\
AAA111\tdevice product:p model:Twin device:twin\n\
BBB222\tdevice product:p model:Twin device:twin\n";
        let mut adb = MockAdb::new(devices_out);
        let twin_props = "[ro.product.model]: [Twin]\n[ro.product.manufacturer]: [Acme]\n[ro.product.name]: [p]\n";
        adb.props.insert("AAA111".into(), twin_props.into());
        adb.props.insert("BBB222".into(), twin_props.into());
        let mgr = DeviceManager::with_state_path(adb, tmp_state("twins"));
        let devices = mgr.collect(false).await.unwrap();
        assert_eq!(devices.len(), 2, "two USB phones must stay two devices");
        assert!(devices.iter().all(|d| d.identity_ambiguous));
        let err = mgr.resolve("auto").await.unwrap_err();
        assert_eq!(err.code(), "E_DEVICE_AMBIGUOUS");
        // Explicit selection still works.
        let sel = mgr.resolve("AAA111").await.unwrap();
        assert_eq!(sel.command_serial, "AAA111");
    }

    #[tokio::test]
    async fn offline_usb_entry_is_not_merged_without_identity() {
        // USB entry offline: no getprop possible → cannot verify it is the
        // same phone as the live Wi-Fi entry, so never auto-bind them.
        let devices_out = "List of devices attached\n\
R58M12ABCDE\toffline\n\
192.168.1.42:5555\tdevice product:beyond1lte model:SM_G973F device:beyond1\n";
        let adb = MockAdb::new(devices_out).with_props(
            "192.168.1.42:5555",
            &props("R58M12ABCDE", "SM_G973F", "samsung"),
        );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("offline-merge"));
        let devices = mgr.collect(false).await.unwrap();
        assert_eq!(devices.len(), 2, "unverifiable identity must not merge");
        let sel = mgr.resolve("auto").await.unwrap();
        assert_eq!(
            sel.command_transport,
            Transport::Wifi,
            "only live entry wins"
        );
    }

    #[tokio::test]
    async fn saved_endpoint_is_reconnected_automatically() {
        let state = tmp_state("reconnect");
        let _ = std::fs::remove_file(&state);
        {
            let adb = MockAdb::new("List of devices attached\n");
            let mgr = DeviceManager::with_state_path(adb, state.clone());
            mgr.remember_endpoint("192.168.1.42:5555").unwrap();
        }
        // Endpoint saved but no longer in `adb devices` → connect is retried.
        let adb = MockAdb::new("List of devices attached\n");
        let mgr = DeviceManager::with_state_path(adb, state.clone());
        mgr.collect(true).await.unwrap();
        let calls = mgr.backend().calls();
        assert!(
            calls
                .iter()
                .any(|c| c.first().map(String::as_str) == Some("connect")),
            "expected an automatic reconnect, got {calls:?}"
        );
        let _ = std::fs::remove_file(&state);
    }

    #[tokio::test]
    async fn connect_persists_and_disconnect_forgets_endpoint() {
        let state = tmp_state("persist");
        let _ = std::fs::remove_file(&state);
        let mut adb = MockAdb::new("List of devices attached\n");
        adb.connect_reply = "connected to 192.168.1.42:5555".into();
        let mgr = DeviceManager::with_state_path(adb, state.clone());
        mgr.connect("192.168.1.42:5555").await.unwrap();
        assert!(mgr
            .load_state()
            .wifi_endpoints
            .contains(&"192.168.1.42:5555".to_string()));
        let done = mgr.disconnect("192.168.1.42:5555").await.unwrap();
        assert_eq!(done, vec!["192.168.1.42:5555".to_string()]);
        assert!(mgr.load_state().wifi_endpoints.is_empty());
        let _ = std::fs::remove_file(&state);
    }

    #[tokio::test]
    async fn connect_failure_is_typed() {
        let state = tmp_state("connfail");
        let adb = MockAdb::new("List of devices attached\n"); // default: failed reply
        let mgr = DeviceManager::with_state_path(adb, state);
        let err = mgr.connect("192.168.1.42:5555").await.unwrap_err();
        assert_eq!(err.code(), "E_WIFI_CONNECT_FAILED");
    }

    #[tokio::test]
    async fn invalid_endpoint_is_rejected_before_spawning_adb() {
        let state = tmp_state("badendpoint");
        let adb = MockAdb::new("");
        let mgr = DeviceManager::with_state_path(adb, state);
        assert!(mgr.connect("not-an-endpoint").await.is_err());
        assert!(mgr.pair("192.168.1.42", Some("123456")).await.is_err());
        assert!(mgr.backend().calls().is_empty(), "adb must not be spawned");
    }

    #[tokio::test]
    async fn pair_requires_code_reply_success() {
        let state = tmp_state("pair");
        let adb = MockAdb::new("");
        let mgr = DeviceManager::with_state_path(adb, state);
        assert!(mgr.pair("192.168.1.42:5555", Some("123456")).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mutations_on_same_logical_device_never_overlap() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mgr = Arc::new(DeviceManager::with_state_path(
            MockAdb::new(""),
            tmp_state("lock-same"),
        ));
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..3 {
            let mgr = Arc::clone(&mgr);
            let active = Arc::clone(&active);
            let max = Arc::clone(&max);
            handles.push(tokio::spawn(async move {
                let _guard = mgr.lock_device("PHONE").await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            max.load(Ordering::SeqCst),
            1,
            "same logical device = serialized"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mutations_on_different_devices_run_in_parallel() {
        let mgr = Arc::new(DeviceManager::with_state_path(
            MockAdb::new(""),
            tmp_state("lock-diff"),
        ));
        let (tx1, rx1) = tokio::sync::oneshot::channel::<()>();
        let (tx2, rx2) = tokio::sync::oneshot::channel::<()>();
        let m1 = Arc::clone(&mgr);
        let h1 = tokio::spawn(async move {
            let _g = m1.lock_device("PHONE_A").await;
            let _ = tx1.send(());
            let _ = rx2.await;
        });
        let m2 = Arc::clone(&mgr);
        let h2 = tokio::spawn(async move {
            let _g = m2.lock_device("PHONE_B").await;
            let _ = tx2.send(());
            let _ = rx1.await;
        });
        let res = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            h1.await.unwrap();
            h2.await.unwrap();
        })
        .await;
        assert!(
            res.is_ok(),
            "different logical devices must not block each other"
        );
    }

    #[tokio::test]
    async fn unknown_selector_is_not_found() {
        let adb = MockAdb::new(TWO_TRANSPORTS)
            .with_props("R58M12ABCDE", &props("R58M12ABCDE", "SM_G973F", "samsung"))
            .with_props(
                "192.168.1.42:5555",
                &props("R58M12ABCDE", "SM_G973F", "samsung"),
            );
        let mgr = DeviceManager::with_state_path(adb, tmp_state("notfound"));
        let err = mgr.resolve("GHOST").await.unwrap_err();
        assert_eq!(err.code(), "E_DEVICE_NOT_FOUND");
    }

    #[test]
    fn transport_ranks_follow_spec() {
        assert!(Transport::Usb.rank() < Transport::Wifi.rank());
        assert!(Transport::Wifi.rank() < Transport::Emulator.rank());
    }

    #[test]
    fn grouping_is_deterministic() {
        let mk = |serial: &str| DeviceEntry {
            serial: serial.to_string(),
            state: DeviceState::Device,
            transport: TransportHint::from_serial(serial),
            extras: BTreeMap::new(),
        };
        let a = group_logical_devices(vec![mk("B"), mk("A")], &HashMap::new());
        let b = group_logical_devices(vec![mk("A"), mk("B")], &HashMap::new());
        let ids = |v: &[LogicalDevice]| v.iter().map(|d| d.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
    }
}
