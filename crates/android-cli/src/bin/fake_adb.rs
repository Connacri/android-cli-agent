//! Fake `adb` used by `crates/android-cli/tests/cli.rs`.
//!
//! Behaviour:
//! 1. If `ACA_FAKE_DIR` points to a directory containing `replies.json`
//!    (a map of `"arg1 arg2 ..."` → `{status?, stdout?, stderr?}`), an exact
//!    key match wins.
//! 2. Otherwise built-in defaults cover the classic flows (version, devices
//!    -l with one phone on two transports, getprop, connect/disconnect/pair,
//!    pm/am/input/screencap/uiautomator).
//! 3. Anything else exits 0 with empty output.
//!
//! Never shipped: the release workflow builds `--bin android-cli` only.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Default, serde::Deserialize)]
struct Reply {
    #[serde(default)]
    status: Option<i32>,
    #[serde(default)]
    stdout: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
}

const VERSION_OUT: &str = "Android Debug Bridge version 1.0.41\n\
Version 35.0.2-12147458\n\
Installed as C:\\fake\\adb.exe\n";

const DEVICES_OUT: &str = "List of devices attached\n\
HWTEST1\tdevice usb:1-1 product:fake model:FakePhone device:fake transport_id:1\n\
192.168.1.42:5555\tdevice product:fake model:FakePhone device:fake transport_id:2\n";

const HIERARCHY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.fake" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" scrollable="false" bounds="[0,0][1080,1920]">
    <node index="0" text="OK" resource-id="com.fake:id/ok" class="android.widget.Button" package="com.fake" content-desc="ok button" checkable="false" checked="false" clickable="true" enabled="true" scrollable="false" bounds="[0,1800][1080,1920]" />
  </node>
</hierarchy>"#;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Strip the `-s <serial>` prefix so keys match the bare sub-command.
    let key = if args.len() >= 2 && args[0] == "-s" {
        args[2..].join(" ")
    } else {
        args.join(" ")
    };

    // 1. canned replies from the test fixture directory
    if let Some(dir) = std::env::var_os("ACA_FAKE_DIR").map(PathBuf::from) {
        if let Ok(text) = std::fs::read_to_string(dir.join("replies.json")) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, Reply>>(&text) {
                if let Some(reply) = map.get(&key) {
                    emit(
                        reply.status.unwrap_or(0),
                        reply.stdout.clone().unwrap_or_default().into_bytes(),
                        reply.stderr.clone().unwrap_or_default().into_bytes(),
                    );
                    return;
                }
            }
        }
    }

    // 2. built-in defaults
    let serial = args.get(1).cloned().unwrap_or_default();

    if key == "version" {
        emit(0, VERSION_OUT.as_bytes().to_vec(), Vec::new());
    } else if key == "devices -l" {
        emit(0, DEVICES_OUT.as_bytes().to_vec(), Vec::new());
    } else if args.len() >= 4 && args[0] == "-s" && args[2..] == ["shell", "getprop"] {
        emit(0, default_props(&serial).into_bytes(), Vec::new());
    } else if key == "exec-out screencap -p" {
        let mut png = vec![0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(b"fake-pixel-data");
        emit(0, png, Vec::new());
    } else if key == "exec-out uiautomator dump /dev/tty" {
        emit(0, HIERARCHY.as_bytes().to_vec(), Vec::new());
    } else if key.starts_with("shell uiautomator dump ") {
        emit(
            0,
            b"Dumped file: /sdcard/aca_window_dump.xml\n".to_vec(),
            Vec::new(),
        );
    } else if key.starts_with("exec-out cat ") {
        emit(0, HIERARCHY.as_bytes().to_vec(), Vec::new());
    } else if let Some(ep) = key.strip_prefix("connect ") {
        emit(0, format!("connected to {ep}\n").into_bytes(), Vec::new());
    } else if let Some(ep) = key.strip_prefix("disconnect ") {
        emit(0, format!("disconnected {ep}\n").into_bytes(), Vec::new());
    } else if key.starts_with("pair ") {
        emit(
            0,
            b"Successfully paired with 192.168.1.42:5555 [guid=0000]\n".to_vec(),
            Vec::new(),
        );
    } else if key.starts_with("shell pm clear ")
        || key.starts_with("uninstall ")
        || key.starts_with("install -r ")
    {
        emit(0, b"Success\n".to_vec(), Vec::new());
    } else {
        // pm list/path/dumpsys/pidof, am, monkey, input...: success, no output
        emit(0, Vec::new(), Vec::new());
    }
}

fn default_props(serial: &str) -> String {
    // The Wi-Fi entry of the default fixture reports the same hardware
    // serial as the USB entry, so both merge into one logical device.
    let hw = if serial.contains(':') {
        "HWTEST1"
    } else {
        serial
    };
    format!(
        "[ro.serialno]: [{hw}]\n\
         [ro.product.model]: [FakePhone]\n\
         [ro.product.manufacturer]: [acme]\n\
         [ro.product.brand]: [acme]\n\
         [ro.product.name]: [fake]\n\
         [ro.product.device]: [fake]\n\
         [ro.build.version.release]: [14]\n\
         [ro.build.version.sdk]: [34]\n\
         [ro.build.fingerprint]: [acme/fake/fake:14/UP1A/eng:user/release-keys]\n\
         [ro.product.cpu.abilist]: [arm64-v8a,armeabi-v7a]\n"
    )
}

fn emit(status: i32, stdout: Vec<u8>, stderr: Vec<u8>) {
    let mut out = std::io::stdout();
    let _ = out.write_all(&stdout);
    let _ = out.flush();
    let mut err = std::io::stderr();
    let _ = err.write_all(&stderr);
    let _ = err.flush();
    std::process::exit(status);
}
