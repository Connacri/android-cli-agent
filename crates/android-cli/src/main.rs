use clap::Parser;

/// Agent-native CLI to inspect, control and automate Android devices over ADB.
///
/// Designed for humans (PowerShell 7) and AI agents: every command supports
/// `--json` with a uniform `{"ok":...}` envelope and stable error codes.
#[derive(Debug, Parser)]
#[command(name = "android-cli", version, about, long_about = None)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
