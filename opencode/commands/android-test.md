---
description: Run Android UI tests — launch an app, exercise flows, verify state, report results
---

# android-test

Run a UI test session against the connected Android phone with `android-cli`.
Always use `--json` and keep evidence (JSON outputs, screenshots) for the report.

## Setup

```powershell
android-cli devices list --json
android-cli doctor --json
```

Stop on `E_NO_DEVICE` / `E_DEVICE_UNAUTHORIZED` / unhealthy doctor with a clear
diagnosis. Pin `--device <serial>` when several phones are connected.

## Test loop

For each step of the scenario:

1. **Act** — `apps launch`, `ui tap`, `ui text`, `ui swipe`, `ui key`.
2. **Observe** — `ui dump --json` (and `ui screenshot step-N.png --json` when the
   visual result matters).
3. **Assert** — check the expected node/state exists in the dump, e.g.
   `ui find --text "Se connecter" --json` must return `data` with ≥1 node, or
   `ui find --resource-id com.app:id/error` must be empty.
4. Record pass/fail per step with the exact command and its `error.code` on failure.

## Useful assertions

| Assertion | Command |
|---|---|
| element visible | `ui find --text "…" --json` → non-empty `data` |
| element absent | `ui find --resource-id "…" --json` → empty `data` |
| app running | `apps info --package PKG --json` → `data.pid != null` |
| app version | `apps info --package PKG --json` → `data.version_name` |
| clean state | `apps clear-data --package PKG --yes --json` (destructive: ask first) |

## Rules

- Destructive steps (`clear-data`, `uninstall`) need user approval before `--yes`.
- Sequence UI actions; do not parallelize taps on the same device.
- End with a summary: steps run, passed/failed, evidence paths, and any
  `error.code` encountered.
