use clap::Parser;

/// Common CLI args used by both the binary and tests
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Enable MCP HTTP server (localhost:18288) [default: false]
    #[arg(long, env = "FATHOM_MCP_HTTP_ENABLED")]
    pub mcp_http: bool,

    /// MCP HTTP server port (default: 18288 - 1.8288m ≈ 1 fathom)
    #[arg(long, env = "FATHOM_MCP_PORT", default_value = "18288")]
    pub mcp_port: u16,

    /// Run only MCP server (headless, no UI)
    #[arg(long, env = "FATHOM_ONLY_MCP")]
    pub only_mcp: bool,

    /// Never start the MCP server in this process (ignores `--mcp-http`, the persisted
    /// auto-start preference, and `StartMcpServer`). Not a CLI flag: set by shells that
    /// must not expose an unguarded MCP endpoint (`fathom-devbridge`).
    #[arg(skip)]
    pub disable_mcp: bool,
}
