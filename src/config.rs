use clap::Parser;

/// Common CLI args used by both the binary and tests
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Force-enable the stdio MCP server on startup, overriding the persisted Settings
    /// toggle (which defaults to off). [default: false]
    #[arg(long, env = "FATHOM_MCP_STDIO_ENABLED")]
    pub mcp_stdio: bool,

    /// Run only the MCP server (headless, no UI) — its stdio IS this process's stdio.
    #[arg(long, env = "FATHOM_ONLY_MCP")]
    pub only_mcp: bool,

    /// Never start the MCP server in this process (ignores `--mcp-stdio` and the persisted
    /// enabled preference). Not a CLI flag: set by shells that
    /// must not expose an MCP endpoint (`fathom-devbridge`).
    #[arg(skip)]
    pub disable_mcp: bool,
}
