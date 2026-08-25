# jtail

Merged live tail of systemd journals across your ssh hosts.

`stern` gives Kubernetes users one command to follow every pod of a service.
`jtail` does the same for plain servers: it connects to each host over ssh,
runs `journalctl` there, and interleaves the streams into a single
timestamp-ordered, colored view.

```
jtail app1 app2 db1 --unit backend --priority warning
```

## Why

Fleets of three to twenty boxes usually have no log stack: Loki or ELK is too
much to operate at that size, so incidents get debugged in tmux with a
`journalctl -f` pane per host. jtail replaces that ritual with one command and
**zero agents** — nothing is installed on the servers. If you can run
`ssh host journalctl`, you can jtail it.

## Requirements

- Local: the `ssh` binary. Key or agent auth is required — jtail runs ssh in
  BatchMode and never prompts for passwords.
- Remote: systemd with `journalctl`, and a user allowed to read the journal
  (typically membership in the `systemd-journal` or `adm` group).
- ssh config aliases, jump hosts, and agent forwarding work as usual: hosts
  are passed to `ssh` verbatim.

## Install

```
cargo install --path .
```

Binary releases are planned.

## Usage

```
jtail HOST [HOST ...] [OPTIONS]
```

| Flag | Meaning |
| --- | --- |
| `-u, --unit UNIT` | only this systemd unit; repeatable; `.service` may be omitted |
| `-p, --priority LEVEL` | this priority or more severe: a name or 0-7, as in `journalctl -p` |
| `--since WHEN` | start point in systemd time syntax: `-1h`, `2026-08-25 14:00` |
| `-n, --lines N` | recent entries per host at startup (default 10) |
| `-g, --grep REGEX` | only messages matching the regex (applied client-side) |
| `--window-ms MS` | cross-host ordering buffer (default 300) |
| `--no-color` | plain output; the `NO_COLOR` environment variable is honored too |

Examples:

```
jtail app1 app2 app3 --unit backend --since -30m
jtail web@prod-1 web@prod-2 --priority err
jtail app1 db1 --grep 'OutOfMemory|deadlock'
```

## How it works

One ssh process per host runs `journalctl --follow --output=json`; jtail
parses the stream, buffers entries for a short window, and releases them in
timestamp order. Each host keeps a stable color derived from its name, and
multi-line messages (stack traces) stay indented under their first line.

Connection loss is marked inline in the stream and retried with exponential
backoff. Every reconnect resumes from the journal cursor of the last entry
seen, so entries logged during a short outage are neither lost nor duplicated.

## Ordering, honestly

Cross-host ordering is best-effort:

- Entries interleave correctly within the buffering window (`--window-ms`).
  An entry delayed beyond it is printed late; its own timestamp still tells
  the true story.
- Ordering is only as good as the fleet's clocks. jtail estimates the offset
  between each remote journal and the local clock and prints a warning when
  it exceeds a couple of seconds.

## Roadmap

- `--save FILE`: write the raw merged ndjson alongside the pretty output
- `--stats`: per-host message and error rates
- user-journal (`--user`) support
- host groups in a config file
- binary releases

## License

MIT
