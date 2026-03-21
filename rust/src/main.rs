mod cef;
#[cfg(feature = "cronet")]
mod cronet;
mod engine;
mod extract;
mod fetch;
mod mcp;
mod robots;
mod session;
mod setup;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(name = "wick", about = "Browser-grade web access for AI agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the Wick daemon
    Serve {
        /// Run as MCP server on stdio
        #[arg(long)]
        mcp: bool,
    },
    /// Fetch a URL and print content to stdout
    Fetch {
        /// The URL to fetch
        url: String,
        /// Output format: markdown, html, text
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Ignore robots.txt restrictions
        #[arg(long)]
        no_robots: bool,
    },
    /// Auto-configure MCP clients (Claude Code, Cursor)
    Setup {
        /// Download CEF for JavaScript rendering (~120MB)
        #[arg(long)]
        with_js: bool,
    },
    /// Print version information
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { mcp: true } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive(tracing::Level::WARN.into()),
                )
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .init();

            let server = mcp::WickServer::new()?;
            let service = server
                .serve(stdio())
                .await
                .inspect_err(|e| tracing::error!("serving error: {:?}", e))?;

            service.waiting().await?;
            Ok(())
        }
        Command::Serve { mcp: false } => {
            anyhow::bail!("currently only --mcp mode is supported");
        }
        Command::Fetch {
            url,
            format,
            no_robots,
        } => {
            let client = engine::Client::new()?;
            let result = fetch::fetch(
                &client,
                &url,
                extract::Format::from_str(&format),
                !no_robots,
            )
            .await?;

            if let Some(title) = &result.title {
                eprintln!("Title: {}", title);
            }
            eprintln!("Status: {} | Time: {}ms\n", result.status_code, result.timing_ms);
            print!("{}", result.content);
            Ok(())
        }
        Command::Setup { with_js } => {
            if with_js {
                setup::install_cef()?;
            }
            setup::setup()
        }
        Command::Version => {
            println!("wick {} (rust)", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
