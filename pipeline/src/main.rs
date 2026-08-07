mod pipeline;
mod targets;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pipeline", about = "Fathom's Dagger-powered build pipeline")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Quick multi-target debug build (linux-x86_64, linux-arm64, windows-x86_64)
    Build,
    /// Format check, clippy, and the test suite
    Test,
    /// Release build + packaged archives for every target, exported to dist/
    Release,
    /// Full pipeline: test, then release
    Ci,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    dagger_sdk::connect(|client| async move {
        match cli.command {
            Command::Build => pipeline::build(&client).await?,
            Command::Test => pipeline::test(&client).await?,
            Command::Release => pipeline::release(&client).await?,
            Command::Ci => {
                pipeline::test(&client).await?;
                pipeline::release(&client).await?;
            }
        }
        Ok(())
    })
    .await?;

    Ok(())
}
