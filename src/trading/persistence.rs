use std::fs;
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;

use crate::trading::alert::Alert;

/// Load alerts from a JSON file. Returns empty vec if file doesn't exist.
pub fn load_alerts(path: &PathBuf) -> Result<Vec<Alert>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            serde_json::from_str(&contents).map_err(|e| format!("failed to parse alerts: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("failed to read alerts file: {e}")),
    }
}

/// Save alerts to a JSON file (overwrites). Writes to a .tmp file first, then renames
/// for atomic replacement. Creates parent directories if needed.
pub fn save_alerts(path: &PathBuf, alerts: &[Alert]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create parent directories: {e}"))?;
    }

    let tmp_path = path.with_extension("tmp");
    let json =
        serde_json::to_string_pretty(alerts).map_err(|e| format!("failed to serialize: {e}"))?;

    fs::write(&tmp_path, json).map_err(|e| format!("failed to write tmp file: {e}"))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("failed to rename tmp to target: {e}"))?;

    Ok(())
}

/// Audit trail logger — appends one line per event to a file.
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Log an order submission.
    pub fn log_submitted(
        &self,
        symbol: &str,
        side: &str,
        quantity: f64,
        order_type: &str,
        order_id: &str,
    ) {
        let line = format!(
            "{} SUBMITTED {} {} {} {} id={}",
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            side,
            symbol,
            quantity,
            order_type,
            order_id,
        );
        self.append(&line);
    }

    /// Log an order fill.
    pub fn log_filled(&self, symbol: &str, side: &str, quantity: f64, price: f64, order_id: &str) {
        let line = format!(
            "{} FILLED {} {} {} @ {} id={}",
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            side,
            symbol,
            quantity,
            price,
            order_id,
        );
        self.append(&line);
    }

    /// Log an order cancellation.
    pub fn log_cancelled(&self, order_id: &str) {
        let line = format!(
            "{} CANCELLED id={}",
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            order_id,
        );
        self.append(&line);
    }

    /// Log an alert trigger.
    pub fn log_alert_triggered(
        &self,
        alert_id: &str,
        condition: &str,
        symbol: &str,
        interval: &str,
    ) {
        let line = format!(
            "{} ALERT_TRIGGERED id={} {} {} {}",
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            alert_id,
            condition,
            symbol,
            interval,
        );
        self.append(&line);
    }

    fn append(&self, line: &str) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::alert::{Alert, AlertCondition};
    use chrono::Utc;

    fn make_alert(id: &str, symbol: &str, condition: AlertCondition) -> Alert {
        Alert {
            id: id.to_string(),
            symbol: symbol.to_string(),
            condition,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = std::env::temp_dir().join("chartgen_test_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("alerts.json");

        let alerts = vec![
            make_alert("a1", "BTCUSD", AlertCondition::PriceAbove(50000.0)),
            make_alert(
                "a2",
                "ETHUSDT",
                AlertCondition::IndicatorSignal {
                    indicator: "rsi".into(),
                    signal: "green_dot".into(),
                },
            ),
        ];

        save_alerts(&path, &alerts).unwrap();
        let loaded = load_alerts(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "a1");
        assert_eq!(loaded[1].id, "a2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let path = std::env::temp_dir().join("chartgen_test_nofile/does_not_exist.json");
        let loaded = load_alerts(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("chartgen_test_mkdirs/deep/nested");
        let _ = fs::remove_dir_all(std::env::temp_dir().join("chartgen_test_mkdirs"));
        let path = dir.join("alerts.json");

        save_alerts(&path, &[]).unwrap();
        assert!(path.exists());

        let _ = fs::remove_dir_all(std::env::temp_dir().join("chartgen_test_mkdirs"));
    }

    #[test]
    fn audit_log_writes_correct_format() {
        let dir = std::env::temp_dir().join("chartgen_test_audit_format");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("trades.log");

        let log = AuditLog::new(path.clone());
        log.log_submitted("XRPUSDT", "BUY", 100.0, "MARKET", "abc123");

        let contents = fs::read_to_string(&path).unwrap();
        let line = contents.trim();

        // Verify structure: timestamp SUBMITTED BUY XRPUSDT 100 MARKET id=abc123
        assert!(line.contains("SUBMITTED"));
        assert!(line.contains("BUY"));
        assert!(line.contains("XRPUSDT"));
        assert!(line.contains("100"));
        assert!(line.contains("MARKET"));
        assert!(line.contains("id=abc123"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_log_appends_multiple_writes() {
        let dir = std::env::temp_dir().join("chartgen_test_audit_append");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("trades.log");

        let log = AuditLog::new(path.clone());
        log.log_submitted("XRPUSDT", "BUY", 100.0, "MARKET", "abc123");
        log.log_filled("XRPUSDT", "BUY", 100.0, 0.812, "abc123");
        log.log_cancelled("def456");
        log.log_alert_triggered("ghi789", "cipher_b/green_dot", "XRPUSDT", "1h");

        let contents = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("SUBMITTED"));
        assert!(lines[1].contains("FILLED"));
        assert!(lines[1].contains("@ 0.812"));
        assert!(lines[2].contains("CANCELLED"));
        assert!(lines[2].contains("id=def456"));
        assert!(lines[3].contains("ALERT_TRIGGERED"));
        assert!(lines[3].contains("cipher_b/green_dot"));

        let _ = fs::remove_dir_all(&dir);
    }
}
