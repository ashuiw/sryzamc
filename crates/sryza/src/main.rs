mod authentication;
mod config;
mod configuration;
mod protocol;
mod scheduler;
mod server;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "sryzamc", version, about = "SryzaMC Minecraft server")]
struct Cli {
    #[arg(long, default_value = ".", global = true)]
    dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the server directory and default configuration.
    Init,
    /// Start the server.
    Run,
    /// Backwards-compatible alias for run.
    Start,
    /// Inspect a Paper/Spigot directory and write an inventory report.
    Migrate { source: PathBuf },
    /// Print build and protocol support information.
    Info,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::Init => config::initialize(&cli.dir),
        Command::Run | Command::Start => server::run(&cli.dir).await,
        Command::Migrate { source } => config::migration_report(&source, &cli.dir),
        Command::Info => {
            println!(
                "SryzaMC {} | Java Edition status protocol 777 (26.3)",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
    }
}
