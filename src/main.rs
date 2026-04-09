mod data;
mod indicator;
mod indicators;
mod renderer;
mod mcp;

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

    let data = data::sample_data(cli.bars);

    let panel_indicators: Vec<_> = cli.panels.iter().filter_map(|name| {
        let ind = indicators::by_name(name);
        if ind.is_none() {
            eprintln!("Unknown indicator: {} (available: {:?})", name, indicators::available());
        }
        ind
    }).collect();

    if let Err(e) = renderer::render_chart(&data, &panel_indicators, &cli.output, cli.width, cli.height) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    println!("Done: {}", cli.output);
}
