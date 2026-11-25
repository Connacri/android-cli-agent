//! Command implementations. Every function returns [`Success`] (JSON payload
//! + human text + exit code) or [`CliError`] (stable code + exit code).
//!
//! Mutating operations (apps lifecycle, ui input) hold the per-logical-device
//! lock from `device-manager` so two mutations never overlap on one phone.

use crate::context::{CliError, Ctx, Success};
use crate::UiFilterArgs;
use adb_engine::AdbBackend;
use app_manager::AppManager;
use device_manager::DeviceManager;
use serde::Serialize;
use std::path::Path;
use ui_engine::{NodeFilter, TapTarget};

// ---------------------------------------------------------------- helpers

/// Resolve the target device for this invocation (selector + auto reconnect).
async fn resolve(
    ctx: &Ctx,
    mgr: &DeviceManager<adb_engine::Adb>,
) -> Result<device_manager::SelectedDevice, CliError> {
    Ok(mgr.resolve(&ctx.selector).await?)
}

fn to_success<T: Serialize>(data: &T, text: String) -> Result<Success, CliError> {
    Success::new(data, text)
}

// ---------------------------------------------------------------- devices

pub async fn devices_list(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let devices = mgr.collect(true).await?;
    let mut text = format!(
        "{:<24} {:<12} {:<9} {:<16} {:<8} {}\n",
        "ID", "STATE", "TRANSPORT", "MODEL", "ANDROID", "IP"
    );
    for d in &devices {
        text.push_str(&format!(
            "{:<24} {:<12} {:<9} {:<16} {:<8} {}\n",
            d.id,
            d.state,
            d.best_transport.map(|t| t.as_str()).unwrap_or("-"),
            d.model.as_deref().unwrap_or("-"),
            d.android_version.as_deref().unwrap_or("-"),
            d.ip.as_deref().unwrap_or("-"),
        ));
        if d.identity_ambiguous {
            text.push_str("    ! identity ambiguous: pass --device <serial> to disambiguate\n");
        }
    }
    if devices.is_empty() {
        text.push_str("no devices detected (is a phone connected and authorized?)\n");
    }
    to_success(&devices, text.trim_end().to_string())
}

pub async fn device_info(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let d = &sel.device;
    let mut text = String::new();
    text.push_str(&format!("id: {}\n", d.id));
    text.push_str(&format!("state: {}\n", d.state));
    text.push_str(&format!(
        "best_transport: {}\n",
        d.best_transport.map(|t| t.as_str()).unwrap_or("-")
    ));
    text.push_str(&format!("command_serial: {}\n", sel.command_serial));
    text.push_str(&format!(
        "hardware_serial: {}\n",
        d.hardware_serial.as_deref().unwrap_or("-")
    ));
    text.push_str(&format!("model: {}\n", d.model.as_deref().unwrap_or("-")));
    text.push_str(&format!(
        "manufacturer: {}\n",
        d.manufacturer.as_deref().unwrap_or("-")
    ));
    text.push_str(&format!(
        "android_version: {}\n",
        d.android_version.as_deref().unwrap_or("-")
    ));
    text.push_str(&format!(
        "fingerprint: {}\n",
        d.fingerprint.as_deref().unwrap_or("-")
    ));
    text.push_str(&format!(
        "identity_verified: {} identity_ambiguous: {}\n",
        d.identity_verified, d.identity_ambiguous
    ));
    text.push_str("transports:\n");
    for e in &d.entries {
        text.push_str(&format!(
            "  - {} {} ({})\n",
            e.transport.as_str(),
            e.serial,
            e.state
        ));
    }
    to_success(&sel, text.trim_end().to_string())
}

// ------------------------------------------------------------- transport

pub async fn transport_status(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let devices = mgr.collect(false).await?;
    let saved = mgr.load_state();

    #[derive(Serialize)]
    struct TransportRow {
        device: String,
        serial: String,
        transport: String,
        state: String,
    }
    let mut rows = Vec::new();
    for d in &devices {
        for e in &d.entries {
            rows.push(TransportRow {
                device: d.id.clone(),
                serial: e.serial.clone(),
                transport: e.transport.as_str().to_string(),
                state: e.state.clone(),
            });
        }
    }

    #[derive(Serialize)]
    struct Report {
        rows: Vec<TransportRow>,
        saved_wifi_endpoints: Vec<String>,
    }
    let report = Report {
        rows,
        saved_wifi_endpoints: saved.wifi_endpoints,
    };

    let mut text = String::new();
    text.push_str(&format!(
        "{:<24} {:<24} {:<9} {}\n",
        "DEVICE", "SERIAL", "TRANSPORT", "STATE"
    ));
    for r in &report.rows {
        text.push_str(&format!(
            "{:<24} {:<24} {:<9} {}\n",
            r.device, r.serial, r.transport, r.state
        ));
    }
    if report.rows.is_empty() {
        text.push_str("no transports active\n");
    }
    text.push_str("saved wifi endpoints:\n");
    if report.saved_wifi_endpoints.is_empty() {
        text.push_str("  (none)\n");
    }
    for ep in &report.saved_wifi_endpoints {
        text.push_str(&format!("  - {ep}\n"));
    }
    to_success(&report, text.trim_end().to_string())
}

pub async fn transport_pair(
    ctx: &Ctx,
    wifi: &str,
    code: Option<&str>,
) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    mgr.pair(wifi, code).await?;
    #[derive(Serialize)]
    struct Res {
        action: &'static str,
        endpoint: String,
        paired: bool,
    }
    let res = Res {
        action: "pair",
        endpoint: wifi.to_string(),
        paired: true,
    };
    to_success(&res, format!("paired with {wifi}"))
}

pub async fn transport_connect(ctx: &Ctx, wifi: &str) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    mgr.connect(wifi).await?;
    #[derive(Serialize)]
    struct Res {
        action: &'static str,
        endpoint: String,
        connected: bool,
    }
    let res = Res {
        action: "connect",
        endpoint: wifi.to_string(),
        connected: true,
    };
    to_success(
        &res,
        format!("connected to {wifi} (saved for auto-reconnect)"),
    )
}

pub async fn transport_disconnect(ctx: &Ctx, device: Option<&str>) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let target = device.unwrap_or(&ctx.selector);
    let done = mgr.disconnect(target).await?;
    #[derive(Serialize)]
    struct Res {
        action: &'static str,
        disconnected: Vec<String>,
    }
    let res = Res {
        action: "disconnect",
        disconnected: done.clone(),
    };
    to_success(&res, format!("disconnected: {}", done.join(", ")))
}

// ------------------------------------------------------------------ apps

pub async fn apps_list(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let apps = AppManager::new(mgr.backend().clone());
    let list = apps.list(&sel.command_serial).await?;
    let mut text = format!("{:<6} {}\n", "TYPE", "PACKAGE");
    for a in &list {
        text.push_str(&format!(
            "{:<6} {}\n",
            if a.system { "sys" } else { "user" },
            a.package
        ));
    }
    if list.is_empty() {
        text.push_str("no packages found\n");
    }
    to_success(&list, text.trim_end().to_string())
}

pub async fn apps_info(ctx: &Ctx, package: &str) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let apps = AppManager::new(mgr.backend().clone());
    let info = apps.info(&sel.command_serial, package).await?;
    let text = format!(
        "package: {}\nversion_name: {}\nversion_code: {}\nmin_sdk: {}\ntarget_sdk: {}\n\
         system: {}\npid: {}\napk_path: {}\nfirst_install_time: {}\nlast_update_time: {}\n",
        info.package,
        info.version_name.as_deref().unwrap_or("-"),
        info.version_code.as_deref().unwrap_or("-"),
        info.min_sdk.as_deref().unwrap_or("-"),
        info.target_sdk.as_deref().unwrap_or("-"),
        info.system,
        info.pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into()),
        info.apk_path.as_deref().unwrap_or("-"),
        info.first_install_time.as_deref().unwrap_or("-"),
        info.last_update_time.as_deref().unwrap_or("-"),
    );
    to_success(&info, text.trim_end().to_string())
}

async fn mutate<F, Fut>(ctx: &Ctx, action: &str, run: F) -> Result<Success, CliError>
where
    F: FnOnce(String, AppManager<adb_engine::Adb>) -> Fut,
    Fut: std::future::Future<Output = Result<app_manager::AppOpResult, app_manager::AppError>>,
{
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let _guard = mgr.lock_device(&sel.device.id).await;
    let apps = AppManager::new(mgr.backend().clone());
    let res = run(sel.command_serial.clone(), apps).await?;
    tracing::debug!(action, package = %res.package, "mutation done");
    to_success(
        &res,
        format!("{}: {} -> {}", res.action, res.package, res.detail),
    )
}

pub async fn apps_launch(ctx: &Ctx, package: &str) -> Result<Success, CliError> {
    let pkg = package.to_string();
    mutate(ctx, "launch", |serial, apps| async move {
        apps.launch(&serial, &pkg).await
    })
    .await
}

pub async fn apps_stop(ctx: &Ctx, package: &str) -> Result<Success, CliError> {
    let pkg = package.to_string();
    mutate(ctx, "stop", |serial, apps| async move {
        apps.stop(&serial, &pkg).await
    })
    .await
}

pub async fn apps_install(ctx: &Ctx, apk: &Path) -> Result<Success, CliError> {
    let apk = apk.to_path_buf();
    mutate(ctx, "install", |serial, apps| async move {
        apps.install(&serial, &apk).await
    })
    .await
}

pub async fn apps_uninstall(ctx: &Ctx, package: &str) -> Result<Success, CliError> {
    let pkg = package.to_string();
    let confirmation = ctx.confirmation();
    mutate(ctx, "uninstall", move |serial, apps| async move {
        apps.uninstall(&serial, &pkg, confirmation).await
    })
    .await
}

pub async fn apps_clear_data(ctx: &Ctx, package: &str) -> Result<Success, CliError> {
    let pkg = package.to_string();
    let confirmation = ctx.confirmation();
    mutate(ctx, "clear-data", move |serial, apps| async move {
        apps.clear_data(&serial, &pkg, confirmation).await
    })
    .await
}

// -------------------------------------------------------------------- ui

fn filter_of(args: &UiFilterArgs) -> NodeFilter {
    NodeFilter {
        text: args.text.clone(),
        text_contains: args.text_contains.clone(),
        resource_id: args.resource_id.clone(),
        class: args.class.clone(),
        package: args.package.clone(),
        content_desc: args.content_desc.clone(),
        clickable: args.clickable,
    }
}

fn filter_is_empty(f: &NodeFilter) -> bool {
    f.text.is_none()
        && f.text_contains.is_none()
        && f.resource_id.is_none()
        && f.class.is_none()
        && f.package.is_none()
        && f.content_desc.is_none()
        && f.clickable.is_none()
}

pub async fn ui_screenshot(ctx: &Ctx, out: Option<&Path>) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let res = ui.screenshot(&sel.command_serial, out).await?;
    to_success(&res, format!("saved {} ({} bytes)", res.path, res.bytes))
}

pub async fn ui_dump(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let h = ui.dump(&sel.command_serial).await?;
    let mut text = format!("{} nodes\n", h.node_count);
    for n in h.nodes.iter().take(50) {
        text.push_str(&format!(
            "  [{:?}] class={} text={:?} desc={:?} clickable={} bounds=[{},{},{},{}]\n",
            n.depth,
            n.class,
            n.text,
            n.content_desc,
            n.clickable,
            n.bounds.x,
            n.bounds.y,
            n.bounds.width,
            n.bounds.height
        ));
    }
    if h.node_count > 50 {
        text.push_str(&format!("  ... {} more nodes\n", h.node_count - 50));
    }
    to_success(&h, text.trim_end().to_string())
}

pub async fn ui_find(ctx: &Ctx, args: &UiFilterArgs) -> Result<Success, CliError> {
    let filter = filter_of(args);
    if filter_is_empty(&filter) {
        return Err(CliError::new(
            "E_INVALID_ARG",
            "ui find requires at least one selector flag (--text, --resource-id, ...)",
            output::exit::USAGE,
        ));
    }
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let found = ui.find(&sel.command_serial, &filter).await?;
    let mut text = String::new();
    for (i, n) in found.iter().enumerate() {
        text.push_str(&format!(
            "#{} class={} text={:?} desc={:?} clickable={} bounds=[{},{},{},{}]\n",
            i,
            n.class,
            n.text,
            n.content_desc,
            n.clickable,
            n.bounds.x,
            n.bounds.y,
            n.bounds.width,
            n.bounds.height
        ));
    }
    if found.is_empty() {
        text.push_str("no matching node\n");
    }
    to_success(&found, text.trim_end().to_string())
}

pub async fn ui_tap(
    ctx: &Ctx,
    x: Option<i32>,
    y: Option<i32>,
    args: &UiFilterArgs,
) -> Result<Success, CliError> {
    let target = match (x, y) {
        (Some(x), Some(y)) => TapTarget::Point { x, y },
        (None, None) => {
            let filter = filter_of(args);
            if filter_is_empty(&filter) {
                return Err(CliError::new(
                    "E_INVALID_ARG",
                    "ui tap requires --x/--y or at least one selector flag",
                    output::exit::USAGE,
                ));
            }
            TapTarget::Selector(filter)
        }
        _ => {
            return Err(CliError::new(
                "E_INVALID_ARG",
                "ui tap requires both --x and --y",
                output::exit::USAGE,
            ));
        }
    };
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let _guard = mgr.lock_device(&sel.device.id).await;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let res = ui.tap(&sel.command_serial, target).await?;
    to_success(&res, format!("{} at {}", res.action, res.detail))
}

pub async fn ui_swipe(
    ctx: &Ctx,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    duration: Option<u32>,
) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let _guard = mgr.lock_device(&sel.device.id).await;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let res = ui
        .swipe(&sel.command_serial, x1, y1, x2, y2, duration)
        .await?;
    to_success(&res, format!("{}: {}", res.action, res.detail))
}

pub async fn ui_text(ctx: &Ctx, text_in: &str) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let _guard = mgr.lock_device(&sel.device.id).await;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let res = ui.text(&sel.command_serial, text_in).await?;
    to_success(&res, format!("typed {} characters", res.detail))
}

pub async fn ui_key(ctx: &Ctx, keycode: &str) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    let sel = resolve(ctx, &mgr).await?;
    let _guard = mgr.lock_device(&sel.device.id).await;
    let ui = ui_engine::UiEngine::new(mgr.backend().clone());
    let res = ui.key(&sel.command_serial, keycode).await?;
    to_success(&res, format!("sent {}", res.detail))
}

// ---------------------------------------------------------------- appium

pub async fn appium_status(ctx: &Ctx) -> Result<Success, CliError> {
    let mgr = ctx.devices()?;
    // Best effort: device serial may be unavailable; host detection still works.
    let serial = resolve(ctx, &mgr).await.ok().map(|s| s.command_serial);
    let engine = appium_engine::AppiumEngine::new(mgr.backend().clone());
    let status = engine.status(serial.as_deref()).await;
    to_success(&status, status.hint.clone())
}

// ---------------------------------------------------------------- doctor

pub async fn doctor(ctx: &Ctx) -> Result<Success, CliError> {
    #[derive(Serialize)]
    struct Check {
        name: String,
        ok: bool,
        detail: String,
    }
    #[derive(Serialize)]
    struct Report {
        healthy: bool,
        checks: Vec<Check>,
    }

    let mut checks = Vec::new();

    // 1. adb discovery
    let adb_path = ctx.find_adb();
    let adb_ok = adb_path.is_ok();
    checks.push(Check {
        name: "adb".into(),
        ok: adb_ok,
        detail: match &adb_path {
            Ok(p) => p.display().to_string(),
            Err(e) => e.message.clone(),
        },
    });

    // 2. adb version
    if let Ok(path) = &adb_path {
        let adb = adb_engine::Adb::new(path.clone());
        match adb.run(&["version"]).await {
            Ok(out) if out.success() => {
                let version = adb_engine::parse_adb_version(&out.stdout_str())
                    .unwrap_or_else(|| "unknown".into());
                checks.push(Check {
                    name: "adb_version".into(),
                    ok: true,
                    detail: version,
                });
            }
            Ok(out) => checks.push(Check {
                name: "adb_version".into(),
                ok: false,
                detail: format!("exit {}: {}", out.status, out.error_text().trim()),
            }),
            Err(e) => checks.push(Check {
                name: "adb_version".into(),
                ok: false,
                detail: e.to_string(),
            }),
        }
    } else {
        checks.push(Check {
            name: "adb_version".into(),
            ok: false,
            detail: "skipped: adb not found".into(),
        });
    }

    // 3. device detection
    if adb_ok {
        let mgr = ctx.devices()?;
        match mgr.collect(false).await {
            Ok(devices) => {
                let live = devices
                    .iter()
                    .filter(|d| !d.live_serials().is_empty())
                    .count();
                checks.push(Check {
                    name: "devices".into(),
                    ok: true,
                    detail: format!("{} logical device(s), {} live", devices.len(), live),
                });
            }
            Err(e) => checks.push(Check {
                name: "devices".into(),
                ok: false,
                detail: e.to_string(),
            }),
        }
    } else {
        checks.push(Check {
            name: "devices".into(),
            ok: false,
            detail: "skipped: adb not found".into(),
        });
    }

    // 4. state directory writable
    let state_dir = device_manager::default_state_dir();
    let probe = state_dir.join(".doctor-probe");
    let state_ok = std::fs::create_dir_all(&state_dir)
        .and_then(|()| std::fs::write(&probe, b"ok"))
        .map(|()| {
            let _ = std::fs::remove_file(&probe);
        })
        .is_ok();
    checks.push(Check {
        name: "state_dir".into(),
        ok: state_ok,
        detail: format!(
            "{} ({})",
            state_dir.display(),
            if state_ok { "writable" } else { "not writable" }
        ),
    });

    // 5. appium is optional: reported, never fatal
    let engine = appium_engine::AppiumEngine::new(ctx.backend()?);
    let appium = engine.status(None).await;
    checks.push(Check {
        name: "appium_optional".into(),
        ok: true,
        detail: if appium.cli_available {
            format!("detected at {}", appium.cli_path.unwrap_or_default())
        } else {
            "not installed (optional; classic operations unaffected)".into()
        },
    });

    let healthy = checks.iter().all(|c| c.ok);
    let report = Report { healthy, checks };

    let mut text = String::new();
    for c in &report.checks {
        text.push_str(&format!(
            "[{}] {}: {}\n",
            if c.ok { "OK" } else { "FAIL" },
            c.name,
            c.detail
        ));
    }
    text.push_str(if healthy {
        "\nhealthy: yes"
    } else {
        "\nhealthy: no"
    });

    let exit_code = if healthy {
        output::exit::OK
    } else {
        output::exit::ERROR
    };
    Success::with_exit(&report, text, exit_code)
}
