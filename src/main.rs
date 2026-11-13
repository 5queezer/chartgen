mod chart_push;
#[cfg(feature = "scripting")]
mod event_log;
mod fetch;
mod intrabars;
mod mcp;
mod replay;
mod replay_fixtures;
mod search;
mod server;
#[cfg(feature = "scripting")]
mod server_alerts;
#[cfg(feature = "scripting")]
mod setup_service;
#[cfg(test)]
#[path = "test_dir.rs"]
mod test_dir;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chartgen::data;
use chartgen::engine::{Engine, EngineConfig};
use chartgen::indicators;
use chartgen::renderer;
use clap::Parser;

#[derive(Parser)]
#[command(name = "chartgen", about = "Modular trading chart generator")]
struct Cli {
    /// Indicator panels (bottom to top). Available: macd, wavetrend, rsi, cipher_b
    #[arg(short, long, default_values_t = vec!["cipher_b".to_string(), "macd".to_string()])]
    panels: Vec<String>,

    /// Output file
    #[arg(short, long, default_value = "chart.png")]
    output: String,

    /// Image width
    #[arg(long, default_value_t = 1920)]
    width: u32,

    /// Image height
    #[arg(long, default_value_t = 1080)]
    height: u32,

    /// Number of bars
    #[arg(short = 'n', long, default_value_t = 120)]
    bars: usize,

    /// Trading symbol (e.g., BTCUSDT, AAPL, MSFT). If omitted, uses sample data.
    #[arg(short, long)]
    symbol: Option<String>,

    /// Candle interval: a number and a unit (e.g., 1m, 7m, 45m, 4h, 8h, 1d, 2D, 2W, 1M)
    #[arg(short, long, default_value = "4h")]
    interval: String,

    /// Data source: auto, binance, yahoo
    #[arg(long, default_value = "auto")]
    source: String,

    /// Run as MCP server (stdio JSON-RPC)
    #[arg(long)]
    mcp: bool,

    /// Run as remote MCP HTTP server with OAuth 2.1 PKCE
    #[arg(long)]
    serve: bool,

    /// Port for HTTP server (default 9315)
    #[arg(long, default_value_t = 9315)]
    port: u16,

    /// Run in trading mode with live WebSocket feed
    #[arg(long)]
    trade: bool,

    /// Use Binance testnet (paper trading)
    #[arg(long)]
    testnet: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Write replay fixtures of real Binance 1m bars, for CHARTGEN_REPLAY_FIXTURES (ADR-0068)
    ReplayFixtures(replay_fixtures::Args),
}

fn main() {
    // Here and nowhere a test runs: asking for the store may open it.
    #[cfg(feature = "scripting")]
    chartgen::scripting::persistence::open_on_demand();
    let cli = Cli::parse();

    if let Some(Command::ReplayFixtures(args)) = &cli.command {
        if let Err(e) = replay_fixtures::run(args) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    // A replay answers over HTTP, where a call has an owner and a clock. Any other mode
    // would read live prices while the caller believes it is replaying.
    if replay::fixtures_dir().is_some() && (cli.mcp || cli.trade || !cli.serve) {
        eprintln!("CHARTGEN_REPLAY_FIXTURES needs --serve, without --mcp or --trade");
        std::process::exit(2);
    }

    if cli.mcp {
        mcp::run();
        return;
    }

    if cli.serve {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(server::run_server(cli.port, None));
        return;
    }

    if cli.trade {
        let symbol = cli.symbol.clone().unwrap_or_else(|| "BTCUSDT".to_string());
        // The engine folds the kline stream as it arrives, so it needs a real one.
        let interval = match chartgen::interval::Interval::parse(&cli.interval) {
            Ok(iv) if iv.binance_stream().is_some() => iv.canonical(),
            _ => {
                eprintln!(
                    "--trade needs an interval Binance streams natively, not {}",
                    cli.interval
                );
                std::process::exit(2);
            }
        };
        let indicators: Vec<String> = cli.panels.clone();

        let data_dir = dirs_data_dir();
        let notifications =
            match chartgen::config::NotificationConfig::load(&data_dir.join("config.toml")) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("Invalid notification config: {error}");
                    std::process::exit(2);
                }
            };

        let config = EngineConfig {
            symbol: symbol.clone(),
            interval: interval.clone(),
            indicators,
            data_dir,
            notification_ttl_secs: notifications.notification_ttl_secs,
            max_queue_size: notifications.max_queue_size,
        };

        let engine = Arc::new(RwLock::new(Engine::new(config)));
        let engine_for_feed = engine.clone();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Spawn the feed/engine loop
            let _testnet = cli.testnet;
            tokio::spawn(async move {
                chartgen::engine::run_engine(engine_for_feed, |triggered| {
                    eprintln!(
                        "[engine] alert triggered: {} {:?} value={}",
                        triggered.alert.symbol, triggered.alert.condition, triggered.value
                    );
                })
                .await;
            });

            eprintln!(
                "[trade] engine started for {} @ {} (testnet={})",
                symbol, interval, cli.testnet
            );

            // Start the MCP HTTP server with engine reference
            server::run_server(cli.port, Some(engine)).await;
        });
        return;
    }

    let mut data = if let Some(ref sym) = cli.symbol {
        let source = match cli.source.as_str() {
            "binance" => "binance",
            "yahoo" => "yahoo",
            _ => fetch::detect_source(sym),
        };
        match source {
            "binance" => fetch::fetch_binance(sym, &cli.interval, cli.bars, None)
                .map(|f| f.data)
                .unwrap_or_else(|e| {
                    eprintln!("Binance error: {}", e);
                    std::process::exit(1);
                }),
            "yahoo" => fetch::fetch_yahoo(sym, &cli.interval, cli.bars, None)
                .map(|f| f.data)
                .unwrap_or_else(|e| {
                    eprintln!("Yahoo error: {}", e);
                    std::process::exit(1);
                }),
            _ => unreachable!(),
        }
    } else {
        data::sample_data(cli.bars)
    };
    data.symbol = cli.symbol.clone();
    data.interval = Some(cli.interval.clone());

    let panel_indicators: Vec<_> = cli
        .panels
        .iter()
        .filter_map(|name| {
            let ind = indicators::by_name(name);
            if ind.is_none() {
                eprintln!(
                    "Unknown indicator: {} (available: {:?})",
                    name,
                    indicators::available()
                );
            }
            ind
        })
        .collect();

    if let Err(e) =
        renderer::render_chart(&data, &panel_indicators, &cli.output, cli.width, cli.height)
    {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    println!("Done: {}", cli.output);
}

/// Returns the data directory for chartgen state (~/.chartgen/).
fn dirs_data_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".chartgen");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
