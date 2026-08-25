//! Journald entry model and parsing of `journalctl -o json` lines.

use serde_json::Value;

/// A single journald entry, reduced to the fields the tool renders and
/// filters on.
///
/// `host` is the ssh destination the entry came from, not the remote
/// `_HOSTNAME`, so entries are labeled exactly the way the user addressed
/// the machine.
#[derive(Debug, Clone)]
pub struct Record {
    pub host: String,
    pub timestamp_us: i64,
    pub cursor: Option<String>,
    pub unit: Option<String>,
    pub pid: Option<String>,
    pub priority: Option<u8>,
    pub message: String,
}

/// Parses one `journalctl -o json` line into a [`Record`].
///
/// Returns `None` for blank or malformed lines; a live journal stream may
/// legitimately contain both. Journald encodes non-UTF-8 field payloads as
/// arrays of bytes, which are decoded lossily.
pub fn parse(host: &str, line: &str) -> Option<Record> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let obj = value.as_object()?;
    let timestamp_us = field_str(obj.get("__REALTIME_TIMESTAMP")?)?.parse().ok()?;
    Some(Record {
        host: host.to_string(),
        timestamp_us,
        cursor: obj.get("__CURSOR").and_then(field_str),
        unit: obj
            .get("_SYSTEMD_UNIT")
            .or_else(|| obj.get("UNIT"))
            .or_else(|| obj.get("SYSLOG_IDENTIFIER"))
            .and_then(field_str),
        pid: obj
            .get("_PID")
            .or_else(|| obj.get("SYSLOG_PID"))
            .and_then(field_str),
        priority: obj
            .get("PRIORITY")
            .and_then(field_str)
            .and_then(|p| p.parse().ok()),
        message: obj.get("MESSAGE").and_then(field_str).unwrap_or_default(),
    })
}

/// Extracts a journald field value, which is either a string or an array of
/// bytes for non-UTF-8 payloads.
fn field_str(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let bytes: Vec<u8> = items
                .iter()
                .filter_map(|v| v.as_u64().map(|b| b as u8))
                .collect();
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_typical_entry() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1724582400123456","__CURSOR":"s=abc","_SYSTEMD_UNIT":"backend.service","_PID":"4242","PRIORITY":"3","MESSAGE":"boom"}"#;
        let r = parse("app1", line).unwrap();
        assert_eq!(r.host, "app1");
        assert_eq!(r.timestamp_us, 1_724_582_400_123_456);
        assert_eq!(r.cursor.as_deref(), Some("s=abc"));
        assert_eq!(r.unit.as_deref(), Some("backend.service"));
        assert_eq!(r.pid.as_deref(), Some("4242"));
        assert_eq!(r.priority, Some(3));
        assert_eq!(r.message, "boom");
    }

    #[test]
    fn decodes_byte_array_messages() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1","MESSAGE":[104,105]}"#;
        assert_eq!(parse("h", line).unwrap().message, "hi");
    }

    #[test]
    fn falls_back_to_syslog_identifier() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1","SYSLOG_IDENTIFIER":"sshd","MESSAGE":"x"}"#;
        assert_eq!(parse("h", line).unwrap().unit.as_deref(), Some("sshd"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("h", "").is_none());
        assert!(parse("h", "not json").is_none());
        assert!(parse("h", r#"{"MESSAGE":"no timestamp"}"#).is_none());
    }
}
