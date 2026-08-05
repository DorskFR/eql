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
hidden = ["dps"]            # …of which these never appear on screen
```

| Key | Default | What |
|---|---|---|
| `enabled` | `false` | Ship the JSON the reader writes, and (in `atlas = "replay"`) run `eql_atlas --replay` each tick. |
| `exe` | discovered | Path to `eql_atlas`; the other tools are found next to it. |
| `version` | `v2.0` | Upstream release `install-tools` fetches. |
| `replay_secs` | `120` | Seconds between replays (min 10). |
| `replay_timeout_secs` | `600` | Kill a replay that outlives this (min 10). |
| `overlays` | `[]` | GUI overlays to supervise: `dps`, `session_report`, `friend`, `atlas`. |
| `hidden` | `[]` | Subset of `overlays` launched on an isolated desktop (Windows only). |
| `atlas` | `"replay"` | Who keeps the Atlas database: `"replay"` or `"overlay"`. |

### Item icons

```sh
eqld <config.toml> upload-icons [--force]
```

Ships every `<game.root>/uifiles/default/dragitem<n>.dds` sheet to
`PUT /api/v1/icons/sheets/<n>`, where the server crops the 36 40x40 icons out
of each one. Run it once per machine: the client's art never changes, so a
sheet already accepted is skipped on later runs unless `--force` is given. A
sheet the server refuses outright is parked (rerun with `--force` once the
cause is fixed); a transport or 5xx failure is logged and retried by the next
run, and neither stops the other sheets. This is a one-shot — the daemon loop
never touches icons.

Overlays start when `eqgame.exe` appears, are pointed at the character log the
client is currently writing, are restarted with backoff if they die, and are
stopped when the game exits or eqld shuts down. Switching character in-game
repoints them at the new log.

### Lifetime stats without a window on screen

An overlay listed in `hidden` is launched on a desktop eqld creates for itself,
so its window is never drawn while its parser ticks normally. Naming an overlay
that is not in `overlays` is refused, and so is hiding `atlas`. On anything but
Windows there are no desktops to hide behind: eqld warns once and launches it
like any other overlay.

This is what makes `alltime` (the DPS meter's per-build lifetime stats) accrue
at all — the meter only writes that file while it is running. **It never
backfills.** At startup it snapshots a baseline and counts only what it sees
live, so every minute the game runs without the meter is lost for good. Hide it
and leave it running.

A character with two class builds writes one `alltime` file per build. The API
keeps a single document per character and kind, so eqld merges them into one
`{"builds": {"WAR-CLR": …, "WAR-SHM": …}}` document. A file the reader wrote
before `/who` revealed the class combo has no build name and ships flat.

### Quests

`atlas = "replay"` (the default) runs the headless `--replay` tick and refuses
the `atlas` overlay: the overlay autosaves its database and the two would fight.
This mode writes **no quest data at all** — upstream's replay path never builds
any quest state.

`atlas = "overlay"` is the other way round: the replay tick is skipped entirely
and the Atlas overlay runs, keeping its own database live. This is the only way
to get quest progress, and it needs you: the Atlas only credits quests you added
by hand in its quest window, so the overlay must stay visible. eqld logs which
mode is active at startup.
