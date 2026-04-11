use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

pub type OrderId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit { price: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderState {
    Pending,
    Open,
    Filled {
        fill_price: f64,
        filled_at: DateTime<Utc>,
    },
    Cancelled,
    Rejected {
        reason: String,
    },
}

impl fmt::Display for OrderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderState::Pending => write!(f, "Pending"),
            OrderState::Open => write!(f, "Open"),
            OrderState::Filled { .. } => write!(f, "Filled"),
            OrderState::Cancelled => write!(f, "Cancelled"),
            OrderState::Rejected { .. } => write!(f, "Rejected"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: f64,
    pub state: OrderState,
    pub created_at: DateTime<Utc>,
}

/// Tracks all orders and enforces valid state transitions.
pub struct OrderTracker {
    orders: Vec<Order>,
}

impl OrderTracker {
    pub fn new() -> Self {
        Self { orders: Vec::new() }
    }

    /// Create a new order in Pending state.
    pub fn create(
        &mut self,
        id: OrderId,
        symbol: String,
        side: Side,
        order_type: OrderType,
        quantity: f64,
    ) -> &Order {
        let order = Order {
            id,
            symbol,
            side,
            order_type,
            quantity,
            state: OrderState::Pending,
            created_at: Utc::now(),
        };
        self.orders.push(order);
        self.orders.last().unwrap()
    }

    /// Transition order to Open (accepted by exchange).
    pub fn mark_open(&mut self, id: &str) -> Result<(), String> {
        let order = self.find_mut(id)?;
        match &order.state {
            OrderState::Pending => {
                order.state = OrderState::Open;
                Ok(())
            }
            other => Err(format!("invalid transition: {other} → Open")),
        }
    }

    /// Transition order to Filled.
    pub fn mark_filled(&mut self, id: &str, fill_price: f64) -> Result<(), String> {
        let order = self.find_mut(id)?;
        match &order.state {
            OrderState::Open => {
                order.state = OrderState::Filled {
                    fill_price,
                    filled_at: Utc::now(),
                };
                Ok(())
            }
            other => Err(format!("invalid transition: {other} → Filled")),
        }
    }

    /// Transition order to Cancelled.
    pub fn mark_cancelled(&mut self, id: &str) -> Result<(), String> {
        let order = self.find_mut(id)?;
        match &order.state {
            OrderState::Open => {
                order.state = OrderState::Cancelled;
                Ok(())
            }
            other => Err(format!("invalid transition: {other} → Cancelled")),
        }
    }

    /// Transition order to Rejected.
    pub fn mark_rejected(&mut self, id: &str, reason: String) -> Result<(), String> {
        let order = self.find_mut(id)?;
        match &order.state {
            OrderState::Pending => {
                order.state = OrderState::Rejected { reason };
                Ok(())
            }
            other => Err(format!("invalid transition: {other} → Rejected")),
        }
    }

    /// Get order by ID.
    pub fn get(&self, id: &str) -> Option<&Order> {
        self.orders.iter().find(|o| o.id == id)
    }

    /// Get all open orders.
    pub fn open_orders(&self) -> Vec<&Order> {
        self.orders
            .iter()
            .filter(|o| o.state == OrderState::Open)
            .collect()
    }

    /// Get all orders.
    pub fn all(&self) -> &[Order] {
        &self.orders
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut Order, String> {
        self.orders
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or_else(|| format!("order not found: {id}"))
    }
}

impl Default for OrderTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> OrderTracker {
        OrderTracker::new()
    }

    #[test]
    fn create_order_starts_in_pending() {
        let mut t = make_tracker();
        let order = t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        assert_eq!(order.state, OrderState::Pending);
    }

    #[test]
    fn pending_to_open_succeeds() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        assert!(t.mark_open("1").is_ok());
        assert_eq!(t.get("1").unwrap().state, OrderState::Open);
    }

    #[test]
    fn pending_to_rejected_succeeds() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        assert!(t.mark_rejected("1", "insufficient margin".into()).is_ok());
        assert!(matches!(
            t.get("1").unwrap().state,
            OrderState::Rejected { .. }
        ));
    }

    #[test]
    fn open_to_filled_succeeds() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.mark_open("1").unwrap();
        assert!(t.mark_filled("1", 50000.0).is_ok());
        assert!(matches!(
            t.get("1").unwrap().state,
            OrderState::Filled { .. }
        ));
    }

    #[test]
    fn open_to_cancelled_succeeds() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.mark_open("1").unwrap();
        assert!(t.mark_cancelled("1").is_ok());
        assert_eq!(t.get("1").unwrap().state, OrderState::Cancelled);
    }

    #[test]
    fn pending_to_filled_fails() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        let err = t.mark_filled("1", 50000.0).unwrap_err();
        assert!(err.contains("invalid transition"));
    }

    #[test]
    fn pending_to_cancelled_fails() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        let err = t.mark_cancelled("1").unwrap_err();
        assert!(err.contains("invalid transition"));
    }

    #[test]
    fn open_to_rejected_fails() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.mark_open("1").unwrap();
        let err = t.mark_rejected("1", "bad".into()).unwrap_err();
        assert!(err.contains("invalid transition"));
    }

    #[test]
    fn filled_is_terminal() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.mark_open("1").unwrap();
        t.mark_filled("1", 50000.0).unwrap();
        assert!(t.mark_open("1").is_err());
        assert!(t.mark_cancelled("1").is_err());
        assert!(t.mark_rejected("1", "x".into()).is_err());
        assert!(t.mark_filled("1", 1.0).is_err());
    }

    #[test]
    fn cancelled_is_terminal() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.mark_open("1").unwrap();
        t.mark_cancelled("1").unwrap();
        assert!(t.mark_open("1").is_err());
        assert!(t.mark_filled("1", 1.0).is_err());
        assert!(t.mark_rejected("1", "x".into()).is_err());
        assert!(t.mark_cancelled("1").is_err());
    }

    #[test]
    fn open_orders_returns_only_open() {
        let mut t = make_tracker();
        t.create(
            "1".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            1.0,
        );
        t.create(
            "2".into(),
            "ETHUSD".into(),
            Side::Sell,
            OrderType::Limit { price: 3000.0 },
            2.0,
        );
        t.create(
            "3".into(),
            "BTCUSD".into(),
            Side::Buy,
            OrderType::Market,
            0.5,
        );
        t.mark_open("1").unwrap();
        t.mark_open("2").unwrap();
        // order 3 stays Pending
        let open = t.open_orders();
        assert_eq!(open.len(), 2);
        assert!(open.iter().all(|o| o.state == OrderState::Open));
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let t = make_tracker();
        assert!(t.get("nonexistent").is_none());
    }
}
