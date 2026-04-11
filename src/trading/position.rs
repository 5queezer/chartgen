use crate::trading::order::Side;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: Side,
    pub quantity: f64,
    pub entry_price: f64,
    pub current_price: f64,
}

impl Position {
    pub fn unrealized_pnl(&self) -> f64 {
        let diff = self.current_price - self.entry_price;
        match self.side {
            Side::Buy => diff * self.quantity,
            Side::Sell => -diff * self.quantity,
        }
    }
}

pub struct PositionTracker {
    positions: Vec<Position>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
        }
    }

    /// Update or create position on a fill.
    /// If position exists for symbol+side, average into it.
    /// If position exists for opposite side, reduce/close it.
    pub fn on_fill(&mut self, symbol: &str, side: &Side, quantity: f64, price: f64) {
        let opposite = match side {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        };

        // Check for opposite-side position first.
        if let Some(idx) = self
            .positions
            .iter()
            .position(|p| p.symbol == symbol && p.side == opposite)
        {
            let existing_qty = self.positions[idx].quantity;
            if quantity < existing_qty {
                // Reduce position.
                self.positions[idx].quantity = existing_qty - quantity;
            } else if (quantity - existing_qty).abs() < f64::EPSILON {
                // Close position.
                self.positions.remove(idx);
            } else {
                // Flip: close existing and open new position with remaining qty.
                self.positions.remove(idx);
                self.positions.push(Position {
                    symbol: symbol.to_string(),
                    side: side.clone(),
                    quantity: quantity - existing_qty,
                    entry_price: price,
                    current_price: price,
                });
            }
            return;
        }

        // Same-side position: average in.
        if let Some(pos) = self
            .positions
            .iter_mut()
            .find(|p| p.symbol == symbol && p.side == *side)
        {
            let new_qty = pos.quantity + quantity;
            pos.entry_price = (pos.entry_price * pos.quantity + price * quantity) / new_qty;
            pos.quantity = new_qty;
            return;
        }

        // No existing position: create new.
        self.positions.push(Position {
            symbol: symbol.to_string(),
            side: side.clone(),
            quantity,
            entry_price: price,
            current_price: price,
        });
    }

    /// Update current price for PnL calculation.
    pub fn update_price(&mut self, symbol: &str, price: f64) {
        for pos in &mut self.positions {
            if pos.symbol == symbol {
                pos.current_price = price;
            }
        }
    }

    /// Get all open positions.
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    /// Get position for a specific symbol.
    pub fn get(&self, symbol: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.symbol == symbol)
    }

    /// Clear all positions (for restart/rebuild from exchange).
    pub fn clear(&mut self) {
        self.positions.clear();
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_fill_creates_position() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);

        assert_eq!(tracker.positions().len(), 1);
        let pos = &tracker.positions()[0];
        assert_eq!(pos.symbol, "BTCUSD");
        assert_eq!(pos.side, Side::Buy);
        assert!((pos.quantity - 1.0).abs() < f64::EPSILON);
        assert!((pos.entry_price - 50000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn same_side_fill_averages_entry_price() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 52000.0);

        assert_eq!(tracker.positions().len(), 1);
        let pos = &tracker.positions()[0];
        assert!((pos.quantity - 2.0).abs() < f64::EPSILON);
        // (50000 * 1 + 52000 * 1) / 2 = 51000
        assert!((pos.entry_price - 51000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn opposite_side_fill_reduces_position() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 2.0, 50000.0);
        tracker.on_fill("BTCUSD", &Side::Sell, 1.0, 51000.0);

        assert_eq!(tracker.positions().len(), 1);
        let pos = &tracker.positions()[0];
        assert_eq!(pos.side, Side::Buy);
        assert!((pos.quantity - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn opposite_side_fill_closes_position() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);
        tracker.on_fill("BTCUSD", &Side::Sell, 1.0, 51000.0);

        assert!(tracker.positions().is_empty());
    }

    #[test]
    fn opposite_side_fill_flips_position() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);
        tracker.on_fill("BTCUSD", &Side::Sell, 3.0, 51000.0);

        assert_eq!(tracker.positions().len(), 1);
        let pos = &tracker.positions()[0];
        assert_eq!(pos.side, Side::Sell);
        assert!((pos.quantity - 2.0).abs() < f64::EPSILON);
        assert!((pos.entry_price - 51000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn update_price_changes_current_price() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);
        tracker.update_price("BTCUSD", 55000.0);

        let pos = tracker.get("BTCUSD").unwrap();
        assert!((pos.current_price - 55000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unrealized_pnl_long_and_short() {
        // Long: profit when price goes up
        let long_pos = Position {
            symbol: "BTCUSD".to_string(),
            side: Side::Buy,
            quantity: 2.0,
            entry_price: 50000.0,
            current_price: 51000.0,
        };
        assert!((long_pos.unrealized_pnl() - 2000.0).abs() < f64::EPSILON);

        // Short: profit when price goes down
        let short_pos = Position {
            symbol: "BTCUSD".to_string(),
            side: Side::Sell,
            quantity: 2.0,
            entry_price: 50000.0,
            current_price: 49000.0,
        };
        assert!((short_pos.unrealized_pnl() - 2000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn get_returns_none_for_unknown_symbol() {
        let tracker = PositionTracker::new();
        assert!(tracker.get("BTCUSD").is_none());
    }

    #[test]
    fn clear_removes_all_positions() {
        let mut tracker = PositionTracker::new();
        tracker.on_fill("BTCUSD", &Side::Buy, 1.0, 50000.0);
        tracker.on_fill("ETHUSD", &Side::Sell, 10.0, 3000.0);
        assert_eq!(tracker.positions().len(), 2);

        tracker.clear();
        assert!(tracker.positions().is_empty());
    }
}
