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
    /// Format check, UI checks + e2e, clippy, the test suite, and bindings drift
    Test,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    dagger_sdk::connect(|client| async move {
        match cli.command {
            Command::Test => pipeline::test(&client).await?,
        }
        Ok(())
    })
    .await?;

    Ok(())
}
