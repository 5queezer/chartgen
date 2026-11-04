// Curated symbol list for the search dialog. There is no search
// backend, so this is a static catalogue; any other ticker can still be
// entered as free text. Routing (Binance vs Yahoo) happens server-side
// in `detect_source`, keyed on the ticker suffix.

export type SymbolKind = "crypto" | "stock" | "etf" | "index" | "fx" | "futures";

export interface SymbolEntry {
  ticker: string;
  name: string;
  kind: SymbolKind;
  /** Short exchange / venue badge. */
  exchange: string;
}

const crypto = (base: string, name: string): SymbolEntry => ({
  ticker: `${base}USDT`,
  name,
  kind: "crypto",
  exchange: "Binance",
});

const stock = (
  ticker: string,
  name: string,
  exchange = "NASDAQ",
): SymbolEntry => ({ ticker, name, kind: "stock", exchange });

const etf = (ticker: string, name: string, exchange: string): SymbolEntry => ({
  ticker,
  name,
  kind: "etf",
  exchange,
});

const index = (ticker: string, name: string, exchange: string): SymbolEntry => ({
  ticker,
  name,
  kind: "index",
  exchange,
});

const fx = (ticker: string, name: string): SymbolEntry => ({
  ticker,
  name,
  kind: "fx",
  exchange: "FX",
});

const futures = (
  ticker: string,
  name: string,
  exchange: string,
): SymbolEntry => ({ ticker, name, kind: "futures", exchange });

export const SYMBOLS: readonly SymbolEntry[] = [
  crypto("BTC", "Bitcoin"),
  crypto("ETH", "Ethereum"),
  crypto("BNB", "BNB"),
  crypto("SOL", "Solana"),
  crypto("XRP", "XRP"),
  crypto("DOGE", "Dogecoin"),
  crypto("ADA", "Cardano"),
  crypto("AVAX", "Avalanche"),
  crypto("TRX", "TRON"),
  crypto("LINK", "Chainlink"),
  crypto("DOT", "Polkadot"),
  crypto("LTC", "Litecoin"),
  crypto("BCH", "Bitcoin Cash"),
  crypto("SHIB", "Shiba Inu"),
  crypto("UNI", "Uniswap"),
  crypto("ATOM", "Cosmos"),
  crypto("XLM", "Stellar"),
  crypto("NEAR", "NEAR Protocol"),
  crypto("APT", "Aptos"),
  crypto("ARB", "Arbitrum"),
  crypto("OP", "Optimism"),
  crypto("FIL", "Filecoin"),
  crypto("AAVE", "Aave"),
  crypto("INJ", "Injective"),
  crypto("SUI", "Sui"),
  crypto("PEPE", "Pepe"),
  crypto("TON", "Toncoin"),
  crypto("ETC", "Ethereum Classic"),
  crypto("HBAR", "Hedera"),
  stock("AAPL", "Apple Inc."),
  stock("MSFT", "Microsoft Corporation"),
  stock("NVDA", "NVIDIA Corporation"),
  stock("GOOGL", "Alphabet Inc. Class A"),
  stock("AMZN", "Amazon.com, Inc."),
  stock("META", "Meta Platforms, Inc."),
  stock("TSLA", "Tesla, Inc."),
  stock("AMD", "Advanced Micro Devices"),
  stock("NFLX", "Netflix, Inc."),
  stock("INTC", "Intel Corporation"),
  stock("AVGO", "Broadcom Inc."),
  stock("COST", "Costco Wholesale"),
  stock("JPM", "JPMorgan Chase & Co.", "NYSE"),
  stock("V", "Visa Inc.", "NYSE"),
  stock("WMT", "Walmart Inc.", "NYSE"),
  stock("DIS", "The Walt Disney Company", "NYSE"),
  stock("BA", "The Boeing Company", "NYSE"),
  stock("XOM", "Exxon Mobil Corporation", "NYSE"),
  stock("COIN", "Coinbase Global, Inc."),
  stock("MSTR", "Strategy Inc."),
  etf("SPY", "SPDR S&P 500 ETF Trust", "NYSE Arca"),
  etf("QQQ", "Invesco QQQ Trust", "NASDAQ"),
  etf("IWM", "iShares Russell 2000 ETF", "NYSE Arca"),
  etf("DIA", "SPDR Dow Jones Industrial Average ETF", "NYSE Arca"),
  etf("GLD", "SPDR Gold Shares", "NYSE Arca"),
  etf("TLT", "iShares 20+ Year Treasury Bond ETF", "NASDAQ"),
  index("^GSPC", "S&P 500", "SP"),
  index("^IXIC", "NASDAQ Composite", "NASDAQ"),
  index("^DJI", "Dow Jones Industrial Average", "DJ"),
  index("^RUT", "Russell 2000", "RUSSELL"),
  index("^VIX", "CBOE Volatility Index", "CBOE"),
  index("^GDAXI", "DAX Performance Index", "XETRA"),
  index("^N225", "Nikkei 225", "TSE"),
  fx("EURUSD=X", "Euro / U.S. Dollar"),
  fx("GBPUSD=X", "British Pound / U.S. Dollar"),
  fx("USDJPY=X", "U.S. Dollar / Japanese Yen"),
  fx("AUDUSD=X", "Australian Dollar / U.S. Dollar"),
  futures("ES=F", "E-mini S&P 500 Futures", "CME"),
  futures("NQ=F", "E-mini NASDAQ-100 Futures", "CME"),
  futures("GC=F", "Gold Futures", "COMEX"),
  futures("SI=F", "Silver Futures", "COMEX"),
  futures("CL=F", "Crude Oil WTI Futures", "NYMEX"),
];

/**
 * Filter by ticker or name (case-insensitive substring). Exact ticker
 * matches rank first, then ticker prefix, other ticker matches, and
 * finally name matches.
 */
export function searchSymbols(query: string): SymbolEntry[] {
  const q = query.trim().toUpperCase();
  if (q === "") return [...SYMBOLS];
  const rank = (s: SymbolEntry): number => {
    const t = s.ticker.replace(/^\^/, "");
    if (t === q || s.ticker === q) return 0;
    if (t.startsWith(q)) return 1;
    if (t.includes(q)) return 2;
    if (s.name.toUpperCase().includes(q)) return 3;
    return 4;
  };
  return SYMBOLS.map((s) => [rank(s), s] as const)
    .filter(([r]) => r < 4)
    .sort((a, b) => a[0] - b[0])
    .map(([, s]) => s);
}
