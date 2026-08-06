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
| `vendor/eql-log-reader` | Our patch to upstream's log reader, and the commit it applies to |

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
hidden = ["dps"]            # …of which these run without a window
```

| Key | Default | What |
|---|---|---|
| `enabled` | `false` | Ship the JSON the reader writes, and (in `atlas = "replay"`) run `eql_atlas --replay` each tick. |
| `exe` | discovered | Path to `eql_atlas`; the other tools are found next to it. |
| `repo` | `DorskFR/eql` | GitHub repo `install-tools` fetches the reader from. Set it to `blastlaster/eql-log-reader` for stock upstream. |
| `version` | `latest` | Release of `repo` to fetch; a tag, or `latest` for its newest release. |
| `replay_secs` | `120` | Seconds between replays (min 10). |
| `replay_timeout_secs` | `600` | Kill a replay that outlives this (min 10). |
| `overlays` | `[]` | Overlays to supervise: `dps`, `session_report`, `friend`, `atlas`. |
| `hidden` | `[]` | Subset of `overlays` to run with no window. |
| `atlas` | `"replay"` | Who keeps the Atlas database: `"replay"` or `"overlay"`. |

### Toggling without a restart

eqld re-reads its config file on every poll tick and applies what it can while
it runs, so an overlay is switched on or off by editing `overlays` and saving —
no restart, no touching the scheduled task. Each change is logged
(`overlay dps enabled`, `overlay friend disabled`), and an overlay that is still
listed and still healthy is left alone rather than restarted.

| Hot | Restart required |
|---|---|
| everything under `[tools.log_reader]`: `enabled`, `exe`, `repo`, `version`, `replay_secs`, `replay_timeout_secs`, `overlays`, `hidden`, `atlas` | `game.root` |
| everything under `[harvest]`: `enabled`, `dir` | `game.poll_secs` |
| | `api.url`, `api.token` |
| | `state.path` |

A restart-only field that changed is logged as such and the running value is
kept, so nothing is applied by halves. A config file that does not parse is
logged once and ignored: the daemon keeps running on the last one that worked,
and picks up the next edit that fixes it.

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

### The patched log reader

Upstream's suite is five tkinter GUIs plus one headless entry point
(`eql_atlas --replay`). We carry a small additive patch — `vendor/eql-log-reader`,
applied in CI to upstream's pinned commit, published as assets on our own
releases — that adds three console tools: `eql_headless`, the DPS meter's
lifetime-stats layer with no window; `eql_quest_cli`, which curates the
tracked-quest list from the command line; and `eql_fights_cli`, which dumps a
log's completed fights as JSON. `install-tools` fetches that build by
default; `[tools.log_reader] repo` points back at upstream if you would rather
have stock.

### Lifetime stats without a window on screen

An overlay listed in `hidden` runs with no window. For `dps` that means eqld
launches `eql_headless` instead of the meter — a real console process, no
window ever created, on every platform. The other overlays have no headless
build, so eqld falls back to launching them on a desktop it creates for itself
and that nothing renders; that trick is Windows-only, and elsewhere eqld warns
once and launches them normally. A stock upstream install with no
`eql_headless` next to `eql_atlas` falls back the same way. Naming an overlay
that is not in `overlays` is refused, and so is hiding `atlas`.

This is what makes `alltime` (the DPS meter's per-build lifetime stats) accrue
at all — the meter only writes that file while it is running. **It never
backfills.** At startup it snapshots a baseline and counts only what it sees
live, so every minute the game runs without the meter is lost for good. Hide it
and leave it running.

Windows delivers no SIGTERM, so eqld starts `eql_headless` in its own process
group and raises `CTRL_BREAK_EVENT` on it at shutdown, which runs its final
save. If that cannot be delivered the store still autosaves every 15 seconds,
so a hard kill costs at most that much.

A character with two class builds writes one `alltime` file per build. The API
keeps a single document per character and kind, so eqld merges them into one
`{"builds": {"WAR-CLR": …, "WAR-SHM": …}}` document. A file the reader wrote
before `/who` revealed the class combo has no build name and ships flat.

### Fight history

On the same beat as the replay, eqld runs `eql_fights_cli` over every character
log and posts what it finds to `POST /api/v1/fights` as
`{"character", "server", "fights": [...]}`. A fight is upstream's own
`CombatTracker` encounter — start, duration, active seconds, enemies, allies,
damage out and in, healing, kills, deaths, stance, invocation, per-ability
damage, casts and resists — plus the zone it started in, interleaved from the
log's `You have entered <zone>.` lines because the tracker does not record one.

Fights are history, so they accumulate rather than replace: the server keys them
on `(character, server, start_wall)` and ignores one it already has. eqld keeps
the newest `start_wall` it has had accepted in `state.json`, asks the tool only
for what came after it, and filters again before posting, so a tick that fails
replays exactly the same batch and a tick with nothing new posts nothing at all.
The dumps are staged beside the state file, never in the log reader's own
directory — that one is uploaded wholesale as harvest documents.

A fight still running when the log ends is not shipped until it closes: the same
fight would otherwise arrive twice under one `start_wall`, and the first,
half-finished version would be the one that stuck.

### Quests

`atlas = "replay"` (the default) runs the headless `--replay` tick and refuses
the `atlas` overlay: the overlay autosaves its database and the two would fight.
With the patched reader this mode **does** keep quest progress — the replay
credits looted items against the tracked quests and records hand-in dialogue,
resuming from the same persisted byte offset the overlay uses, so nothing is
credited twice.

What no amount of patching gets you is *which* quests you are doing: the log
records that you looted two Crushbone Belts, never that you mean to hand them
in. Naming them is a one-off:

```sh
eql_atlas --quest <eqlog file> add "crushbone belt" --pick 1
eql_atlas --quest <eqlog file> list
```

Until the first `add`, the tracked list is empty, upstream's save returns early
and **no `eql_quest_*.json` exists at all**. That is the normal starting state,
not a failure: eqld harvests whatever files are there and says nothing about
the ones that are not.

`atlas = "overlay"` is the other way round: the replay tick is skipped entirely
and the Atlas overlay runs visibly, keeping its own database and giving you the
quest window to curate in. eqld logs which mode is active at startup.
