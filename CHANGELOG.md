# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-11-25

### Added

- `adb-engine`: adb discovery (`--adb`, `ACA_ADB_PATH`, `ANDROID_HOME`,
  `ANDROID_SDK_ROOT`, `PATH`), 30 s timeouts, raw-byte output for `screencap`,
  parsers for `devices -l`, `getprop`, `adb version`, typed transport failures,
  `AdbBackend` trait for mock-based testing.
- `device-manager`: logical device identity (USB + Wi-Fi merged on hardware serial,
  property-fingerprint fallback flagged ambiguous, twins never auto-merged),
  transport priority USB > Wi-Fi > emulator, Wi-Fi pair/connect/disconnect, saved
  endpoints with automatic reconnection, flat JSON state file (atomic writes),
  per-logical-device mutation locks.
- `app-manager`: package inventory (`list`, `info`), lifecycle (`launch` with `am start`
  fallback, `stop`, `install`), destructive operations (`uninstall`, `clear-data`)
  gated by explicit human confirmation before any adb spawn.
- `ui-engine`: screenshot (`exec-out screencap -p`), UI hierarchy dump with file
  fallback, node search with AND selectors, tap by coordinates or selector, swipe,
  shell-quoted text input, validated key events.
- `appium-engine`: optional Appium CLI and UiAutomator2 server detection (never
  required for classic operations).
- `android-cli` CLI: `devices`, `device`, `transport`, `apps`, `ui`, `appium`,
  `doctor` command tree; global `--json`, `--device`, `--adb`, `--yes`, `--verbose`;
  uniform JSON envelope with stable error codes; exit codes 0/1/2/3.
- Test suite: 90+ unit and end-to-end tests (mocked adb backends and a `fake-adb`
  fixture binary), running on Ubuntu and Windows CI.
- Docs: ARCHITECTURE, COMMANDS, SECURITY, COMPATIBILITY, TROUBLESHOOTING; agent skill;
  OpenCode commands; PowerShell install/uninstall/doctor scripts.

[0.1.0]: https://github.com/Connacri/android-cli-agent/releases/tag/v0.1.0
