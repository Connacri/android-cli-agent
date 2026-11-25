---
name: android-cli-agent
description: >-
  Control and automate Android phones from this machine over ADB with the
  android-cli binary. Use when the user asks to inspect an Android screen,
  list or launch/install/uninstall apps, tap/swipe/type on the phone,
  connect a device over Wi-Fi, screenshot the device, or run Android UI
  tests — anything involving an Android phone, adb, wireless debugging,
  or the android-cli commands.
---

# android-cli-agent

`android-cli` is an agent-native CLI: every command supports `--json` and returns
exactly one document — `{"ok":true,"data":...}` or
`{"ok":false,"error":{"code":"E_*","message":"..."}}` — with stable error codes and
exit codes `0` ok / `1` error / `2` usage / `3` confirmation declined.

## Invocation rules

1. **Always pass `--json`** and parse `data`/`error`. Never scrape human text.
2. Check the exit code: `3` means a human declined a destructive confirmation —
   report it, do not retry in a loop.
3. Pick the device explicitly when more than one phone is connected:
   `android-cli devices list --json` first, then `--device <serial>`.
   `--device auto` refuses with `E_DEVICE_AMBIGUOUS` rather than guessing.
4. Diagnostics go to stderr; stdout stays pure JSON. Log stderr only on failure.
5. Mutations on one device are serialized by the tool; you do not need to add
   your own lock, but avoid launching parallel *different* commands that fight
   over the same screen (e.g. two taps) — sequence them.

## Command map

| Goal | Command |
|---|---|
| List phones | `android-cli devices list --json` |
| Phone details | `android-cli device info --device auto --json` |
| Transports / saved Wi-Fi | `android-cli transport status --json` |
| Wi-Fi pair (Android 11+) | `android-cli transport pair --wifi IP:PORT --code XXXXXX --json` |
| Wi-Fi connect (auto-reconnect saved) | `android-cli transport connect --wifi IP:PORT --json` |
| Wi-Fi disconnect | `android-cli transport disconnect --device IP:PORT --json` |
| Installed apps | `android-cli apps list --device auto --json` |
| App details (version, pid, sdk) | `android-cli apps info --package PKG --json` |
| Launch / stop | `android-cli apps launch --package PKG --json` · `apps stop` |
| Install APK | `android-cli apps install --apk FILE --json` |
| Uninstall (destructive) | `android-cli apps uninstall --package PKG --yes --json` |
| Clear data (destructive) | `android-cli apps clear-data --package PKG --yes --json` |
| Screenshot | `android-cli ui screenshot OUT.png --json` |
| Screen hierarchy | `android-cli ui dump --json` |
| Find UI nodes | `android-cli ui find --text "OK" --json` (also `--text-contains`, `--resource-id`, `--class`, `--package`, `--content-desc`, `--clickable`) |
| Tap | `android-cli ui tap --x 540 --y 1860 --json` or `ui tap --text "OK" --json` |
| Swipe / type / keys | `ui swipe --x1 … --y1 … --x2 … --y2 …` · `ui text "…" ` · `ui key KEYCODE_BACK` |
| Appium availability (optional) | `android-cli appium status --json` |
| Diagnostics | `android-cli doctor --json` |

## Destructive operations

`apps uninstall` and `apps clear-data` prompt a human (`[y/N]`) on stderr **before**
touching the device. In agent context:

- Prefer asking the user first, then run with `--yes`.
- Never pass `--yes` on your own initiative for a package the user did not name.
- Without `--yes` and without a terminal, the command exits `3` / `E_CONFIRM_DECLINED`.

## Typical flows

**Inspect then act**

```json
android-cli devices list --json
android-cli ui dump --json
android-cli ui tap --text "Se connecter" --json
android-cli ui screenshot step2.png --json
```

**Wireless debugging**

```json
android-cli transport pair --wifi 192.168.1.42:37123 --code 482913 --json
android-cli transport connect --wifi 192.168.1.42:5555 --json
```

## Error codes you may see

`E_ADB_NOT_FOUND` (install platform-tools / set `ACA_ADB_PATH`) ·
`E_NO_DEVICE` · `E_DEVICE_NOT_FOUND` · `E_DEVICE_AMBIGUOUS` (choose `--device`) ·
`E_DEVICE_UNAUTHORIZED` (accept the USB debugging prompt) · `E_DEVICE_OFFLINE` ·
`E_WIFI_CONNECT_FAILED` / `E_WIFI_PAIR_FAILED` · `E_APP_NOT_FOUND` ·
`E_APK_NOT_FOUND` · `E_APP_OP_FAILED` · `E_CONFIRM_DECLINED` (exit 3) ·
`E_UI_NO_MATCH` · `E_UI_DUMP_FAILED` · `E_INVALID_ARG` (exit 2) · `E_ADB_TIMEOUT`.

Full reference: `docs/COMMANDS.md`; recovery steps: `docs/TROUBLESHOOTING.md`.
