# eql

Self-updating Magelo-style profile for EverQuest Legends: a headless Rust
daemon on the game machine uploads inventory dumps and log events to a Rust
API, which renders equipment, derived stats, and spells — plus a
browser-based UI layout designer that deploys skins back into the game.

Tickets live in the YouTrack `EQL` project.

| Path | What |
|---|---|
| `crates/eql-core` | Shared types + parsers (inventory dumps, layouts) |
| `crates/eqld` | Daemon: watches game folder, uploads, installs skins and the in-game social |
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

[socials]
enabled = false             # keep the in-game EQLD button applied
# bar = 1                   # hotbar it is placed on (0 = leave the bars alone)
# page = 1

[skin]
enabled = false             # keep the installed skin up to date
layout = "dorskui"
export = false              # push the in-game arrangement back up as a layout
```

| Key | Default | What |
|---|---|---|
| `enabled` | `false` | Ship the JSON the reader writes, and (in `atlas = "replay"`) run `eql_atlas --replay` each tick. |
| `exe` | discovered | Path to `eql_atlas`; the other tools are found next to it. |
| `repo` | `DorskFR/eql` | GitHub repo the reader is fetched from. Set it to `blastlaster/eql-log-reader` for stock upstream. |
| `version` | `latest` | Release of `repo` to fetch; a tag, or `latest` for its newest release. |
| `auto_install` | `true` | Fetch and install the reader when it is wanted and missing, instead of only warning. |
| `replay_secs` | `120` | Seconds between replays (min 10). |
| `replay_timeout_secs` | `600` | Kill a replay that outlives this (min 10). |
| `overlays` | `[]` | Overlays to supervise: `dps`, `session_report`, `friend`, `atlas`. |
| `hidden` | `[]` | Subset of `overlays` to run with no window. |
| `atlas` | `"replay"` | Who keeps the Atlas database: `"replay"` or `"overlay"`. |

And outside `[tools.log_reader]`:

| Key | Default | What |
|---|---|---|
| `game.process` | `eqgame.exe` | The client's name in the process list. Set it to `""` to say it cannot be seen at all. |
| `socials.bar` | `1` | Hotbar the EQLD button is placed on, 1-10. `0` installs the social but touches no bar. |
| `socials.page` | `1` | Page of that hotbar, 1-10. |
| `skin.enabled` | `false` | Keep the installed skin up to date from the API. |
| `skin.layout` | — | The layout to install; required when `skin.enabled` is on. |
| `skin.name` | layout's default | A named skin inside that layout, like `--skin` on the subcommand. |
| `skin.check_secs` | `300` | Seconds between bundle checks (min 30, or `0` for every tick). |

### Toggling without a restart

eqld re-reads its config file on every poll tick and applies what it can while
it runs, so an overlay is switched on or off by editing `overlays` and saving —
no restart, no touching the scheduled task. Each change is logged
(`overlay dps enabled`, `overlay friend disabled`), and an overlay that is still
listed and still healthy is left alone rather than restarted.

| Hot | Restart required |
|---|---|
| everything under `[tools.log_reader]`: `enabled`, `exe`, `repo`, `version`, `auto_install`, `replay_secs`, `replay_timeout_secs`, `overlays`, `hidden`, `atlas` | `game.root` |
| everything under `[harvest]`: `enabled`, `dir` | `game.poll_secs` |
| everything under `[socials]`: `enabled`, `bar`, `page` | |
| everything under `[skin]` | |
| `game.process` | |
| | `api.url`, `api.token` |
| | `state.path` |

A restart-only field that changed is logged as such and the running value is
kept, so nothing is applied by halves. A config file that does not parse is
logged once and ignored: the daemon keeps running on the last one that worked,
and picks up the next edit that fixes it.

### One eqld at a time

The daemon takes a lock file — `eqld.lock`, beside `state.json` — before it
starts, holding its own pid, and releases it on a clean exit. A second instance
reads that file, and if the pid is still alive and still eqld it refuses to
start:

```
another eqld is already running as pid 8124 (eqld.exe); stop it, or start with
--force to take C:\Users\dorsk\AppData\Local\eqld\eqld.lock from it
```

It exits non-zero and uploads nothing, which is the point: two daemons on one
game folder upload every dump twice. A lock left behind by a crash or a hard
kill does not block anything — a pid that is gone, or that now belongs to some
other program, is stale and is taken over on the next start. `eqld <config>
--force` takes the lock unconditionally, for the case where the refusal is
wrong. This is a plain file, not a Windows named mutex, so it behaves the same
under Wine, Winlator, macOS and Termux.

The subcommands are one-shots and are not locked; only the daemon loop is.

### Keeping the skin installed

```toml
[skin]
enabled = true
layout = "dorskui"
# name = "v4"
export = true               # also send the in-game arrangement back
# screen = [1280, 720]      # only if eqclient.ini cannot be trusted
```

### Channels

A channel is one UI across every machine, with a variant per resolution:

```toml
[skin]
enabled = true
channel = "dorskui"         # instead of `layout`
export  = true
```

Layouts named `dorskui@3840x2160`, `dorskui@1440x1050`, `dorskui@1280x720` are
variants of the `dorskui` channel. eqld reads the render size from
`eqclient.ini`, picks the variant, and installs it as `uifiles/dorskui/` — the
skin folder is named for the channel, not the variant, so `/loadskin dorskui`
is the same command on the PC, the MacBook and the phone.

No exact match falls back to the nearest variant by **aspect first, then area**.
Positions are percentages and reflow correctly within a shape; a variant of a
different shape does not, however close its pixel count.

A variant owns its `style` as well as its rects, because window geometry cannot
express content scale:

| Key | What |
|-----|------|
| `font_shift` | Added to every `<Font>` tag, clamped to the 1..=5 the client ships |
| `gem` | Spell gem cell in px; the 40/64 icon ratio is kept |

That matters more than resolution: 1280x720 on a 6.8" phone wants the *large*
fonts, the same 1280x720 on a laptop wants small ones. It is DPI, not pixels,
so the variant states it rather than deriving it.

`layout` still works for a single fixed layout; `channel` supersedes it.

With `export = true` each tick reads `UI_<Character>_<server>_LO1.ini`, hashes
the geometry of the tracked windows only — chat routing and bag positions churn
constantly and must not count — and uploads on change. In
channel mode it writes back to `<channel>@<W>x<H>`, so a machine edits its own
variant and cannot disturb another's; otherwise it names the upload
`<character>-<server>-<W>x<H>-<YYYYmmdd-HHMMSS>`. No change, no upload. The
render size comes from `eqclient.ini` (`WindowedWidth`/`WindowedHeight`, else
`Width`/`Height`); `screen` overrides it.

`uifiles/<skin>/` is ours and is only read at `/loadskin`, so window **sizes**
are written whatever the client is doing. The `UI_*_LO1.ini` carries the
**positions**, belongs to the client, and is rewritten by it on exit — so that
one still waits for the client to be gone, and eqld remembers it is owed. With
the client up the skin folder is refreshed in place rather than renamed aside,
because renaming a directory it holds open fails on Windows.

Export only reads, so it runs whether or not the client is up. The client
rewrites that ini as it exits, which is the edge the hash is waiting for — so a
session's rearranging lands one tick after you quit. A skin install is adopted
as already-seen, so installing does not bounce straight back up as a new
layout.

`install-skin` is a one-shot you run by hand. `[skin] enabled = true` puts the
same work on the daemon's tick, which matters where typing a command line is
painful. Each check fetches the layout's bundle, hashes it, and installs it only
if that hash is not the one recorded in `state.json` — so an unchanged skin is
never written over the client's files, and a redesign in the web designer lands
by itself within `check_secs`. Changing `layout` or `name` also counts as a
change, and both are picked up without restarting the daemon.

**The game must be closed**, for the same reason the social installer waits:
`<root>/uifiles/` and the `UI_*_LO1.ini` belong to the client, which rewrites
them when it exits. eqld says so once and keeps checking; the moment the game is
gone the new skin lands, and the log tells you to go and apply it:

```
a new skin is installed; run in game: /loadskin dorskui
```

The client does not reload a skin by itself, so that `/loadskin` is still on
you. The previous skin directory and any ini replaced are backed up beside
themselves first, exactly as the subcommand does.

### Log colour

```toml
[log]
colour = false   # `color` works too
```

Left out, colour is on unless `NO_COLOR` is set. Turn it off where the escape
codes are printed rather than acted on: a console reached through Wine, or a
log the daemon is redirected into. Read once at startup, so changing it needs a
restart.

### Finding the game

`game.process` is the name eqld looks for in the process list to decide whether
the client is running. It defaults to `eqgame.exe`, which is what the client
reports natively and inside a Wine prefix. Rename it if your setup shows
something else.

Three things hang off that answer: overlays start and stop with the game, and
the social installer and skin sync only write while it is gone. If the client
cannot be seen under any name — which is possible under Winlator, where the
container may not expose its processes to whatever eqld can see — set:

```toml
[game]
process = ""
```

That does not mean "always closed". It means eqld does not know, and the safe
answer to not knowing is to never write into the game root behind the client's
back: `[socials] enabled` and `[skin] enabled` become no-ops that log why, and
no overlay is ever launched. Inventory dumps, log events, harvest and fights all
still upload — those only read. Run `install-social` and `install-skin` by hand,
with the game closed, when you want those files changed; both still work, and
`install-social` warns that it could not verify the game was shut.

### The in-game EQLD button

```sh
eqld <config.toml> install-social
```

Writes one social named `EQLD` into every character's
`<game.root>/<Character>_<server>_LO1.ini`, so everything the daemon reads is
one click away in game:

```
/log on
/who
/outputfile inventory
/outputfile spellbook
/outputfile missingspells
```

That is a social's whole capacity — the client's own `EQUI_SocialEditWnd.xml`
defines five lines — and `/log on` has to lead, because with logging off none of
the rest is observable. **The game must be closed**: the client owns that file
and rewrites it when it exits, so anything written mid-session is thrown away.
eqld refuses to write while `eqgame.exe` is running.

An existing social named `EQLD` is updated where it is, keeping the colour you
picked and any hotbutton already bound to that slot; otherwise the first slot
that holds nothing is claimed. A social you named something else is never
touched, every other section of the file is carried over byte for byte, and the
file is copied to `<name>.eqld.bak` before a write. A character in
`_characters.ini` who has never logged in has no ini yet and is reported as
such.

#### …on a hotbar, not just in the socials list

A social in `[Socials]` is not on screen: you still have to open the socials
window and drag it onto a bar, which on a phone with no mouse is the thing the
button exists to avoid. So eqld also writes the hotbutton, into
`[HotButtons]` — bar 1, page 1 by default:

```
[HotButtons]
Page1Button5=E12,@-1,0000000000000000,0,EQLD,
```

The client serialises a hotbutton as `%c%d,%c%d,%s,%d,%s,%s`: type and slot,
icon type and slot, item guid, item id, label, item name. The type character is
`'A' + type`, so a social (type 4) is `E` and `@-1` is "no custom icon". The
slot is the social's **zero-based** index over the 12-per-page grid, so
`Page2Button1` in `[Socials]` is `(2-1)*12 + (1-1) = 12` → `E12`. From 120 up
that same field means an alternate advancement ability instead — which is what
the unlabelled `E451` and `E6120` entries in a real ini are — and the 10x12
social grid tops out at 119, so the two can never collide.

Bars are `[HotButtons]` for bar 1 and `[HotButtons2]`…`[HotButtons10]` for the
rest; each has ten pages of twelve buttons. Point the button somewhere else
with:

```toml
[socials]
enabled = true
bar = 4
page = 1
```

A button that already holds something else is never taken — the first *empty*
button of that page is claimed, and if all twelve are full eqld says so and
writes nothing rather than clobbering one. An EQLD button you have already
placed is found on **any** bar and left where it is, so moving it in game
sticks; if the social itself moves slot, the index on that button is corrected
rather than a second button being added. `bar = 0` installs the social and
leaves every hotbar alone.

`[socials] enabled = true` puts the same work on the daemon's tick, which is
what makes it stick: the client rewrites the file on every exit, and the daemon
re-applies it the moment the game is gone. It is off by default — this edits a
file you own.

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

## EverQuest on a phone, under Winlator

Winlator runs the Windows client on Android through Wine and Box64. eqld runs
there too, and everything the profile needs — inventory, log events, fights,
skins — is file-watching and HTTP, none of which cares that the CPU is ARM.

### Which binary

Releases ship `eqld-windows-x86_64.exe`, `eqld-macos-aarch64`,
`eqld-linux-x86_64` and `eqld-linux-aarch64`. The two Linux ones are static musl
builds: no glibc, no shared libraries, drop them anywhere and run them.

**Inside the container** (the safe choice): use the Windows exe. It is another
Windows process next to `eqgame.exe`, it sees the same drive letters and the
same `C:\...\EverQuest Legends` path the client does, and there is nothing to
work out about paths. It is emulated, so it costs a little CPU; for a poll loop
and a few HTTP posts that is not much.

**Natively under Termux** (lighter, conditional): use `eqld-linux-aarch64`. No
emulation at all. It only works if Termux can *reach the game files*, and that
depends on where Winlator keeps its container. Check before committing to it:

```sh
ls ~/storage/shared/Winlator          # Termux: termux-setup-storage first
find /sdcard -maxdepth 6 -name eqgame.exe 2>/dev/null
```

If that finds the client, point `game.root` at the directory holding it and you
are done. If it finds nothing, the container lives in Winlator's app-private
storage (`/data/data/com.winlator/...`), which is unreadable from Termux without
root — there is no config to fix that, so run the exe inside the container
instead. Note also that Termux cannot see processes inside the container, so
`[game] process = ""` applies (see *Finding the game*): reading and uploading
work, writing the social and the skin do not.

### Starting it with the game

Winlator has no Task Scheduler. A `.bat` in the container that starts eqld and
then the game is the whole mechanism — point the Winlator shortcut at it instead
of at `eqgame.exe`:

```bat
@echo off
cd /d "C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest Legends"
start "" /min eqld.exe eqld.toml
eqgame.exe patchme
```

`start /min` returns immediately and leaves eqld running; the `.bat` then blocks
on `eqgame.exe`, so the window closes when you quit the game. eqld keeps running
after that — which is what you want, since the social and the skin are applied
*after* the client exits. Launching the shortcut twice is safe: the second eqld
sees the lock, prints the pid holding it, and exits.

### What to switch on, and what it costs

On a phone the defaults are already close to right.

| Setting | Effect |
|---|---|
| `overlays = []` (the default) | No overlay windows. On a small screen with no mouse, anything else is in the way. |
| `[tools.log_reader] enabled = false` | The lightest footprint there is: no `eql_atlas --replay` process is ever spawned, and nothing is harvested. You keep inventory, spellbook, `/who` and log events — the profile page stays current. You lose lifetime DPS stats, quest progress and fight history. |
| `[tools.log_reader] enabled = true` | The replay process runs once per `replay_secs`, for a few seconds. That is the whole cost, and it buys fights, quests and `alltime` stats. |
| `replay_secs = 600` | The middle road: same features, one tenth as many wake-ups as the 120s default. Nothing is lost — a replay resumes from its saved byte offset, so a long gap just means a longer catch-up. |
| `poll_secs = 30` | Fewer directory scans. Uploads are that much less prompt, which for a profile page nobody is refreshing costs nothing. |

The EQLD social button is worth more here than anywhere else: `/log on`, `/who`
and the three `/outputfile` dumps are five commands you would otherwise type on
a touch keyboard, and it is one tap. It needs `[socials] enabled = true` and a
client whose process eqld can see, since the ini is only writable while the game
is closed.

Skin sync is the other one. Design the layout in the browser on a desktop, set
`[skin] enabled = true` and `layout` on the phone, and the next time you close
the game eqld installs it; `/loadskin` in game and the small-screen layout is
there. No file transfer, no CLI.
