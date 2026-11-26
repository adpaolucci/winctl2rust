#![forbid(unsafe_code)]

use clap::Parser;
use guap_mcp::PolicyMode;
use winctl2rust_core::init_logging;
use winctl2rust_mcp::{get_tools_manifest, run_stdio_server};

#[derive(Parser, Debug)]
#[command(
    name = "winctl2rust-mcp",
    version,
    about = "Policy-gated JSON-RPC 2.0 stdio MCP server for winctl2rust (22 tools / 40 endpoints)"
)]
struct McpCliArgs {
    /// Print all 40 MCP tool JSON Schemas and exit
    #[arg(long)]
    tools: bool,

    /// Enable mutating and privileged-mutating actions (still requires confirm_destructive=true per call)
    #[arg(long)]
    allow_mutating: bool,

    /// Logging verbosity written strictly to stderr (error, warn, info, debug, trace)
    #[arg(long, default_value = "warn")]
    log_level: String,
}

fn policy_from_args(args: &McpCliArgs) -> PolicyMode {
    if args.allow_mutating {
        PolicyMode::AllowMutating
    } else {
        PolicyMode::ReadOnlyOnly
    }
}

fn main() -> anyhow::Result<()> {
    let args = McpCliArgs::parse();
    init_logging(&args.log_level);
    let policy = policy_from_args(&args);

    if args.tools {
        let manifest = get_tools_manifest(policy);
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }

    run_stdio_server(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_cli_defaults_to_read_only_and_parses_flags() {
        let default_args = McpCliArgs::try_parse_from(["winctl2rust-mcp"]).unwrap();
        assert!(!default_args.tools);
        assert!(!default_args.allow_mutating);
        assert_eq!(default_args.log_level, "warn");
        assert_eq!(policy_from_args(&default_args), PolicyMode::ReadOnlyOnly);

        let mut_args =
            McpCliArgs::try_parse_from(["winctl2rust-mcp", "--tools", "--allow-mutating", "--log-level", "info"])
                .unwrap();
        assert!(mut_args.tools);
        assert!(mut_args.allow_mutating);
        assert_eq!(mut_args.log_level, "info");
        assert_eq!(policy_from_args(&mut_args), PolicyMode::AllowMutating);
    }
}
