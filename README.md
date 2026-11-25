# android-cli-agent

**Agent-native CLI to inspect, control and automate Android phones from a Windows PC over ADB.**

`android-cli` is built for humans *and* AI agents: every command supports `--json` with a
uniform `{"ok":...}` envelope, stable machine-readable error codes and documented exit codes.
No cloud, no subscription, no proprietary Android app — just the official Android SDK
Platform-Tools and a single `.exe`.

- **Offline-first**: works fully locally; state lives in a flat JSON file.
- **Logical device identity**: one phone appearing over USB *and* Wi-Fi is one device.
- **Transport priority**: USB > Wi-Fi > offline, with saved-endpoint auto-reconnect.
- **Safe by default**: destructive operations (`apps uninstall`, `apps clear-data`) require
  explicit human confirmation before any adb process is spawned.
- **Optional Appium**: UiAutomator2 detection is reported, never required.

## Install

### From a GitHub release (end users)

```powershell
./scripts/install.ps1            # downloads the latest released android-cli.exe
./scripts/doctor.ps1             # verify the setup
```

### From source (developers)

```powershell
cargo build                      # debug build
./scripts/install.ps1 -FromBuild ./target/debug/android-cli.exe
```

Release artifacts are **only** produced by GitHub Actions (`.github/workflows/release.yml`),
never built or signed locally.

## Quick start

```powershell
android-cli devices list --json            # detected phones (logical identity)
android-cli device info --device auto      # details of the selected phone
android-cli transport status --json        # transports + saved Wi-Fi endpoints
android-cli apps list --device auto --json # installed packages
android-cli ui screenshot shot.png         # capture the screen
android-cli ui tap --text "OK" --json      # tap the first matching node
android-cli doctor                         # diagnose the local setup
```

Destructive operations ask for confirmation (or take `--yes`):

```powershell
android-cli apps uninstall --package com.example.app      # prompts [y/N]
android-cli apps uninstall --package com.example.app --yes # pre-authorized
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | runtime failure (device, app, ADB, IO…) |
| 2 | invalid usage (also emitted by clap) |
| 3 | human declined a destructive confirmation |

Error codes (`E_*`) are stable: `E_ADB_NOT_FOUND`, `E_DEVICE_NOT_FOUND`,
`E_DEVICE_AMBIGUOUS`, `E_DEVICE_UNAUTHORIZED`, `E_DEVICE_OFFLINE`, `E_APP_NOT_FOUND`,
`E_CONFIRM_DECLINED`, `E_UI_NO_MATCH`, …

## Documentation

| Document | Content |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | crates, data flow, locking, identity rules |
| [docs/COMMANDS.md](docs/COMMANDS.md) | full command reference |
| [docs/SECURITY.md](docs/SECURITY.md) | confirmation gates, threat model, secrets |
| [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) | Android 8+, USB/Wi-Fi, Windows, optional Appium |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | common failures and fixes |
| [skills/android-cli-agent/SKILL.md](skills/android-cli-agent/SKILL.md) | agent skill |
| [opencode/commands/](opencode/commands/) | OpenCode commands |

## Workspace layout

```
crates/
  android-cli/      CLI binary (clap), JSON contract, exit codes
  adb-engine/       thin wrapper around the official adb binary
  device-manager/   logical identity, transports, Wi-Fi, per-device locks
  app-manager/      package inventory + lifecycle, confirmation gates
  ui-engine/        screenshot, hierarchy dump, find/tap/swipe/text/key
  appium-engine/    optional Appium/UiAutomator2 detection
  security/         explicit human confirmation gates
  output/           uniform JSON envelope + exit codes
```

## Development

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks on Ubuntu and Windows. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
