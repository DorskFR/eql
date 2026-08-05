# eql

Self-updating Magelo-style profile for EverQuest Legends: a headless Rust
daemon on the game machine uploads inventory dumps and log events to a Rust
API, which renders equipment, derived stats, and spells — plus a
browser-based UI layout designer that deploys skins back into the game.

Tickets live in the YouTrack `EQL` project.

| Path | What |
|---|---|
| `crates/eql-core` | Shared types + parsers (inventory dumps, layouts) |
| `crates/eqld` | Daemon: watches game folder, uploads, installs skins |
| `crates/eqls` | Server: ingest API, item DB, stat engine, web UI |
| `fixtures/` | Sample game artifacts (skin XMLs, layout, ini) used by tests |

```sh
cargo test --workspace
```

## eqld config

```toml
[game]
root = 'C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest Legends'

[api]
url = "https://eql.dorsk.dev"
token = "…"

[tools.log_reader]
enabled = true              # harvest headlessly via `eql_atlas --replay`
overlays = ["dps"]          # in-game windows to launch alongside it
```

| Key | Default | What |
|---|---|---|
| `enabled` | `false` | Run `eql_atlas --replay` each tick and ship the JSON it writes. Windowless. |
| `exe` | discovered | Path to `eql_atlas`; the other tools are found next to it. |
| `version` | `v2.0` | Upstream release `install-tools` fetches. |
| `replay_secs` | `120` | Seconds between replays (min 10). |
| `replay_timeout_secs` | `600` | Kill a replay that outlives this (min 10). |
| `overlays` | `[]` | GUI overlays to supervise: `dps`, `session_report`, `friend`, `atlas`. |

Overlays start when `eqgame.exe` appears, are pointed at the character log the
client is currently writing, are restarted with backoff if they die, and are
stopped when the game exits or eqld shuts down. Switching character in-game
repoints them at the new log.

`atlas` is refused while `enabled = true`: the Atlas overlay autosaves its
database and would fight the concurrent `--replay`. Run one or the other.
