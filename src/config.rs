//! Notification settings in the optional `~/.chartgen/config.toml`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationConfig {
    pub notification_ttl_secs: u64,
    pub max_queue_size: usize,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notification_ttl_secs: 3600,
            max_queue_size: 1000,
        }
    }
}

impl NotificationConfig {
    /// Load optional settings, using defaults only when the file is absent.
    /// Other read errors and invalid contents are startup errors.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(input) => Self::parse(&input).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    // The supported TOML subset is two bare root keys with positive decimal
    // integers (optional '+' and digit separators), comments and blank lines.
    // Reject unsupported syntax rather than pretending to parse general TOML.
    fn parse(input: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut seen = HashSet::new();
        for (index, raw) in input.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| {
                format!("line {line_number}: expected setting = positive decimal integer")
            })?;
            let (key, value) = (key.trim(), value.trim());
            if !matches!(key, "notification_ttl_secs" | "max_queue_size") {
                return Err(format!("line {line_number}: unsupported setting {key:?}"));
            }
            if !seen.insert(key) {
                return Err(format!("line {line_number}: duplicate setting {key}"));
            }
            let invalid = || {
                format!("line {line_number}: {key} must be a positive decimal integer no greater than {}", i64::MAX)
            };
            let decimal = value.strip_prefix('+').unwrap_or(value);
            if !decimal
                .split('_')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(invalid());
            }
            let digits = decimal.replace('_', "");
            if digits.starts_with('0') && digits.len() > 1 {
                return Err(invalid());
            }
            let number = digits.parse::<i64>().map_err(|_| invalid())?;
            if number <= 0 {
                return Err(invalid());
            }
            match key {
                "notification_ttl_secs" => config.notification_ttl_secs = number as u64,
                "max_queue_size" => {
                    config.max_queue_size = usize::try_from(number).map_err(|_| {
                        format!("line {line_number}: {key} exceeds this platform's capacity")
                    })?;
                }
                _ => unreachable!(),
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn omitted_settings_keep_defaults() {
        assert_eq!(
            NotificationConfig::parse("# no overrides\n\n").unwrap(),
            NotificationConfig::default()
        );
        let config = NotificationConfig::parse("max_queue_size = 25").unwrap();
        assert_eq!(config.notification_ttl_secs, 3600);
        assert_eq!(config.max_queue_size, 25);
        let config = NotificationConfig::parse("notification_ttl_secs = 60").unwrap();
        assert_eq!(config.notification_ttl_secs, 60);
        assert_eq!(config.max_queue_size, 1000);
    }

    #[test]
    fn positive_decimal_overrides_allow_comments_and_separators() {
        let config = NotificationConfig::parse("# notifications\r\n notification_ttl_secs = +7_200 # two hours\r\nmax_queue_size=1_500\r\n").unwrap();
        assert_eq!(config.notification_ttl_secs, 7200);
        assert_eq!(config.max_queue_size, 1500);
        let config =
            NotificationConfig::parse("notification_ttl_secs = 1\nmax_queue_size = 1").unwrap();
        assert_eq!(config.notification_ttl_secs, 1);
        assert_eq!(config.max_queue_size, 1);
    }

    #[test]
    fn invalid_values_are_not_silently_defaulted_or_truncated() {
        for key in ["notification_ttl_secs", "max_queue_size"] {
            for value in [
                "0",
                "-1",
                "1.5",
                "true",
                "\"20\"",
                "",
                "01",
                "_1",
                "1_",
                "1__0",
                "0x10",
                "1e3",
                "9223372036854775808",
                "18446744073709551616",
            ] {
                let input = format!("# header\n{key} = {value}");
                let error = NotificationConfig::parse(&input).unwrap_err();
                assert!(error.contains("line 2"), "{input}: {error}");
                assert!(error.contains(key), "{input}: {error}");
            }
        }
    }

    #[test]
    fn unsupported_or_duplicate_entries_fail_with_a_line_number() {
        for input in [
            "unrecognized = 1",
            "[notifications]",
            "max_queue_size 12",
            "max_queue_size = 2 = 3",
            "max_queue_size = 1\nmax_queue_size = 2",
        ] {
            let error = NotificationConfig::parse(input).unwrap_err();
            assert!(error.contains("line"), "{error}");
        }
    }

    #[test]
    fn missing_file_uses_defaults_and_present_file_is_loaded() {
        let dir = crate::test_dir::new(format!("chartgen_config_{}", uuid::Uuid::new_v4()));
        let path = dir.join("config.toml");
        assert_eq!(
            NotificationConfig::load(&path).unwrap(),
            NotificationConfig::default()
        );
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "notification_ttl_secs = 120\nmax_queue_size = 3").unwrap();
        assert_eq!(
            NotificationConfig::load(&path).unwrap(),
            NotificationConfig {
                notification_ttl_secs: 120,
                max_queue_size: 3
            }
        );
        fs::write(&path, "max_queue_size = 0").unwrap();
        let error = NotificationConfig::load(&path).unwrap_err();
        assert!(error.contains(path.to_str().unwrap()));
        assert!(error.contains("line 1"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_path_is_an_error() {
        let dir = crate::test_dir::new(format!("chartgen_config_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let error = NotificationConfig::load(&dir).unwrap_err();
        assert!(error.contains(dir.to_str().unwrap()));
        fs::remove_dir_all(&dir).unwrap();
    }
}
