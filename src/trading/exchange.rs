use serde::{Deserialize, Serialize};

pub type OrderId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit { price: f64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: f64,
    pub status: OrderStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: Side,
    pub quantity: f64,
    pub entry_price: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Balance {
    pub asset: String,
    pub available: f64,
    pub locked: f64,
}

#[async_trait::async_trait]
pub trait Exchange: Send + Sync {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderId, String>;
    async fn cancel_order(&self, id: &OrderId) -> Result<(), String>;
    async fn open_orders(&self) -> Result<Vec<Order>, String>;
    async fn positions(&self) -> Result<Vec<Position>, String>;
    async fn account_balance(&self) -> Result<Vec<Balance>, String>;
}

// ---------------------------------------------------------------------------
// BinanceTestnet
// ---------------------------------------------------------------------------

pub struct BinanceTestnet {
    api_key: String,
    secret_key: String,
    client: reqwest::Client,
    base_url: String,
}

impl BinanceTestnet {
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key,
            client: reqwest::Client::new(),
            base_url: "https://testnet.binance.vision".to_string(),
        }
    }

    fn timestamp_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_millis() as u64
    }
}

fn sign_params(params: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(params.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn side_to_str(side: &Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

fn parse_order_status(s: &str) -> OrderStatus {
    match s {
        "NEW" => OrderStatus::New,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" => OrderStatus::Cancelled,
        "REJECTED" => OrderStatus::Rejected,
        _ => OrderStatus::Rejected,
    }
}

fn parse_side(s: &str) -> Side {
    match s {
        "BUY" => Side::Buy,
        _ => Side::Sell,
    }
}

fn parse_order_type(s: &str, price: Option<f64>) -> OrderType {
    match s {
        "LIMIT" => OrderType::Limit {
            price: price.unwrap_or(0.0),
        },
        _ => OrderType::Market,
    }
}

#[async_trait::async_trait]
impl Exchange for BinanceTestnet {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderId, String> {
        let (type_str, extra) = match &req.order_type {
            OrderType::Market => ("MARKET", String::new()),
            OrderType::Limit { price } => ("LIMIT", format!("&price={price}&timeInForce=GTC")),
        };

        let ts = Self::timestamp_ms();
        let params = format!(
            "symbol={}&side={}&type={}&quantity={}{}&timestamp={}",
            req.symbol,
            side_to_str(&req.side),
            type_str,
            req.quantity,
            extra,
            ts,
        );
        let signature = sign_params(&params, &self.secret_key);
        let url = format!(
            "{}/api/v3/order?{}&signature={}",
            self.base_url, params, signature
        );

        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        body["orderId"]
            .as_u64()
            .map(|id| id.to_string())
            .or_else(|| body["orderId"].as_str().map(|s| s.to_string()))
            .ok_or_else(|| format!("unexpected response: {body}"))
    }

    async fn cancel_order(&self, id: &OrderId) -> Result<(), String> {
        // cancel_order requires the symbol; we pass it via the id as "SYMBOL:ORDER_ID"
        // or we can query open orders first. For simplicity, the caller must provide
        // "SYMBOL:ORDER_ID" format.
        let parts: Vec<&str> = id.splitn(2, ':').collect();
        let (symbol, order_id) = match parts.as_slice() {
            [sym, oid] => (*sym, *oid),
            _ => return Err("cancel_order id must be SYMBOL:ORDER_ID".to_string()),
        };

        let ts = Self::timestamp_ms();
        let params = format!("symbol={symbol}&orderId={order_id}&timestamp={ts}");
        let signature = sign_params(&params, &self.secret_key);
        let url = format!(
            "{}/api/v3/order?{}&signature={}",
            self.base_url, params, signature
        );

        let resp = self
            .client
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let body = resp.text().await.map_err(|e| e.to_string())?;
            Err(format!("cancel failed: {body}"))
        }
    }

    async fn open_orders(&self) -> Result<Vec<Order>, String> {
        let ts = Self::timestamp_ms();
        let params = format!("timestamp={ts}");
        let signature = sign_params(&params, &self.secret_key);
        let url = format!(
            "{}/api/v3/openOrders?{}&signature={}",
            self.base_url, params, signature
        );

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let body: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;

        Ok(body
            .iter()
            .map(|o| {
                let price = o["price"].as_str().and_then(|s| s.parse::<f64>().ok());
                Order {
                    id: o["orderId"].to_string(),
                    symbol: o["symbol"].as_str().unwrap_or_default().to_string(),
                    side: parse_side(o["side"].as_str().unwrap_or("SELL")),
                    order_type: parse_order_type(o["type"].as_str().unwrap_or("MARKET"), price),
                    quantity: o["origQty"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    status: parse_order_status(o["status"].as_str().unwrap_or("NEW")),
                }
            })
            .collect())
    }

    async fn positions(&self) -> Result<Vec<Position>, String> {
        // Spot testnet has no positions endpoint; derive from account balances.
        let balances = self.account_balance().await?;
        Ok(balances
            .into_iter()
            .filter(|b| b.available > 0.0 || b.locked > 0.0)
            .map(|b| Position {
                symbol: b.asset.clone(),
                side: Side::Buy,
                quantity: b.available + b.locked,
                entry_price: 0.0, // spot API does not track entry price
            })
            .collect())
    }

    async fn account_balance(&self) -> Result<Vec<Balance>, String> {
        let ts = Self::timestamp_ms();
        let params = format!("timestamp={ts}");
        let signature = sign_params(&params, &self.secret_key);
        let url = format!(
            "{}/api/v3/account?{}&signature={}",
            self.base_url, params, signature
        );

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        let arr = body["balances"]
            .as_array()
            .ok_or_else(|| format!("unexpected account response: {body}"))?;

        Ok(arr
            .iter()
            .map(|b| Balance {
                asset: b["asset"].as_str().unwrap_or_default().to_string(),
                available: b["free"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
                locked: b["locked"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binance_testnet_new() {
        let exch = BinanceTestnet::new("key123".into(), "secret456".into());
        assert_eq!(exch.api_key, "key123");
        assert_eq!(exch.secret_key, "secret456");
        assert_eq!(exch.base_url, "https://testnet.binance.vision");
    }

    #[test]
    fn test_sign_params() {
        // Known HMAC-SHA256 test vector
        let signature = sign_params("symbol=BTCUSDT&side=BUY&timestamp=1234567890", "my_secret");
        // Verify it produces a 64-char hex string (SHA-256 = 32 bytes = 64 hex chars)
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));

        // Same inputs must produce the same signature
        let sig2 = sign_params("symbol=BTCUSDT&side=BUY&timestamp=1234567890", "my_secret");
        assert_eq!(signature, sig2);

        // Different secret must produce a different signature
        let sig3 = sign_params(
            "symbol=BTCUSDT&side=BUY&timestamp=1234567890",
            "other_secret",
        );
        assert_ne!(signature, sig3);
    }
}
