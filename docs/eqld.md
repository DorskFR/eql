# eqld reference

Everything the daemon reads from its TOML file, and what each part of it does on the game machine.
Skins are covered in [skins.md](skins.md), the log reader and its overlays in [log-reader.md](log-reader.md), and running on Android in [winlator.md](winlator.md).

## Configuration

```toml
[game]
root = 'C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest Legends'

[api]
url = "https://eqls.example.com"
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

Keys under `[tools.log_reader]`:

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
| `game.root` | required | The game folder, the one holding `eqgame.exe`. |
| `game.poll_secs` | `5` | Seconds between scans of the game folder. |
| `api.url`, `api.token` | required | The eqls base URL and its `EQLS_MACHINE_TOKEN`. |
| `state.path` | `<state dir>/eqld/state.json` | Where eqld remembers what it has already uploaded. |
| `game.process` | `eqgame.exe` | The client's name in the process list. Set it to `""` to say it cannot be seen at all. |
| `socials.bar` | `1` | Hotbar the EQLD button is placed on, 1-10. `0` installs the social but touches no bar. |
| `socials.page` | `1` | Page of that hotbar, 1-10. |
| `skin.enabled` | `false` | Keep the installed skin up to date from the API. |
| `skin.layout` | — | The layout to install; required when `skin.enabled` is on. |
| `skin.name` | layout's default | A named skin inside that layout, like `--skin` on the subcommand. |
| `skin.check_secs` | `300` | Seconds between bundle checks (min 30, or `0` for every tick). |
| `log.upload` | `true` | Ship eqld's own log to the server so this machine can be debugged remotely. |
| `log.device` | hostname | What the server files those logs under. |

## Toggling without a restart

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

## One eqld at a time

The daemon takes a lock file — `eqld.lock`, beside `state.json` — before it
starts, holding its own pid, and releases it on a clean exit. A second instance
reads that file, and if the pid is still alive and still eqld it refuses to
start:

```
another eqld is already running as pid 8124 (eqld.exe); stop it, or start with
--force to take C:\Users\you\AppData\Local\eqld\eqld.lock from it
```

It exits non-zero and uploads nothing, which is the point: two daemons on one
game folder upload every dump twice. A lock left behind by a crash or a hard
kill does not block anything — a pid that is gone, or that now belongs to some
other program, is stale and is taken over on the next start. `eqld <config>
--force` takes the lock unconditionally, for the case where the refusal is
wrong. This is a plain file, not a Windows named mutex, so it behaves the same
under Wine, Winlator, macOS and Termux.

The subcommands are one-shots and are not locked; only the daemon loop is.

## Log colour

```toml
[log]
colour = false   # `color` works too
```

Left out, colour is on unless `NO_COLOR` is set. Turn it off where the escape
codes are printed rather than acted on: a console reached through Wine, or a
log the daemon is redirected into. Read once at startup, so changing it needs a
restart.

## Reading a machine's log from the server

eqld keeps its own log in memory and ships it with the rest, so a daemon on a
phone inside a Winlator container can be debugged without holding the phone:

```toml
[log]
upload = true          # default
device = "phone"       # default: the hostname
```

Every run gets a session id, and lines are uploaded in numbered batches as they
appear. A batch the server refuses goes back on the front of the queue and is
retried under its own number, so a flaky link duplicates nothing. The buffer
holds 4000 lines and drops the **oldest** first; whatever it dropped is counted
and reported alongside the session, so a gap is always visible rather than
silent.

| Route | What |
|---|---|
| `GET /api/v1/devices` | Every machine that has checked in |
| `GET /api/v1/devices/{device}/sessions` | Its runs, newest first |
| `GET /api/v1/devices/{device}/sessions/{session}` | The lines, in order |

All three need the machine token, unlike the rest of the read API — a daemon log
names paths and config. The web UI reads them under **Devices**, using the token
the layout editor already keeps. Uploads are pruned after 14 days.

## Finding the game

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

## The in-game EQLD button

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

### …on a hotbar, not just in the socials list

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

## Item icons

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

## What eqld touches

eqld reads the files the client writes (inventory dumps, chat logs). In the game folder it writes only the skin folder under `uifiles/` and the per-character ini files, backing each up first: the same files `/loadskin` and the in-game socials window change. It also installs and runs the third-party log reader in that tool's own directory. It does not touch the client binary, its memory or its network traffic.

The repository contains no game art, textures or client binaries. Item icons only exist on a server after `upload-icons` sends them from a local install. The skin template (`crates/eqls/template/dorskui/` and its copy in `fixtures/equi/`) is a set of `EQUI_*.xml` window definitions edited from the client's stock UI files.
