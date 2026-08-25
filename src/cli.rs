//! Command-line interface.

use clap::Parser;

/// Merged live tail of systemd journals across ssh hosts.
///
/// Connects to every host with plain `ssh`, runs `journalctl -f -o json`
/// there, and interleaves the streams into one timestamp-ordered, colored
/// view. Nothing is installed on the remote side.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Merged live tail of systemd journals across ssh hosts"
)]
pub struct Cli {
    /// Hosts to tail: any ssh destination (`host`, `user@host`, ssh config aliases)
    #[arg(required = true, value_name = "HOST")]
    pub hosts: Vec<String>,

    /// Only show entries of this systemd unit; repeatable; `.service` may be omitted
    #[arg(short, long, value_name = "UNIT")]
    pub unit: Vec<String>,

    /// Only show entries at this priority or more severe (a name or 0-7, as in journalctl -p)
    #[arg(short, long, value_name = "LEVEL")]
    pub priority: Option<String>,

    /// Start from this point in time, in systemd time syntax (forwarded to journalctl)
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,

    /// Number of recent entries to show per host at startup; ignored with --since
    #[arg(short = 'n', long, value_name = "N", default_value_t = 10)]
    pub lines: u32,

    /// Only show entries whose message matches this regular expression
    #[arg(short, long, value_name = "REGEX")]
    pub grep: Option<String>,

    /// How long to buffer entries for cross-host ordering, in milliseconds
    #[arg(long, value_name = "MS", default_value_t = 300)]
    pub window_ms: u64,

    /// Disable colored output (the NO_COLOR environment variable works too)
    #[arg(long)]
    pub no_color: bool,
}
