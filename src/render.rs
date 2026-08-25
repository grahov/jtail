//! Terminal rendering of entries and notices.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use owo_colors::{AnsiColors, OwoColorize};

use crate::record::Record;

/// Formats entries as `HH:MM:SS.mmm host unit[pid]: message`.
///
/// A host keeps one stable color derived from its name, so the same fleet
/// looks the same across runs. Message coloring follows priority: red for
/// err and above, yellow for warning, dim for debug. Continuation lines of
/// multi-line messages (stack traces) stay indented under their first line.
pub struct Renderer {
    color: bool,
    host_width: usize,
    tz: TimeZone,
}

const HOST_COLORS: [AnsiColors; 6] = [
    AnsiColors::Cyan,
    AnsiColors::Green,
    AnsiColors::Magenta,
    AnsiColors::Blue,
    AnsiColors::BrightCyan,
    AnsiColors::BrightGreen,
];

impl Renderer {
    pub fn new(color: bool, hosts: &[String]) -> Self {
        Self {
            color,
            host_width: hosts.iter().map(|h| h.len()).max().unwrap_or(0),
            tz: TimeZone::system(),
        }
    }

    pub fn entry(&self, record: &Record) -> String {
        let time = self.format_time(record.timestamp_us);
        let host = format!("{:<width$}", record.host, width = self.host_width);
        let host = self.paint(&host, host_color(&record.host));
        let source = match (&record.unit, &record.pid) {
            (Some(unit), Some(pid)) => format!("{}[{pid}]", trim_unit(unit)),
            (Some(unit), None) => trim_unit(unit).to_string(),
            (None, _) => "-".to_string(),
        };
        let message = self.paint_message(record);
        format!("{time} {host} {source}: {message}")
    }

    pub fn notice(&self, at_us: i64, host: &str, text: &str) -> String {
        let time = self.format_time(at_us);
        let line = format!("{time} --- {host}: {text} ---");
        if self.color {
            line.dimmed().to_string()
        } else {
            line
        }
    }

    fn paint_message(&self, record: &Record) -> String {
        let text = indent_continuations(&record.message);
        if !self.color {
            return text;
        }
        match record.priority {
            Some(p) if p <= 3 => text.color(AnsiColors::Red).to_string(),
            Some(4) => text.color(AnsiColors::Yellow).to_string(),
            Some(7) => text.dimmed().to_string(),
            _ => text,
        }
    }

    fn paint(&self, text: &str, color: AnsiColors) -> String {
        if self.color {
            text.color(color).to_string()
        } else {
            text.to_string()
        }
    }

    fn format_time(&self, timestamp_us: i64) -> String {
        let Ok(ts) = Timestamp::from_microsecond(timestamp_us) else {
            return "??:??:??.???".to_string();
        };
        let zoned = ts.to_zoned(self.tz.clone());
        format!(
            "{}.{:03}",
            zoned.strftime("%H:%M:%S"),
            timestamp_us.rem_euclid(1_000_000) / 1000
        )
    }
}

/// Picks a stable palette color from the host name.
fn host_color(host: &str) -> AnsiColors {
    let hash = host.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    HOST_COLORS[hash % HOST_COLORS.len()]
}

/// Drops the `.service` suffix; every other unit type stays visible.
fn trim_unit(unit: &str) -> &str {
    unit.strip_suffix(".service").unwrap_or(unit)
}

fn indent_continuations(message: &str) -> String {
    if !message.contains('\n') {
        return message.to_string();
    }
    message.replace('\n', "\n    ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(timestamp_us: i64) -> Record {
        Record {
            host: "app1".into(),
            timestamp_us,
            cursor: None,
            unit: Some("backend.service".into()),
            pid: Some("42".into()),
            priority: Some(6),
            message: "started".into(),
        }
    }

    #[test]
    fn formats_a_plain_entry() {
        let r = Renderer::new(false, &["app1".to_string(), "db-long".to_string()]);
        let line = r.entry(&record(0));
        assert!(line.ends_with("app1    backend[42]: started"));
    }

    #[test]
    fn indents_multi_line_messages() {
        let mut rec = record(0);
        rec.message = "boom\n  at Foo.bar".into();
        let r = Renderer::new(false, &["app1".to_string()]);
        assert!(r.entry(&rec).contains("boom\n      at Foo.bar"));
    }

    #[test]
    fn notices_carry_host_and_text() {
        let r = Renderer::new(false, &["app1".to_string()]);
        let line = r.notice(0, "app1", "connection lost, retrying in 1s");
        assert!(line.ends_with("--- app1: connection lost, retrying in 1s ---"));
    }
}
