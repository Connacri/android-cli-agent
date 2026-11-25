//! `android-cli` — agent-native CLI to inspect, control and automate Android
//! devices over ADB.
//!
//! Contract:
//! - `--json` prints exactly one document on stdout:
//!   `{"ok":true,"data":...}` or `{"ok":false,"error":{"code","message"}}`.
//! - Diagnostics (tracing) always go to stderr, never to stdout.
//! - Exit codes: 0 success, 1 runtime error, 2 usage error, 3 confirmation
//!   declined (see `output::exit`).

mod commands;
mod context;

use clap::{Args, Parser, Subcommand};
use context::{CliError, Ctx, Success};
use std::path::PathBuf;

/// Agent-native CLI to inspect, control and automate Android devices over ADB.
///
/// Designed for humans (PowerShell 7) and AI agents: every command supports
/// `--json` with a uniform `{"ok":...}` envelope and stable error codes.
#[derive(Debug, Parser)]
#[command(
    name = "android-cli",
    version,
    about = "Inspect, control and automate Android devices over ADB",
    long_about = None
)]
struct Cli {
    /// Emit one JSON document: {"ok":true,"data":...} or {"ok":false,"error":{...}}
    #[arg(long, global = true)]
    json: bool,

    /// Device selector: `auto` (default), a logical id, a hardware serial
    /// or any adb serial (pins that transport).
    #[arg(long, global = true, default_value = "auto")]
    device: String,

    /// Explicit path to the adb executable (overrides ACA_ADB_PATH discovery)
    #[arg(long, global = true)]
    adb: Option<PathBuf>,

    /// Pre-authorize destructive confirmations (apps uninstall, apps clear-data)
    #[arg(short = 'y', long, global = true)]
    yes: bool,

    /// Verbose diagnostics on stderr
    #[arg(short = 'v', long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List detected devices (logical identity, transports, states)
    Devices {
        #[command(subcommand)]
        cmd: DevicesCmd,
    },
    /// Details of one resolved device
    Device {
        #[command(subcommand)]
        cmd: DeviceCmd,
    },
    /// ADB transports: status, Wi-Fi pairing, connect, disconnect
    Transport {
        #[command(subcommand)]
        cmd: TransportCmd,
    },
    /// Installed applications and lifecycle operations
    Apps {
        #[command(subcommand)]
        cmd: AppsCmd,
    },
    /// UI inspection and input control (section 7 of the specification)
    Ui {
        #[command(subcommand)]
        cmd: UiCmd,
    },
    /// Optional Appium UiAutomator2 engine status (never required)
    Appium {
        #[command(subcommand)]
        cmd: AppiumCmd,
    },
    /// Diagnose the local setup (adb, devices, state dir, appium)
    Doctor,
}

#[derive(Debug, Subcommand)]
enum DevicesCmd {
    /// List all detected devices
    List,
}

#[derive(Debug, Subcommand)]
enum DeviceCmd {
    /// Show detailed information about the resolved device
    Info,
}

#[derive(Debug, Subcommand)]
enum TransportCmd {
    /// Show transport status for every device plus saved Wi-Fi endpoints
    Status,
    /// Pair with an Android 11+ device (on-screen pairing code)
    Pair {
        /// IP:PORT shown by the device
        #[arg(long = "wifi")]
        wifi: String,
        /// 6-digit pairing code displayed on the phone
        #[arg(long)]
        code: Option<String>,
    },
    /// Connect to a paired Wi-Fi endpoint (saved for auto-reconnect)
    Connect {
        /// IP:PORT of the device
        #[arg(long = "wifi")]
        wifi: String,
    },
    /// Disconnect Wi-Fi endpoints of a device selector or an IP:PORT
    Disconnect {
        /// Logical selector, serial or IP:PORT (falls back to the global --device)
        #[arg(long)]
        device: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum AppsCmd {
    /// List installed packages (system/user flags included)
    List,
    /// Detailed information about one package
    Info {
        #[arg(long)]
        package: String,
    },
    /// Launch the package's launcher activity
    Launch {
        #[arg(long)]
        package: String,
    },
    /// Force-stop a package
    Stop {
        #[arg(long)]
        package: String,
    },
    /// Install (or replace) an APK file
    Install {
        #[arg(long)]
        apk: PathBuf,
    },
    /// Uninstall a package (destructive: asks for confirmation)
    Uninstall {
        #[arg(long)]
        package: String,
    },
    /// Clear a package's data (destructive: asks for confirmation)
    ClearData {
        #[arg(long)]
        package: String,
    },
}

/// Shared selector fields for `ui find` / `ui tap` (AND semantics).
#[derive(Debug, Clone, Default, Args)]
struct UiFilterArgs {
    /// Exact text match
    #[arg(long)]
    text: Option<String>,
    /// Substring match on the node text
    #[arg(long)]
    text_contains: Option<String>,
    /// Exact resource-id match, e.g. com.example:id/title
    #[arg(long = "resource-id")]
    resource_id: Option<String>,
    /// Exact class match, e.g. android.widget.Button
    #[arg(long)]
    class: Option<String>,
    /// Exact package match
    #[arg(long)]
    package: Option<String>,
    /// Exact content-desc match
    #[arg(long = "content-desc")]
    content_desc: Option<String>,
    /// Only clickable (true) or non-clickable (false) nodes
    #[arg(long)]
    clickable: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum UiCmd {
    /// Capture a screenshot to a PNG file
    Screenshot {
        /// Destination file (default: screenshot-<epoch>.png)
        out: Option<PathBuf>,
    },
    /// Dump the UI hierarchy as parsed nodes
    Dump,
    /// Find UI nodes matching a selector
    Find {
        #[command(flatten)]
        filter: UiFilterArgs,
    },
    /// Tap at coordinates, or on the first node matching the selector
    Tap {
        #[arg(long)]
        x: Option<i32>,
        #[arg(long)]
        y: Option<i32>,
        #[command(flatten)]
        filter: UiFilterArgs,
    },
    /// Swipe from (x1,y1) to (x2,y2)
    Swipe {
        #[arg(long)]
        x1: i32,
        #[arg(long)]
        y1: i32,
        #[arg(long)]
        x2: i32,
        #[arg(long)]
        y2: i32,
        /// Duration in milliseconds (default 300)
        #[arg(long)]
        duration: Option<u32>,
    },
    /// Type text into the focused field (spaces supported)
    Text { text: String },
    /// Send a key event (KEYCODE_* name or numeric code)
    Key { keycode: String },
}

#[derive(Debug, Subcommand)]
enum AppiumCmd {
    /// Report optional Appium / UiAutomator2 availability
    Status,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let ctx = Ctx::new(cli.json, cli.yes, cli.device, cli.adb);
    let result = dispatch(&ctx, cli.command).await;

    let code = match result {
        Ok(success) => {
            emit_success(&ctx, &success);
            success.exit
        }
        Err(err) => {
            emit_error(&ctx, &err);
            err.exit
        }
    };
    std::process::exit(code);
}

async fn dispatch(ctx: &Ctx, command: Command) -> Result<Success, CliError> {
    tracing::debug!(selector = %ctx.selector, json = ctx.json, "dispatch");
    match command {
        Command::Devices { cmd } => match cmd {
            DevicesCmd::List => commands::devices_list(ctx).await,
        },
        Command::Device { cmd } => match cmd {
            DeviceCmd::Info => commands::device_info(ctx).await,
        },
        Command::Transport { cmd } => match cmd {
            TransportCmd::Status => commands::transport_status(ctx).await,
            TransportCmd::Pair { wifi, code } => {
                commands::transport_pair(ctx, &wifi, code.as_deref()).await
            }
            TransportCmd::Connect { wifi } => commands::transport_connect(ctx, &wifi).await,
            TransportCmd::Disconnect { device } => {
                commands::transport_disconnect(ctx, device.as_deref()).await
            }
        },
        Command::Apps { cmd } => match cmd {
            AppsCmd::List => commands::apps_list(ctx).await,
            AppsCmd::Info { package } => commands::apps_info(ctx, &package).await,
            AppsCmd::Launch { package } => commands::apps_launch(ctx, &package).await,
            AppsCmd::Stop { package } => commands::apps_stop(ctx, &package).await,
            AppsCmd::Install { apk } => commands::apps_install(ctx, &apk).await,
            AppsCmd::Uninstall { package } => commands::apps_uninstall(ctx, &package).await,
            AppsCmd::ClearData { package } => commands::apps_clear_data(ctx, &package).await,
        },
        Command::Ui { cmd } => match cmd {
            UiCmd::Screenshot { out } => commands::ui_screenshot(ctx, out.as_deref()).await,
            UiCmd::Dump => commands::ui_dump(ctx).await,
            UiCmd::Find { filter } => commands::ui_find(ctx, &filter).await,
            UiCmd::Tap { x, y, filter } => commands::ui_tap(ctx, x, y, &filter).await,
            UiCmd::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration,
            } => commands::ui_swipe(ctx, x1, y1, x2, y2, duration).await,
            UiCmd::Text { text } => commands::ui_text(ctx, &text).await,
            UiCmd::Key { keycode } => commands::ui_key(ctx, &keycode).await,
        },
        Command::Appium { cmd } => match cmd {
            AppiumCmd::Status => commands::appium_status(ctx).await,
        },
        Command::Doctor => commands::doctor(ctx).await,
    }
}

fn init_tracing(verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let default = if verbose { "debug" } else { "warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

fn emit_success(ctx: &Ctx, success: &Success) {
    if ctx.json {
        let _ = output::print_json_success(&success.data);
    } else if !success.text.is_empty() {
        println!("{}", success.text);
    }
}

fn emit_error(ctx: &Ctx, err: &CliError) {
    if ctx.json {
        let _ = output::print_json_error(err.code, &err.message);
    } else {
        eprintln!("error[{}]: {}", err.code, err.message);
    }
}
