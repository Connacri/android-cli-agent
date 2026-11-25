//! End-to-end tests: real `android-cli` binary driven against the bundled
//! `fake-adb` fixture (no physical phone, no real adb).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static UNIQUE: AtomicUsize = AtomicUsize::new(0);

/// Fresh isolated state home per invocation (no cross-test pollution).
fn fresh_home() -> PathBuf {
    let n = UNIQUE.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("aca-cli-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_in(home: &Path, extra_env: Option<(&str, &str)>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_android-cli"));
    cmd.env("ACA_ADB_PATH", env!("CARGO_BIN_EXE_fake-adb"));
    cmd.env("ANDROID_CLI_AGENT_HOME", home);
    if let Some((k, v)) = extra_env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null()); // no terminal: interactive confirmations decline
    cmd.args(args);
    cmd.output().expect("spawn android-cli")
}

fn run(args: &[&str]) -> Output {
    run_in(&fresh_home(), None, args)
}

fn json_of(out: &Output) -> (serde_json::Value, i32) {
    let code = out.status.code().unwrap_or(-1);
    let text = String::from_utf8_lossy(&out.stdout);
    let value = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "invalid JSON (exit {code}): {e}\nstdout: {text}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (value, code)
}

// ------------------------------------------------------------------ devices

#[test]
fn devices_list_merges_usb_and_wifi_of_one_phone() {
    let out = run(&["devices", "list", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    let data = v["data"].as_array().expect("data must be an array");
    assert_eq!(data.len(), 1, "one phone on two transports = one device");
    assert_eq!(data[0]["id"], "HWTEST1");
    assert_eq!(data[0]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(data[0]["best_transport"], "usb", "USB has priority");
    assert_eq!(data[0]["identity_verified"], true);
}

#[test]
fn devices_list_text_mode_prints_table() {
    let out = run(&["devices", "list"]);
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(code, 0);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("HWTEST1"), "stdout: {text}");
    assert!(text.contains("TRANSPORT"));
}

#[test]
fn device_info_auto_resolves_the_only_phone() {
    let out = run(&["device", "info", "--device", "auto", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["id"], "HWTEST1");
    assert_eq!(v["data"]["command_serial"], "HWTEST1");
    assert_eq!(v["data"]["command_transport"], "usb");
}

#[test]
fn device_info_unknown_selector_fails_with_stable_code() {
    let out = run(&["device", "info", "--device", "GHOST", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_DEVICE_NOT_FOUND");
}

// --------------------------------------------------------------- transport

#[test]
fn transport_status_reports_rows_and_saved_endpoints() {
    let out = run(&["transport", "status", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    let rows = v["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "usb + wifi entries");
    assert!(v["data"]["saved_wifi_endpoints"].is_array());
}

#[test]
fn transport_connect_saves_endpoint_for_auto_reconnect() {
    let home = fresh_home();
    let out = run_in(
        &home,
        None,
        &[
            "transport",
            "connect",
            "--wifi",
            "192.168.1.55:5555",
            "--json",
        ],
    );
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);

    let out = run_in(&home, None, &["transport", "status", "--json"]);
    let (v, _) = json_of(&out);
    let saved = v["data"]["saved_wifi_endpoints"].as_array().unwrap();
    assert!(
        saved.iter().any(|e| e == "192.168.1.55:5555"),
        "endpoint must be persisted: {saved:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn transport_disconnect_takes_ip_port_target() {
    let out = run(&[
        "transport",
        "disconnect",
        "--device",
        "192.168.1.42:5555",
        "--json",
    ]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["disconnected"][0], "192.168.1.42:5555");
}

#[test]
fn transport_connect_rejects_invalid_endpoint() {
    let out = run(&[
        "transport",
        "connect",
        "--wifi",
        "not-an-endpoint",
        "--json",
    ]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "E_WIFI_CONNECT_FAILED");
}

// -------------------------------------------------------------------- apps

#[test]
fn apps_list_returns_empty_inventory() {
    let out = run(&["apps", "list", "--device", "auto", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(
        v["data"].as_array().unwrap().len(),
        0,
        "empty state is valid"
    );
}

#[test]
fn apps_info_missing_package_is_typed_error() {
    let out = run(&["apps", "info", "--package", "com.ghost", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_APP_NOT_FOUND");
}

#[test]
fn apps_list_on_unknown_device_fails() {
    let out = run(&["apps", "list", "--device", "GHOST", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "E_DEVICE_NOT_FOUND");
}

#[test]
fn destructive_uninstall_declines_without_yes() {
    // stdin is null → EOF → the human never confirms → exit 3, no adb mutation
    let out = run(&[
        "apps",
        "uninstall",
        "--package",
        "com.example.app",
        "--json",
    ]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 3, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "E_CONFIRM_DECLINED");
}

#[test]
fn destructive_uninstall_runs_with_yes() {
    let out = run(&[
        "apps",
        "uninstall",
        "--package",
        "com.example.app",
        "--yes",
        "--json",
    ]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["action"], "uninstall");
    assert_eq!(v["data"]["detail"], "Success");
}

#[test]
fn device_failure_reply_becomes_typed_error() {
    // Fixture directory with an explicit Failure reply for uninstall.
    let dir = fresh_home();
    std::fs::write(
        dir.join("replies.json"),
        r#"{"uninstall com.example.app": {"stdout": "Failure [DELETE_FAILED_INTERNAL_ERROR]\n"}}"#,
    )
    .unwrap();
    let out = run_in(
        &dir,
        Some(("ACA_FAKE_DIR", dir.to_str().unwrap())),
        &[
            "apps",
            "uninstall",
            "--package",
            "com.example.app",
            "--yes",
            "--json",
        ],
    );
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "E_APP_OP_FAILED");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("DELETE_FAILED_INTERNAL_ERROR"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------- ui

#[test]
fn ui_tap_by_selector_hits_node_center() {
    let out = run(&["ui", "tap", "--text", "OK", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["action"], "tap");
    assert_eq!(v["data"]["detail"], "540,1860");
}

#[test]
fn ui_tap_without_target_is_usage_error() {
    let out = run(&["ui", "tap", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "E_INVALID_ARG");
}

#[test]
fn ui_find_requires_a_selector() {
    let out = run(&["ui", "find", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "E_INVALID_ARG");
}

#[test]
fn ui_find_returns_matching_nodes() {
    let out = run(&["ui", "find", "--clickable", "true", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let found = v["data"].as_array().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["text"], "OK");
}

#[test]
fn ui_screenshot_writes_png_file() {
    let dest = std::env::temp_dir().join(format!("aca-shot-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let out = run(&["ui", "screenshot", dest.to_str().unwrap(), "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    let bytes = std::fs::read(&dest).expect("screenshot file must exist");
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    let _ = std::fs::remove_file(&dest);
}

#[test]
fn ui_text_rejects_newline_before_adb() {
    let out = run(&["ui", "text", "line1\nline2", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "E_INVALID_ARG");
}

// ----------------------------------------------------------------- appium

#[test]
fn appium_status_is_optional_and_never_fails() {
    let out = run(&["appium", "status", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert!(v["data"]["cli_available"].is_boolean());
    assert!(v["data"]["hint"].is_string());
}

// ----------------------------------------------------------------- doctor

#[test]
fn doctor_reports_healthy_setup() {
    let out = run(&["doctor", "--json"]);
    let (v, code) = json_of(&out);
    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["healthy"], true);
    let checks = v["data"]["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["name"] == "adb" && c["ok"] == true));
    assert!(checks
        .iter()
        .any(|c| c["name"] == "appium_optional" && c["ok"] == true));
}

// ------------------------------------------------------------------ usage

#[test]
fn missing_subcommand_exits_with_usage_code() {
    let out = run(&["--json"]);
    assert_eq!(out.status.code().unwrap_or(-1), 2, "clap usage error");
}

#[test]
fn missing_adb_reports_stable_code() {
    // Point discovery at a PATH without adb and no explicit flag.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_android-cli"));
    cmd.env_remove("ACA_ADB_PATH");
    cmd.env("PATH", "");
    cmd.env("ANDROID_HOME", "");
    cmd.env("ANDROID_SDK_ROOT", "");
    cmd.env("ANDROID_CLI_AGENT_HOME", fresh_home());
    cmd.stdin(Stdio::null());
    cmd.args(["devices", "list", "--json"]);
    let out = cmd.output().unwrap();
    let (v, code) = json_of(&out);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "E_ADB_NOT_FOUND");
}
