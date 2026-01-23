use clap::{Parser, Subcommand};
use starbase::{App, AppResult, AppSession};

mod commands;
mod help;

use commands::{
    run_dist_install, run_dist_list, run_dist_uninstall, run_dist_update, run_extension_install,
    run_extension_list, run_extension_uninstall, run_extension_update, run_generate, run_migrate,
    run_tool_install, run_tool_list, run_tool_uninstall, run_tool_update, run_transform,
    run_validate, run_version,
};

/// Morphir CLI - Tools for functional domain modeling and business logic
#[derive(Parser)]
#[command(name = "morphir")]
#[command(about = "CLI for working with Morphir IR - functional domain modeling and business logic", long_about = None)]
#[command(version)]
#[command(disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Print help (use --help-all to include experimental commands)
    #[arg(short, long, action = clap::ArgAction::Help)]
    help: Option<bool>,

    /// Print help including experimental commands
    #[arg(long)]
    help_all: bool,

    /// Print version
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Subcommand)]
enum Commands {
    // ===== Experimental Commands (hidden by default) =====
    /// [Experimental] Validate Morphir IR models
    #[command(hide = true)]
    Validate {
        /// Path to the Morphir IR file or directory
        #[arg(short, long)]
        input: Option<String>,
    },
    /// [Experimental] Generate code from Morphir IR
    #[command(hide = true)]
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
    /// [Experimental] Transform Morphir IR
    #[command(hide = true)]
    Transform {
        /// Path to the Morphir IR file or directory
        #[arg(short, long)]
        input: Option<String>,
        /// Output path
        #[arg(short, long)]
        output: Option<String>,
    },

    // ===== Stable Commands =====
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
    /// Print version information
    Version {
        /// Output version info as JSON
        #[arg(long)]
        json: bool,
    },

    // ===== Internal/Hidden Commands =====
    /// Output usage spec for documentation generation
    #[command(hide = true)]
    Usage,
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
            Commands::Version { json } => run_version(*json),
            Commands::Usage => {
                use clap::CommandFactory;
                let cli = Cli::command();
                let spec: usage::Spec = cli.into();
                println!("{}", spec);
                Ok(None)
            }
        }
    }
}

#[tokio::main]
async fn main() -> starbase::MainResult {
    use clap::CommandFactory;

    // Check for help/version flags first to print our custom banner
    let args: Vec<String> = std::env::args().collect();

    if help::should_show_banner(&args) {
        help::print_banner();
    }

    // Handle full help variants
    if help::should_show_full_help(&args) {
        help::print_full_help::<Cli>();
        return Ok(std::process::ExitCode::SUCCESS);
    }

    // Handle version subcommand early (before starbase) to avoid double execution
    if args.len() >= 2 && args[1] == "version" {
        let json = args.iter().any(|a| a == "--json");
        if let Some(code) = run_version(json)? {
            return Ok(std::process::ExitCode::from(code));
        }
        return Ok(std::process::ExitCode::SUCCESS);
    }

    // Handle usage subcommand early (before starbase) to avoid double execution
    if args.len() >= 2 && args[1] == "usage" {
        use clap::CommandFactory;
        let cli = Cli::command();
        let spec: usage::Spec = cli.into();
        println!("{}", spec);
        return Ok(std::process::ExitCode::SUCCESS);
    }

    let cli = Cli::parse();

    // Handle case where no command is provided
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            Cli::command().print_help().ok();
            return Ok(std::process::ExitCode::SUCCESS);
        }
    };

    // Create session with command
    let session = MorphirSession { command };

    // Initialize and run starbase App
    let exit_code = App::default()
        .run(
            session,
            |mut session| async move { session.execute().await },
        )
        .await?;

    Ok(std::process::ExitCode::from(exit_code))
}
