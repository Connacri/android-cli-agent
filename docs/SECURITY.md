# Security

## Threat model (summary)

`android-cli` runs **on the operator's machine** and drives an attached Android phone
through the official `adb`. Anyone who can run it already has local code execution and,
with USB debugging enabled, substantial control over connected phones. The controls below
limit *accidental* damage, *unexpected* behavior and *credential leakage* — they do not
sandbox a hostile local user.

## Destructive operations require explicit human confirmation

Affected commands: `apps uninstall`, `apps clear-data`.

Policy (implemented in the `security` crate, enforced in `app-manager`):

1. The gate executes **before any adb process is spawned** — a declined or unreadable
   confirmation never reaches the device.
2. Accepted answers: `y` / `yes` (case-insensitive). Empty input, EOF, `n`, or anything
   else declines.
3. Declining exits with code **3** and error code `E_CONFIRM_DECLINED`.
4. `--yes` is the only bypass and must be passed explicitly by the human/agent harness
   (it is recorded in the command line, i.e. auditable).
5. The prompt names the exact action and target:
   `CONFIRM DESTRUCTIVE: apps uninstall on 'com.example.app'? [y/N]`
6. The gate reads stdin/writers through injectable handles, so automation can never
   block on — or spoof — a real terminal by accident.

`apps install` is intentionally *not* gated: it only adds data (and uses `adb install -r`,
which cannot delete user data). If you prefer gating it, it is a one-line registry change
in `security::is_destructive`.

## Secrets and data

- **No credentials are stored.** The only persisted file is
  `~/.android-cli-agent/state.json`, containing Wi-Fi endpoints (`ip:port`) previously
  connected by the operator. It contains no keys, tokens or personal data.
- The tool has **no network listeners**, no telemetry, no cloud calls. The only network
  activity is adb's own Wi-Fi connection to endpoints the operator requested.
- Output (`--json`) may include device identifiers (hardware serial, model, IP) — treat
  command output as potentially sensitive when sharing logs.
- Secrets must never be committed: no `.env`, keys or tokens in this repository
  (see `CONTRIBUTING.md`).

## Input validation

- Wi-Fi endpoints must be `HOST:PORT` with a numeric port — validated **before** adb is
  spawned (`E_WIFI_CONNECT_FAILED` with an `invalid endpoint` reason).
- `ui text` rejects newlines; `ui key` accepts only `KEYCODE_*`/digits — no shell
  metacharacters ever reach `input`.
- All adb invocations use argv arrays (no shell concatenation), so package names, APK
  paths and selectors cannot inject shell syntax.
- APK paths are checked for existence before adb runs.

## Least privilege on the phone

- The tool never requests or escalates privileges on the device; it only uses what the
  already-authorized adb session permits.
- Uninstall/clear-data operate only on the package named in the command.
- No `su`, no `pm` operations beyond the documented ones.

## Process isolation

- adb is spawned with `stdin = null` (a hung adb cannot consume terminal input) and a
  30-second timeout (`E_ADB_TIMEOUT`).
- Per-logical-device mutation locks prevent overlapping mutations that could corrupt
  device state or interleave confirmations.

## Reporting a vulnerability

Open a private GitHub security advisory on this repository (or contact the maintainer
through GitHub). Do not open a public issue for exploitable vulnerabilities.
See [SECURITY.md](../SECURITY.md) for the policy and supported versions.
