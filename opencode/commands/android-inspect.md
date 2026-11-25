---
description: Inspect an Android phone — devices, screen hierarchy, screenshot, app inventory
---

# android-inspect

Inspect the connected Android phone(s) with `android-cli`. Always use `--json`.

## 1. Inventory

```powershell
android-cli devices list --json
```

- Report each logical device: id, state, transport (usb/wifi), model, Android version.
- If `identity_ambiguous` is true or the result is `E_DEVICE_AMBIGUOUS`, ask the user
  which serial to use and repeat with `--device <serial>`.
- If `E_DEVICE_UNAUTHORIZED` → tell the user to accept the USB debugging prompt.
- If `E_NO_DEVICE` → suggest `android-cli transport connect --wifi IP:PORT` or check
  the cable; end with the diagnosis, do not loop.

## 2. Device details

```powershell
android-cli device info --device auto --json
```

## 3. What is on screen

```powershell
android-cli ui dump --json
```

Summarize the visible nodes: text, resource-id, class, clickable. Then take a
screenshot for the user:

```powershell
android-cli ui screenshot ./screen.png --json
```

Report the saved path.

## 4. App inventory (if relevant)

```powershell
android-cli apps list --device auto --json
android-cli apps info --package <PACKAGE> --json
```

## Rules

- Never modify anything in inspect mode (no tap/swipe/text/install/uninstall).
- Surface `error.code` + `error.message` verbatim when a command fails.
- If more than one phone is connected, always pin `--device <serial>`.
