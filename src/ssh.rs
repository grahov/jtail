//! Per-host journal streaming over ssh.

use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::mpsc::Sender;

use crate::record::{self, Record};

/// Message from a host task to the main event loop.
pub enum Event {
    /// A parsed journal entry. `live` is false for backlog replayed at
    /// session start, which must not feed clock-skew estimation.
    Entry { record: Record, live: bool },
    /// A line of operational status, printed outside the ordered stream.
    Notice { host: String, text: String },
}

/// Remote `journalctl` parameters shared by every host session.
#[derive(Clone)]
pub struct JournalArgs {
    pub units: Vec<String>,
    pub priority: Option<u8>,
    pub since: Option<String>,
    pub lines: u32,
}

const CONNECT_TIMEOUT_SECS: u32 = 10;
const RECONNECT_MAX: Duration = Duration::from_secs(30);
const STDERR_LINES_SHOWN: usize = 3;

/// Tails one host's journal forever, reconnecting with exponential backoff.
///
/// After the first received entry every reconnect resumes from the last
/// seen cursor, so entries logged during a short outage are neither lost
/// nor duplicated. Returns once the main loop closes the channel.
pub async fn run_host(host: String, args: JournalArgs, tx: Sender<Event>) {
    let mut cursor: Option<String> = None;
    let mut backoff = Duration::from_secs(1);
    loop {
        let outcome = tail_once(&host, &args, &mut cursor, &tx).await;
        if tx.is_closed() {
            return;
        }
        let text = match outcome {
            Ok(0) => format!("connection failed, retrying in {}s", backoff.as_secs()),
            Ok(_) => {
                backoff = Duration::from_secs(1);
                format!("connection lost, retrying in {}s", backoff.as_secs())
            }
            Err(err) => format!("{err:#}, retrying in {}s", backoff.as_secs()),
        };
        if notify(&tx, &host, text).await.is_err() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

/// Runs one ssh session to completion and returns how many entries it
/// yielded.
async fn tail_once(
    host: &str,
    args: &JournalArgs,
    cursor: &mut Option<String>,
    tx: &Sender<Event>,
) -> Result<u64> {
    let session_start_us = unix_micros();
    let mut child = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-T")
        .arg(host)
        .arg(remote_command(args, cursor.as_deref()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("failed to spawn ssh")?;
    let stdout = child.stdout.take().context("child stdout missing")?;
    let stderr = child.stderr.take().context("child stderr missing")?;
    tokio::spawn(forward_stderr(host.to_string(), stderr, tx.clone()));
    let mut lines = BufReader::new(stdout).lines();
    let mut received = 0u64;
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(rec) = record::parse(host, &line) else {
            continue;
        };
        received += 1;
        if let Some(c) = &rec.cursor {
            *cursor = Some(c.clone());
        }
        let live = rec.timestamp_us >= session_start_us;
        if tx.send(Event::Entry { record: rec, live }).await.is_err() {
            break;
        }
    }
    let _ = child.kill().await;
    Ok(received)
}

/// Surfaces the first few ssh diagnostics (auth failures, host key
/// complaints) as notices instead of swallowing them.
async fn forward_stderr(host: String, stderr: ChildStderr, tx: Sender<Event>) {
    let mut lines = BufReader::new(stderr).lines();
    let mut shown = 0usize;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() || shown >= STDERR_LINES_SHOWN {
            continue;
        }
        shown += 1;
        if notify(&tx, &host, format!("ssh: {line}")).await.is_err() {
            return;
        }
    }
}

async fn notify(tx: &Sender<Event>, host: &str, text: String) -> Result<(), ()> {
    tx.send(Event::Notice {
        host: host.to_string(),
        text,
    })
    .await
    .map_err(|_| ())
}

/// Builds the remote command line, resuming from `cursor` when one exists.
///
/// Cursor resume takes precedence over `--since`/`--lines`: those describe
/// where the very first session starts, while the cursor pins exactly where
/// the previous session stopped.
fn remote_command(args: &JournalArgs, cursor: Option<&str>) -> String {
    let mut parts = vec![
        "journalctl".to_string(),
        "--output=json".to_string(),
        "--follow".to_string(),
        "--no-pager".to_string(),
    ];
    for unit in &args.units {
        parts.push(format!("--unit={unit}"));
    }
    if let Some(p) = args.priority {
        parts.push(format!("--priority={p}"));
    }
    match (cursor, &args.since) {
        (Some(c), _) => parts.push(format!("--after-cursor={c}")),
        (None, Some(since)) => parts.push(format!("--since={since}")),
        (None, None) => parts.push(format!("--lines={}", args.lines)),
    }
    parts
        .iter()
        .map(|p| shell_quote(p))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quotes one argument for the remote POSIX shell.
fn shell_quote(s: &str) -> String {
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_=+./:@,".contains(c));
    if safe && !s.is_empty() {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Current wall-clock time in microseconds since the Unix epoch, the unit
/// journald timestamps use.
pub fn unix_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> JournalArgs {
        JournalArgs {
            units: vec!["backend.service".into()],
            priority: Some(4),
            since: None,
            lines: 10,
        }
    }

    #[test]
    fn builds_the_initial_command() {
        let cmd = remote_command(&args(), None);
        assert_eq!(
            cmd,
            "journalctl --output=json --follow --no-pager --unit=backend.service --priority=4 --lines=10"
        );
    }

    #[test]
    fn since_replaces_the_startup_tail() {
        let mut a = args();
        a.since = Some("-1h".into());
        let cmd = remote_command(&a, None);
        assert!(cmd.contains("--since=-1h"));
        assert!(!cmd.contains("--lines"));
    }

    #[test]
    fn cursor_resume_overrides_the_start_position() {
        let mut a = args();
        a.since = Some("-1h".into());
        let cmd = remote_command(&a, Some("s=deadbeef;i=1"));
        assert!(cmd.ends_with("'--after-cursor=s=deadbeef;i=1'"));
        assert!(!cmd.contains("--since"));
        assert!(!cmd.contains("--lines"));
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(
            shell_quote("--unit=backend.service"),
            "--unit=backend.service"
        );
        assert_eq!(shell_quote("--since=1 hour ago"), "'--since=1 hour ago'");
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
    }
}
