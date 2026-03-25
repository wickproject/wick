mod analytics;
mod captcha;
mod cef;
#[cfg(feature = "cronet")]
mod cronet;
mod download;
mod engine;
mod media;
mod extract;
mod fetch;
mod mcp;
mod pro;
mod robots;
mod search;
mod session;
mod setup;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(name = "wick", about = "Browser-grade web access for AI agents")]
struct Cli {
    /// SOCKS5 proxy for residential IP tunneling (e.g., socks5://localhost:1080)
    /// Also reads from WICK_PROXY environment variable.
    #[arg(long, global = true, env = "WICK_PROXY")]
    proxy: Option<String>,

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
    /// Search the web and print results
    Search {
        /// Search query
        query: String,
        /// Number of results (default 5)
        #[arg(short, long, default_value = "5")]
        num: usize,
    },
    /// Download media (video/audio) from a URL
    Download {
        /// The URL containing media to download
        url: String,
        /// Output directory (default: current directory)
        #[arg(short, long)]
        output: Option<String>,
        /// Just show info, don't download
        #[arg(long)]
        info: bool,
    },
    /// Auto-configure MCP clients (Claude Code, Cursor)
    Setup,
    /// Manage Pro subscription
    Pro {
        #[command(subcommand)]
        action: ProAction,
    },
    /// Print version information
    Version,
}

#[derive(Subcommand)]
enum ProAction {
    /// Activate Pro (opens browser for $20/month subscription)
    Activate {
        /// Use an existing API key instead of creating a new subscription
        #[arg(long)]
        key: Option<String>,
    },
    /// Show Pro subscription status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let proxy = cli.proxy.as_deref();

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

            let server = mcp::WickServer::new(proxy)?;
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
            let client = engine::Client::new(proxy)?;
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
        Command::Search { query, num } => {
            let client = engine::Client::new(proxy)?;
            let results = search::search(&client, &query, num).await?;
            println!("{}", search::format_results(&results));
            Ok(())
        }
        Command::Download { url, output, info } => {
            if info {
                let vi = download::info(&url).await?;
                println!("Title: {}", vi.title);
                if let Some(dur) = vi.duration_secs {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    println!("Duration: {}:{:02}", mins, secs);
                }
                println!("Format: {}", vi.format);
                println!("Size: {}", vi.size_approx);
            } else {
                let result = download::download(&url, output.as_deref()).await?;
                println!("Downloaded: {}", result.path);
                println!("Size: {:.1} MB", result.size_mb);
            }
            Ok(())
        }
        Command::Setup => {
            analytics::ping("install");
            setup::setup()
        }
        Command::Pro { action } => match action {
            ProAction::Activate { key } => pro::activate(key).await,
            ProAction::Status => pro::status().await,
        },
        Command::Version => {
            let pro = if cef::is_available() { " + Pro" } else { "" };
            println!("wick {}{} (rust)", env!("CARGO_PKG_VERSION"), pro);
            Ok(())
        }
    }
}
