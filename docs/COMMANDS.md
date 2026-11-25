# Commands

Global flags (accepted by every command):

| Flag | Default | Effect |
|---|---|---|
| `--json` | off | print exactly one JSON envelope on stdout |
| `--device <selector>` | `auto` | `auto`, a logical id, a hardware serial, or an adb serial (pins that transport) |
| `--adb <path>` | discovery | explicit adb executable (overrides `ACA_ADB_PATH`) |
| `-y, --yes` | off | pre-authorize destructive confirmations |
| `-v, --verbose` | off | debug diagnostics on stderr |

Exit codes: `0` success · `1` runtime error · `2` usage · `3` confirmation declined.

---

## devices list

```powershell
android-cli devices list --json
```

Lists every detected phone as a **logical device** (USB + Wi-Fi appearances merged).
Data: array of `{id, state, best_transport, best_serial, model, android_version, ip,
identity_verified, identity_ambiguous, entries[], props}`.

## device info

```powershell
android-cli device info --device auto --json
```

Resolves the selector (auto-reconnects saved Wi-Fi endpoints first) and reports the
logical device plus the `command_serial`/`command_transport` that commands will use.

Errors: `E_NO_DEVICE`, `E_DEVICE_NOT_FOUND`, `E_DEVICE_AMBIGUOUS`, `E_DEVICE_UNAUTHORIZED`,
`E_DEVICE_OFFLINE`.

## transport status

```powershell
android-cli transport status --json
```

Rows: one per adb transport entry `{device, serial, transport, state}` plus
`saved_wifi_endpoints` (the auto-reconnect list).

## transport pair (Android 11+)

```powershell
android-cli transport pair --wifi IP:PORT --code 123456 --json
```

Pairs using the on-screen pairing code. Errors: `E_WIFI_PAIR_FAILED`, `E_INVALID_ARG`.

## transport connect

```powershell
android-cli transport connect --wifi IP:PORT --json
```

Connects and **persists** the endpoint for automatic reconnection.
Errors: `E_WIFI_CONNECT_FAILED` (invalid endpoints are rejected before adb is spawned).

## transport disconnect

```powershell
android-cli transport disconnect --device IP:PORT --json
android-cli transport disconnect --device auto --json     # all Wi-Fi entries of that phone
```

Forgets the endpoint from the saved list as well.

## apps list

```powershell
android-cli apps list --device auto --json
```

Array of `{package, apk_path, system}` sorted by package id (two `pm list` calls).

## apps info

```powershell
android-cli apps info --package com.example.app --json
```

`{package, version_name, version_code, min_sdk, target_sdk, first_install_time,
last_update_time, apk_path, system, pid}`. Error: `E_APP_NOT_FOUND`.

## apps launch / stop

```powershell
android-cli apps launch --package com.example.app --json
android-cli apps stop --package com.example.app --json
```

`launch` tries `monkey` then falls back to `cmd package resolve-activity` + `am start`.
Mutations hold the per-device lock.

## apps install

```powershell
android-cli apps install --apk ./app.apk --json
```

`adb install -r` (replace). Error: `E_APK_NOT_FOUND` (checked before adb), `E_APP_OP_FAILED`.

## apps uninstall · apps clear-data — DESTRUCTIVE

```powershell
android-cli apps uninstall --package com.example.app            # interactive [y/N]
android-cli apps uninstall --package com.example.app --json     # prompts on stderr
android-cli apps uninstall --package com.example.app --yes      # pre-authorized
```

The confirmation gate runs **before any adb process is spawned**. Declining exits `3`
with `E_CONFIRM_DECLINED`. See `docs/SECURITY.md`.

## ui screenshot

```powershell
android-cli ui screenshot [OUT.png] --json
```

`exec-out screencap -p`, PNG magic verified. Default: `screenshot-<epoch>.png`.

## ui dump

```powershell
android-cli ui dump --json
```

UI hierarchy via `exec-out uiautomator dump /dev/tty`, with a file fallback
(`uiautomator dump /sdcard/...` + `cat`) for devices that do not stream to stdout.

## ui find

```powershell
android-cli ui find --clickable true --json
android-cli ui find --text "Se connecter" --json
android-cli ui find --resource-id com.example.app:id/email --json
```

Selector flags (AND semantics): `--text`, `--text-contains`, `--resource-id`, `--class`,
`--package`, `--content-desc`, `--clickable true|false`. At least one is required
(`E_INVALID_ARG`, exit 2). Error `E_UI_NO_MATCH` when nothing matches (tap path).

## ui tap

```powershell
android-cli ui tap --x 540 --y 1860 --json        # absolute pixels
android-cli ui tap --text "Se connecter" --json   # center of the first match
```

## ui swipe / ui text / ui key

```powershell
android-cli ui swipe --x1 500 --y1 1500 --x2 500 --y2 500 --duration 400 --json
android-cli ui text "hello world" --json          # spaces ok, newline rejected
android-cli ui key KEYCODE_BACK --json            # KEYCODE_* or numeric code
```

`ui text` rejects newlines (`E_INVALID_ARG`) — use `ui key ENTER`.

## appium status

```powershell
android-cli appium status --json
```

Optional by design: reports host CLI detection (`ACA_APPIUM_PATH`, PATH) and whether the
UiAutomator2 server packages are on the device. Never fails the workflow; classic
operations do not depend on it.

## doctor

```powershell
android-cli doctor --json
```

Checks: `adb` discovery, `adb_version`, `device detection`, `state_dir` writability,
`appium_optional` (informational). `data.healthy` is `true` only when all required checks
pass; the process exits `1` when unhealthy (the report is still printed with `ok:true`).

## Environment variables

| Variable | Purpose |
|---|---|
| `ACA_ADB_PATH` | explicit adb executable |
| `ACA_APPIUM_PATH` | explicit Appium executable |
| `ANDROID_CLI_AGENT_HOME` | state directory (default `~/.android-cli-agent`) |
| `RUST_LOG` | tracing filter (e.g. `android_cli=debug`) |
