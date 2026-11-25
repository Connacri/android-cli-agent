# Compatibility

## Host

| Item | Requirement | Verified |
|---|---|---|
| OS | Windows 10/11 (primary target); Linux/macOS expected to work (CI runs Ubuntu) | CI: windows-latest + ubuntu-latest |
| Shell | PowerShell 7 recommended; scripts also run on Windows PowerShell 5.1 | — |
| Rust (source builds) | stable ≥ 1.80 (CI uses latest stable) | local: 1.100.0 |
| adb | Android SDK Platform-Tools (discovered automatically) | local: 35.x via `ANDROID_HOME` |

The binary needs no runtime other than the OS; there is no cloud component, no database
and no background service.

## Device

| Item | Requirement |
|---|---|
| Android | **8.0 (API 26) and newer** — commands rely on `pm`, `am`, `dumpsys`, `input`, `screencap`, `uiautomator dump`, all present since Android 8 |
| USB debugging | enabled; the authorization prompt must be accepted |
| Wi-Fi debugging | `adb pair` requires **Android 11+** (Wireless debugging screen); plain `adb connect IP:PORT` works on older versions with a manually started `adb tcpip` |
| Root | not required, not used |

### Feature matrix

| Feature | Min Android | Notes |
|---|---|---|
| devices/info/transport | 8 | any adb-visible state is reported |
| Wi-Fi pair (`transport pair`) | 11 | on-screen pairing code |
| Wi-Fi connect + auto-reconnect | 8 (`adb tcpip`) / 11 (wireless debugging) | endpoints persisted in `state.json` |
| apps inventory/info | 8 | `pm`/`dumpsys`/`pidof` |
| apps launch | 8 | `monkey` + `am start` fallback |
| ui screenshot/dump/tap/… | 8 | `uiautomator dump`, `input` |
| Appium UiAutomator2 detection | — | optional; classic ops never need it |

## ADB transports

- **USB**: preferred (rank 1).
- **Wi-Fi**: rank 2; a phone connected through both appearances is one logical device.
- **Emulator**: rank 3 (`emulator-*` serials are recognized).
- **Offline / unauthorized**: detected and reported with typed errors
  (`E_DEVICE_OFFLINE`, `E_DEVICE_UNAUTHORIZED`); never auto-selected.

## Appium (optional)

- Host CLI: detected on `PATH` (`appium`, `appium.cmd`, `appium.ps1`) or via
  `ACA_APPIUM_PATH`.
- Device: presence of `io.appium.uiautomator2.server(.test)` packages.
- Appium is **never** required for `devices`, `device`, `transport`, `apps`, `ui`
  or `doctor`.

## Not supported (by design)

- Controlling iOS or desktop platforms.
- Re-implementing the adb protocol (the official binary is always used).
- Headless server mode / multi-tenant hosting.
