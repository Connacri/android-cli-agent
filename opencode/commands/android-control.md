---
description: Control an Android phone — tap, swipe, type, keys, launch/stop apps, manage packages
---

# android-control

Drive the connected Android phone with `android-cli`. Always use `--json`.

## Preconditions

```powershell
android-cli devices list --json
```

Pin `--device <serial>` when several phones are present.

## UI control

```powershell
android-cli ui find --text "OK" --json          # or --text-contains / --resource-id / --class
android-cli ui tap --text "OK" --json           # tap node center
android-cli ui tap --x 540 --y 1860 --json      # tap absolute pixels
android-cli ui swipe --x1 500 --y1 1500 --x2 500 --y2 500 --json
android-cli ui text "hello world" --json        # no newlines; use ui key ENTER between lines
android-cli ui key KEYCODE_BACK --json
```

Sequence UI actions one at a time; verify with `ui dump` or `ui screenshot` after
important steps.

## App lifecycle

```powershell
android-cli apps launch --package PKG --json
android-cli apps stop --package PKG --json
android-cli apps install --apk .\app.apk --json
```

## Destructive operations — confirmation required

`apps uninstall` and `apps clear-data` require explicit human approval:

1. Confirm the exact package with the user.
2. Only then run with `--yes`:

```powershell
android-cli apps uninstall --package PKG --yes --json
android-cli apps clear-data --package PKG --yes --json
```

Without `--yes` the command exits `3` (`E_CONFIRM_DECLINED`) — report that to the user
and stop. Never add `--yes` unprompted.

## Error handling

- `E_UI_NO_MATCH` → re-dump (`ui dump`) and refine the selector.
- `E_DEVICE_AMBIGUOUS` → ask which device.
- `E_APP_OP_FAILED` → show `error.message` (it contains the device's own reason).
