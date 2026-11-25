# Contributing

Thanks for improving `android-cli-agent`.

## Ground rules

- Work on `main` (or a short-lived branch when a review is requested). **Never**
  force-push, rewrite published history, or move/delete release tags.
- One logical change per commit, [Conventional Commits](https://www.conventionalcommits.org/),
  imperative subject ≤ 72 chars.
- Never commit secrets: `.env*`, keys, tokens, passwords, credentials.
- Production artifacts (release `.exe`) are built **only** by GitHub Actions — never
  build or sign releases locally.
- Do not weaken or delete tests to make CI pass; fix the code instead.

## Development setup

```powershell
# Rust stable (rustup) + Android SDK Platform-Tools (adb) are required
git clone https://github.com/Connacri/android-cli-agent.git
cd android-cli-agent
cargo build
cargo test
```

## Checks before pushing

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three checks on `ubuntu-latest` and `windows-latest`.

## Project layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Library crates never print or parse
CLI flags; the binary crate owns clap, output formatting and exit codes. New features
should land in the crate that owns that domain, with tests against a mocked
`AdbBackend`.

## Adding a command

1. Implement the logic in the owning crate with unit tests (canned backend).
2. Add the clap variant + dispatch arm in `crates/android-cli/src/main.rs`.
3. Implement the handler in `crates/android-cli/src/commands.rs`, returning
   `Success`/`CliError` (stable `E_*` code).
4. Add an end-to-end test in `crates/android-cli/tests/cli.rs` (extend the `fake-adb`
   defaults or use a `replies.json` fixture when needed).
5. Document it in `docs/COMMANDS.md` and `CHANGELOG.md`.
6. User-facing strings must work for `en`, `fr` and `ar` readers (keep them simple and
   translatable; the CLI itself is English-first).

## Reporting bugs

Open an issue with: OS/PowerShell version, `android-cli doctor --json` output (redact
serials/IPs if needed), the exact command and its JSON output.
Security issues: see [SECURITY.md](SECURITY.md).
