use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use starbase::{App, AppResult, AppSession};

mod commands;

use commands::{
    run_dist_install, run_dist_list, run_dist_uninstall, run_dist_update, run_extension_install,
    run_extension_list, run_extension_uninstall, run_extension_update, run_generate, run_migrate,
    run_tool_install, run_tool_list, run_tool_uninstall, run_tool_update, run_transform,
    run_validate,
};

fn print_banner() {
    use owo_colors::{OwoColorize, XtermColors};

    // Morphir brand colors: blue (#00A3E0) and orange (#F26522)
    let blue = XtermColors::from(33);    // Bright blue (xterm 33)
    let orange = XtermColors::from(208); // Orange (xterm 208)

    // ASCII art "morphir" with "morph" in blue and "ir" in orange
    println!();
    println!(
        "  {}{}",
        "_ __ ___   ___  _ __ _ __ | |__".color(blue),
        "(_)_ __".color(orange)
    );
    println!(
        " {}{}",
        "| '_ ` _ \\ / _ \\| '__| '_ \\| '_ \\".color(blue),
        "| | '__|".color(orange)
    );
    println!(
        " {}{}",
        "| | | | | | (_) | |  | |_) | | | ".color(blue),
        "| | |".color(orange)
    );
    println!(
        " {}{}",
        "|_| |_| |_|\\___/|_|  | .__/|_| |_".color(blue),
        "|_|_|".color(orange)
    );
    println!("                     {}", "|_|".color(blue));
    println!(
        "  v{} (built {})",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE")
    );
    println!();
}

/// Morphir CLI - Rust tooling for the Morphir ecosystem
#[derive(Parser)]
#[command(name = "morphir")]
#[command(about = "Morphir CLI tool for Rust", long_about = None)]
#[command(version)]
#[command(disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Print help
    #[arg(short, long, action = clap::ArgAction::Help)]
    help: Option<bool>,

    /// Print version
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Subcommand)]
enum Commands {
    /// Validate Morphir IR models
    Validate {
        /// Path to the Morphir IR file or directory
        #[arg(short, long)]
        input: Option<String>,
    },
    /// Generate code from Morphir IR
    Generate {
        /// Target language or format
        #[arg(short, long)]
        target: Option<String>,
        /// Path to the Morphir IR file or directory
        #[arg(short, long)]
        input: Option<String>,
        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Transform Morphir IR
    Transform {
        /// Path to the Morphir IR file or directory
        #[arg(short, long)]
        input: Option<String>,
        /// Output path
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Manage Morphir tools, distributions, and extensions
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Manage Morphir distributions
    Dist {
        #[command(subcommand)]
        action: DistAction,
    },
    /// Manage Morphir extensions
    Extension {
        #[command(subcommand)]
        action: ExtensionAction,
    },
    /// Manage Morphir IR
    Ir {
        #[command(subcommand)]
        action: IrAction,
    },
    /// Generate JSON Schema for Morphir IR
    Schema {
        /// Output file path (optional)
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

#[derive(Clone, Subcommand)]
enum ToolAction {
    /// Install a Morphir tool or extension
    Install {
        /// Name of the tool to install
        name: String,
        /// Version to install (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// List installed Morphir tools
    List,
    /// Update an installed Morphir tool
    Update {
        /// Name of the tool to update
        name: String,
        /// Version to update to (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Uninstall a Morphir tool
    Uninstall {
        /// Name of the tool to uninstall
        name: String,
    },
}

#[derive(Clone, Subcommand)]
enum DistAction {
    /// Install a Morphir distribution
    Install {
        /// Name of the distribution to install
        name: String,
        /// Version to install (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// List installed Morphir distributions
    List,
    /// Update an installed Morphir distribution
    Update {
        /// Name of the distribution to update
        name: String,
        /// Version to update to (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Uninstall a Morphir distribution
    Uninstall {
        /// Name of the distribution to uninstall
        name: String,
    },
}

#[derive(Clone, Subcommand)]
enum ExtensionAction {
    /// Install a Morphir extension
    Install {
        /// Name of the extension to install
        name: String,
        /// Version to install (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// List installed Morphir extensions
    List,
    /// Update an installed Morphir extension
    Update {
        /// Name of the extension to update
        name: String,
        /// Version to update to (defaults to latest)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Uninstall a Morphir extension
    Uninstall {
        /// Name of the extension to uninstall
        name: String,
    },
}

#[derive(Clone, Subcommand)]
enum IrAction {
    /// Migrate IR between versions
    Migrate {
        /// Input file, directory, or remote source (e.g., github:owner/repo)
        #[arg(short, long)]
        input: String,
        /// Output file or directory
        #[arg(short, long)]
        output: std::path::PathBuf,
        /// Target version (v4 or classic)
        #[arg(long)]
        target_version: Option<String>,
        /// Force refresh cached remote sources
        #[arg(long)]
        force_refresh: bool,
        /// Skip cache entirely for remote sources
        #[arg(long)]
        no_cache: bool,
    },
}

/// Application session for Morphir CLI
#[derive(Clone)]
struct MorphirSession {
    command: Commands,
}

#[async_trait::async_trait]
impl AppSession for MorphirSession {
    async fn execute(&mut self) -> AppResult {
        match &self.command {
            Commands::Validate { input } => run_validate(input.clone()),
            Commands::Generate {
                target,
                input,
                output,
            } => run_generate(target.clone(), input.clone(), output.clone()),
            Commands::Transform { input, output } => run_transform(input.clone(), output.clone()),
            Commands::Tool { action } => match action {
                ToolAction::Install { name, version } => {
                    run_tool_install(name.clone(), version.clone())
                }
                ToolAction::List => run_tool_list(),
                ToolAction::Update { name, version } => {
                    run_tool_update(name.clone(), version.clone())
                }
                ToolAction::Uninstall { name } => run_tool_uninstall(name.clone()),
            },
            Commands::Dist { action } => match action {
                DistAction::Install { name, version } => {
                    run_dist_install(name.clone(), version.clone())
                }
                DistAction::List => run_dist_list(),
                DistAction::Update { name, version } => {
                    run_dist_update(name.clone(), version.clone())
                }
                DistAction::Uninstall { name } => run_dist_uninstall(name.clone()),
            },
            Commands::Extension { action } => match action {
                ExtensionAction::Install { name, version } => {
                    run_extension_install(name.clone(), version.clone())
                }
                ExtensionAction::List => run_extension_list(),
                ExtensionAction::Update { name, version } => {
                    run_extension_update(name.clone(), version.clone())
                }
                ExtensionAction::Uninstall { name } => run_extension_uninstall(name.clone()),
            },
            Commands::Ir { action } => match action {
                IrAction::Migrate {
                    input,
                    output,
                    target_version,
                    force_refresh,
                    no_cache,
                } => run_migrate(
                    input.clone(),
                    output.clone(),
                    target_version.clone(),
                    *force_refresh,
                    *no_cache,
                ),
            },
            Commands::Schema { output } => commands::schema::run_schema(output.clone()),
        }
    }
}

#[tokio::main]
async fn main() -> starbase::MainResult {
    // Check for help/version flags first to print our custom banner
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1
        || args.iter().any(|a| a == "--help" || a == "-h")
        || args.iter().any(|a| a == "--version" || a == "-V")
    {
        print_banner();
    }

    let cli = Cli::parse();

    // Create session with command
    let session = MorphirSession {
        command: cli.command,
    };

    // Initialize and run starbase App
    let exit_code = App::default()
        .run(
            session,
            |mut session| async move { session.execute().await },
        )
        .await?;

    Ok(std::process::ExitCode::from(exit_code))
}
