# Troubleshooting

Run the built-in diagnostics first:

```powershell
android-cli doctor --json
```

## `E_ADB_NOT_FOUND`

adb was not discovered. Fixes:

1. Install **Android SDK Platform-Tools** (Android Studio → SDK Manager → SDK Tools).
2. Or set `ANDROID_HOME` / `ANDROID_SDK_ROOT` to your SDK directory.
3. Or pass the binary explicitly: `android-cli --adb "C:\Android\Sdk\platform-tools\adb.exe" …`
4. Or set `ACA_ADB_PATH` to the full path of `adb.exe`.

## `E_DEVICE_UNAUTHORIZED`

Unlock the phone and accept the **"Allow USB debugging?"** prompt (check *Always allow*).
Replug the cable if the prompt does not appear. On the phone: *Settings → Developer
options → Revoke USB debugging authorizations* then replug to force a fresh prompt.

## `E_DEVICE_OFFLINE`

The transport is dead. Try, in order:

```powershell
adb kill-server; adb start-server
android-cli transport status --json     # see which entry is offline
# USB: change cable/port (data cable, not charge-only)
# Wi-Fi: android-cli transport connect --wifi IP:PORT
```

## `E_DEVICE_AMBIGUOUS`

Several phones with the same model were detected without a readable hardware serial, so
`--device auto` refuses to guess. Choose explicitly:

```powershell
android-cli devices list --json          # see candidate serials
android-cli apps list --device <serial> --json
```

## `E_NO_DEVICE`

No phone is visible. Check the cable, USB debugging, and `adb devices` manually. For
Wi-Fi: `android-cli transport connect --wifi IP:PORT` (pair first on Android 11+:
`android-cli transport pair --wifi IP:PORT --code XXXXXX`).

## Wi-Fi keeps dropping / endpoint not reconnecting

Endpoints are saved on successful `transport connect` and reconnected once at every
collect. Verify with `android-cli transport status --json` → `saved_wifi_endpoints`.
Remove stale entries with `transport disconnect --device IP:PORT`.
Both devices must be on the same network; some guest APs block client-to-client traffic.

## `E_CONFIRM_DECLINED` (exit 3)

You declined the destructive confirmation (or stdin was not a terminal, e.g. a pipe).
Re-run interactively and answer `y`, or pass `--yes` deliberately.

## `E_UI_DUMP_FAILED`

`uiautomator dump` produced no XML. Common on locked/keyguard screens or when another
automation tool holds the uiautomator service:

- unlock the screen and retry;
- stop other automation (Appium/UiAutomator sessions);
- `android-cli ui screenshot check.png` to see what the phone actually displays.

## `E_UI_NO_MATCH`

No node matched the selector. Dump first and inspect:

```powershell
android-cli ui dump --json
android-cli ui find --text-contains "connect" --json
```

Remember: `--text` is exact, `--text-contains` is a substring.

## `ui text` rejects my string

Newlines are not supported by `input text`; send them as key events:

```powershell
android-cli ui text "line one"
android-cli ui key KEYCODE_ENTER
android-cli ui text "line two"
```

## PowerShell says `android-cli` is not recognized

The install script adds the install directory to the *user* `PATH`; open a **new**
terminal, or:

```powershell
$env:Path += ";$env:LOCALAPPDATA\Programs\android-cli"
```

## Wrong adb picked up

`--adb` beats `ACA_ADB_PATH` beats `ANDROID_HOME`/`ANDROID_SDK_ROOT` beats `PATH`.
Check what is being used: `android-cli doctor --json` → `checks[adb].detail`.

## Getting more detail

```powershell
android-cli devices list -v --json      # tracing debug on stderr
$env:RUST_LOG = "android_cli=debug"     # same, via env
```

JSON output always stays clean on stdout; logs go to stderr.
