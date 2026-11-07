# The log reader, overlays, fights and quests

eqld drives a patched build of the third-party EQL log reader to harvest lifetime DPS stats, fight history and quest progress. The `[tools.log_reader]` keys are listed in [eqld.md](eqld.md#configuration).

## Overlays

Overlays start when `eqgame.exe` appears, are pointed at the character log the
client is currently writing, are restarted with backoff if they die, and are
stopped when the game exits or eqld shuts down. Switching character in-game
repoints them at the new log.

## The patched log reader

Upstream's suite is five tkinter GUIs plus one headless entry point
(`eql_atlas --replay`). We carry a small additive patch — `vendor/eql-log-reader`,
applied in CI to upstream's pinned commit, published as assets on our own
releases — that adds three console tools: `eql_headless`, the DPS meter's
lifetime-stats layer with no window; `eql_quest_cli`, which curates the
tracked-quest list from the command line; and `eql_fights_cli`, which dumps a
log's completed fights as JSON. `install-tools` fetches that build by
default; `[tools.log_reader] repo` points back at upstream if you would rather
have stock.

The daemon installs it for you. With `enabled = true` (or any overlay listed)
and no reader on the machine, the first tick downloads the release asset and
runs it — the Inno installer on Windows, the tarball's `install.sh` elsewhere —
and then rewires replay, fights and the overlays without a restart. Unlike the
social and the skin this does **not** wait for the client to exit: the installer
only ever writes into its own directory, never the game root, and waiting would
mean a whole session harvested by nothing. A failed install is logged once with
the reason and retried on a backoff that doubles from one minute to one hour, so
a machine that is offline does not fill the console. `auto_install = false`
turns it off and goes back to warning once and doing nothing; `install-tools`,
with or without `--force`, is unchanged either way.

## Lifetime stats without a window on screen

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

## Fight history

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

## Quests

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
