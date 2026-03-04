mod client;
mod commands;
mod config;
mod error;
mod platform;
mod state;

use clap::{Parser, Subcommand};
use colored::Colorize;

#[derive(Parser)]
#[command(name = "forge", version = "0.1.0", about = "Forge CLI — tool manager for the artifact portal")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Override portal URL
    #[arg(long, global = true)]
    portal_url: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with the portal via device pairing
    Login,

    /// Browse available tools in the registry
    Catalog {
        /// Output raw JSON
        #[arg(long)]
        json: bool,
    },

    /// Install a tool (with dependency resolution)
    Install {
        /// Tool slug to install
        tool: String,

        /// Project slug (required if tool name is ambiguous)
        #[arg(long)]
        project: Option<String>,

        /// Target platform (default: auto-detect)
        #[arg(long)]
        platform: Option<String>,

        /// Install directory (default: /opt/tools)
        #[arg(long)]
        path: Option<String>,

        /// Skip all confirmation prompts
        #[arg(long, short)]
        yes: bool,

        /// Include optional dependencies
        #[arg(long)]
        with_optional: bool,

        /// Skip recommended dependencies
        #[arg(long)]
        skip_recommended: bool,
    },

    /// Upgrade installed tools to the latest version
    Upgrade {
        /// Tool slug to upgrade (omit with --all)
        tool: Option<String>,

        /// Upgrade all installed tools
        #[arg(long)]
        all: bool,

        /// Skip confirmation prompts
        #[arg(long, short)]
        yes: bool,
    },

    /// Remove an installed tool
    Uninstall {
        /// Tool slug to remove
        tool: String,

        /// Project slug (required if ambiguous)
        #[arg(long)]
        project: Option<String>,

        /// Force removal even if other tools depend on it
        #[arg(long)]
        force: bool,

        /// Also remove orphaned auto-dependencies
        #[arg(long)]
        cascade: bool,
    },

    /// List installed tools
    List,

    /// Verify integrity of installed tools via SHA-256
    Verify {
        /// Tool slug to verify (omit with --all)
        tool: Option<String>,

        /// Verify all installed tools
        #[arg(long)]
        all: bool,
    },

    /// Show detailed information about a tool
    Info {
        /// Tool slug
        tool: String,

        /// Project slug (required if ambiguous)
        #[arg(long)]
        project: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let portal_url = cli.portal_url.as_deref();

    let result = match cli.command {
        Commands::Login => commands::login::run(portal_url).await,
        Commands::Catalog { json } => commands::catalog::run(portal_url, json).await,
        Commands::Install {
            tool,
            project,
            platform,
            path,
            yes,
            with_optional,
            skip_recommended,
        } => {
            commands::install::run(
                &tool,
                project.as_deref(),
                platform.as_deref(),
                path.as_deref(),
                portal_url,
                yes,
                with_optional,
                skip_recommended,
            )
            .await
        }
        Commands::Upgrade { tool, all, yes } => {
            commands::upgrade::run(tool.as_deref(), all, yes, portal_url).await
        }
        Commands::Uninstall {
            tool,
            project,
            force,
            cascade,
        } => commands::uninstall::run(&tool, project.as_deref(), force, cascade).await,
        Commands::List => commands::list::run().await,
        Commands::Verify { tool, all } => commands::verify::run(tool.as_deref(), all).await,
        Commands::Info { tool, project } => {
            commands::info::run(&tool, project.as_deref(), portal_url).await
        }
    };

    if let Err(e) = result {
        // Check for domain-specific errors
        let root = e.root_cause();
        if let Some(forge_err) = root.downcast_ref::<error::ForgeError>() {
            match forge_err {
                error::ForgeError::NotAuthenticated => {
                    eprintln!("{} {forge_err}", "Error:".red().bold());
                }
                error::ForgeError::TokenExpired => {
                    eprintln!("{} {forge_err}", "Error:".red().bold());
                }
                _ => {
                    eprintln!("{} {forge_err}", "Error:".red().bold());
                }
            }
        } else {
            eprintln!("{} {e:#}", "Error:".red().bold());
        }
        std::process::exit(1);
    }
}
