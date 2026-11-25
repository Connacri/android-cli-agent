# Architecture

`android-cli-agent` is a Cargo workspace of eight crates. The binary crate composes the
library crates; nothing talks to a phone except through the official `adb` executable.

```
android-cli (bin)
 ├── adb-engine        spawn/discover/classify/parse adb
 ├── device-manager    logical identity · transports · Wi-Fi · locks · state file
 ├── app-manager       inventory + lifecycle + confirmation gate
 ├── ui-engine         screencap · uiautomator dump · input
 ├── appium-engine     optional detection (host PATH + device packages)
 ├── security          confirmation gate primitives (injectable IO)
 └── output            {"ok":...} envelope · stable codes · exit codes
```

## Command flow

```
clap parse (global flags: --json --device --adb --yes --v)
   └─ dispatch (main.rs)
        └─ commands.rs::*
             ├─ ctx.devices()  → DeviceManager<Adb>
             ├─ resolve(selector) → SelectedDevice { logical device, command_serial }
             ├─ [mutations] mgr.lock_device(logical_id)   ← per-device lock
             └─ engine op → Success { data, text, exit } | CliError { code, message, exit }
                  └─ emit: JSON envelope (stdout) or text (stdout); tracing → stderr
```

## adb-engine

- Discovery order: explicit `--adb` → `ACA_ADB_PATH` → `ANDROID_HOME`/`ANDROID_SDK_ROOT`
  (`platform-tools/adb[.exe]`) → `PATH`.
- Every invocation has a 30 s timeout and captures **raw bytes** (`exec-out screencap -p`
  returns a PNG).
- `AdbBackend` is an RPITIT trait (`fn run(&self, &[&str]) -> impl Future + Send`), so all
  higher layers are testable against canned mocks — and futures stay `Send`.
- `classify_failure` maps classic messages to typed errors: `E_DEVICE_NOT_FOUND`,
  `E_DEVICE_OFFLINE`, `E_DEVICE_UNAUTHORIZED`, `E_ADB_TIMEOUT`, …

## Logical device identity

The same phone often appears twice in `adb devices` (hardware serial + `ip:port`).
`device-manager::group_logical_devices` merges entries with these rules:

1. **Hardware serial** (`ro.serialno`, `ro.boot.serialno`, `ro.boot.serial`) — strong key:
   USB + Wi-Fi entries merge into one logical device, `identity_verified = true`.
2. **Fallback fingerprint** (`manufacturer|model|product`) — weak key used only when the
   hardware serial is unreadable:
   - one physical (non-Wi-Fi) entry + Wi-Fi entries → merged, `identity_ambiguous = true`;
   - several physical entries with the same fingerprint → **distinct phones**: kept
     separate, both flagged ambiguous, and `--device auto` refuses with
     `E_DEVICE_AMBIGUOUS` (explicit serial still works).
3. **No properties at all** (offline/unauthorized entries) → the entry stays alone;
   it is never auto-bound to another transport because its identity cannot be verified.

Transport priority: **USB (rank 1) > Wi-Fi (rank 2) > emulator (rank 3)**. Offline entries
are never selected. If USB disappears, the same logical device is served over Wi-Fi only
after its identity was verified from properties.

## Persistence

One flat JSON file, no database: `~/.android-cli-agent/state.json`
(`%USERPROFILE%`/`$HOME`, overridable with `ANDROID_CLI_AGENT_HOME`).

```json
{ "wifi_endpoints": ["192.168.1.42:5555"] }
```

Saved endpoints are reconnected once (best effort) on every `devices`/`resolve` collect,
then `adb devices -l` is re-read. Writes are atomic (`.tmp` + rename).

## Concurrency

- `DeviceManager::lock_device(logical_id)` returns a per-logical-device async mutex:
  two mutations **never** overlap on the same phone, whatever transport each uses.
- Different logical devices proceed in parallel (each mutation holds its own lock).
- The CLI takes the lock around every mutating command (`apps launch/stop/install/
  uninstall/clear-data`, `ui tap/swipe/text/key`).

## Confirmation gates (security crate)

Destructive operations (`apps uninstall`, `apps clear-data`) call
`security::confirm_with` **before any adb process is spawned**:

- accepted answers: `y` / `yes` (case-insensitive); anything else declines;
- `--yes` records explicit pre-authorization (agents/CI where the human passed the flag);
- the reader/writer are injectable, so tests never block on a real terminal.

## Output contract

```json
{"ok":true,"data":{...}}
{"ok":false,"error":{"code":"E_DEVICE_NOT_FOUND","message":"..."}}
```

Exactly one document on stdout per `--json` invocation; diagnostics go to stderr.
Exit codes: `0` OK · `1` runtime error · `2` usage · `3` confirmation declined.

## Testing strategy

| Layer | Approach |
|---|---|
| parsers (adb, XML, bounds) | pure unit tests + shared fixtures in `tests/fixtures/` |
| identity/transport/locks | canned `AdbBackend` mocks (tokio tests, parallel-lock proof) |
| apps/ui/appium | canned mocks per command key |
| CLI end-to-end | real `android-cli` binary against the bundled `fake-adb` bin (`crates/android-cli/tests/cli.rs`) |

No physical phone is required for the test suite; CI runs it on Ubuntu and Windows.
