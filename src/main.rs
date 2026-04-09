mod data;
mod fetch;
mod indicator;
mod indicators;
mod mcp;
mod renderer;

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

    /// Candle interval (e.g., 1m, 5m, 15m, 1h, 4h, 1d, 1wk)
    #[arg(short, long, default_value = "4h")]
    interval: String,

    /// Data source: auto, binance, yahoo
    #[arg(long, default_value = "auto")]
    source: String,

    /// Run as MCP server (stdio JSON-RPC)
    #[arg(long)]
    mcp: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.mcp {
        mcp::run();
        return;
    }

    let mut data = if let Some(ref sym) = cli.symbol {
        let source = match cli.source.as_str() {
            "binance" => "binance",
            "yahoo" => "yahoo",
            _ => detect_source(sym),
        };
        match source {
            "binance" => fetch::fetch_binance(sym, &cli.interval, cli.bars).unwrap_or_else(|e| {
                eprintln!("Binance error: {}", e);
                std::process::exit(1);
            }),
            "yahoo" => fetch::fetch_yahoo(sym, &cli.interval, cli.bars).unwrap_or_else(|e| {
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

fn detect_source(symbol: &str) -> &'static str {
    let s = symbol.to_uppercase();
    let crypto_quotes = ["USDT", "BUSD", "BTC", "ETH", "BNB", "USDC", "FDUSD"];
    if crypto_quotes
        .iter()
        .any(|q| s.ends_with(q) && s.len() > q.len())
    {
        "binance"
    } else {
        "yahoo"
    }
}
