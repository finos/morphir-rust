use clap::{Parser, Subcommand};
use starbase::{App, AppResult, AppSession};

mod commands;

use commands::{run_generate, run_tool_install, run_tool_list, run_tool_uninstall, run_tool_update, run_transform, run_validate};

/// Morphir CLI - Rust tooling for the Morphir ecosystem
#[derive(Parser)]
#[command(name = "morphir")]
#[command(about = "Morphir CLI tool for Rust", long_about = None)]
#[command(version)]
struct Cli {
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

/// Application session for Morphir CLI
#[derive(Clone)]
struct MorphirSession {
    command: Commands,
}

#[async_trait::async_trait]
impl AppSession for MorphirSession {
    async fn execute(&mut self) -> AppResult {
        match &self.command {
            Commands::Validate { input } => {
                run_validate(input.clone())
            }
            Commands::Generate { target, input, output } => {
                run_generate(target.clone(), input.clone(), output.clone())
            }
            Commands::Transform { input, output } => {
                run_transform(input.clone(), output.clone())
            }
            Commands::Tool { action } => {
                match action {
                    ToolAction::Install { name, version } => {
                        run_tool_install(name.clone(), version.clone())
                    }
                    ToolAction::List => {
                        run_tool_list()
                    }
                    ToolAction::Update { name, version } => {
                        run_tool_update(name.clone(), version.clone())
                    }
                    ToolAction::Uninstall { name } => {
                        run_tool_uninstall(name.clone())
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> starbase::MainResult {
    let cli = Cli::parse();

    // Create session with command
    let session = MorphirSession {
        command: cli.command,
    };

    // Initialize and run starbase App
    let exit_code = App::default()
        .run(session, |mut session| async move {
            session.execute().await
        })
        .await?;
    
    Ok(std::process::ExitCode::from(exit_code))
}
