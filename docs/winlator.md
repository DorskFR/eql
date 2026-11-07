# EverQuest on a phone, under Winlator

Winlator runs the Windows client on Android through Wine and Box64. eqld runs
there too, and everything the profile needs — inventory, log events, fights,
skins — is file-watching and HTTP, none of which cares that the CPU is ARM.

## Which binary

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
`[game] process = ""` applies (see [Finding the game](eqld.md#finding-the-game)): reading and uploading
work, writing the social and the skin do not.

## Starting it with the game

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

## What to switch on, and what it costs

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
