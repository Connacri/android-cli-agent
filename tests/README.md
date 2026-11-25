# tests/

Shared test assets for the whole workspace.

- `fixtures/` — canned files consumed by unit tests and shown in the docs:
  - `devices-mixed.txt` — sample `adb devices -l` output with USB, Wi-Fi,
    unauthorized and offline entries (used by `adb-engine` tests).
  - `window_dump.xml` — sample `uiautomator dump` hierarchy (used by
    `ui-engine` tests).

Executable tests live with their crates (Cargo convention):

| Kind | Location |
|---|---|
| Unit tests | `crates/*/src/lib.rs` (`#[cfg(test)]`) |
| CLI end-to-end tests (against the `fake-adb` fixture binary) | `crates/android-cli/tests/cli.rs` |
| Transport / identity scenarios | `crates/device-manager/src/lib.rs` (mocked adb backend) |
