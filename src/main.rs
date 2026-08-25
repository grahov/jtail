//! jtail: merged live tail of systemd journals across ssh hosts.

#![forbid(unsafe_code)]

mod cli;
mod filter;
mod merge;
mod record;
mod render;
mod skew;
mod ssh;

use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::cli::Cli;
use crate::filter::Filters;
use crate::merge::Merger;
use crate::render::Renderer;
use crate::skew::SkewTracker;
use crate::ssh::{Event, JournalArgs};

const CHANNEL_CAPACITY: usize = 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let priority = cli
        .priority
        .as_deref()
        .map(filter::parse_priority)
        .transpose()?;
    let grep = cli
        .grep
        .as_deref()
        .map(Regex::new)
        .transpose()
        .context("invalid --grep pattern")?;
    let filters = Filters::new(&cli.unit, priority, grep);
    let color =
        !cli.no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    let renderer = Renderer::new(color, &cli.hosts);
    let journal_args = JournalArgs {
        units: cli.unit.iter().map(|u| filter::normalize_unit(u)).collect(),
        priority,
        since: cli.since.clone(),
        lines: cli.lines,
    };

    let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
    for host in &cli.hosts {
        tokio::spawn(ssh::run_host(
            host.clone(),
            journal_args.clone(),
            tx.clone(),
        ));
    }
    drop(tx);

    let mut merger = Merger::new(Duration::from_millis(cli.window_ms));
    let mut skews: HashMap<String, SkewTracker> = HashMap::new();
    let mut flush = tokio::time::interval(FLUSH_INTERVAL);
    flush.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => break,
            event = rx.recv() => match event {
                None => break,
                Some(Event::Entry { record, live }) => {
                    if live {
                        let tracker = skews.entry(record.host.clone()).or_default();
                        if let Some(skew_us) = tracker.observe(record.timestamp_us, ssh::unix_micros()) {
                            let text = format!(
                                "clock skew vs local ~{:+.1}s, cross-host ordering unreliable",
                                skew_us as f64 / 1e6
                            );
                            println!("{}", renderer.notice(ssh::unix_micros(), &record.host, &text));
                        }
                    }
                    if filters.matches(&record) {
                        merger.push(record, Instant::now());
                    }
                }
                Some(Event::Notice { host, text }) => {
                    println!("{}", renderer.notice(ssh::unix_micros(), &host, &text));
                }
            },
            _ = flush.tick() => {
                for record in merger.pop_ready(Instant::now()) {
                    println!("{}", renderer.entry(&record));
                }
            }
        }
    }

    for record in merger.drain() {
        println!("{}", renderer.entry(&record));
    }
    Ok(())
}
