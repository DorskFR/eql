# eql

Self-updating EverQuest Legends character profile: a daemon on the game PC feeds a web page of gear, stats and fights.

[![release](https://img.shields.io/github/v/release/DorskFR/eql)](https://github.com/DorskFR/eql/releases)
[![CI](https://github.com/DorskFR/eql/actions/workflows/ci.yml/badge.svg)](https://github.com/DorskFR/eql/actions/workflows/ci.yml)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/profile-dark.png">
  <img alt="eql profile page for a level 42 Shadow Knight: equipped items in a slot grid, vitals, attributes and resists summed from the gear, and tabs for best in slot, bags, bank, events, fights, atlas, quests and lifetime stats" src="docs/assets/profile-light.png" width="900">
</picture>

^ A profile page served by a local eqls with fake seeded data, regenerated with `scripts/readme-screenshots.mjs`.

eql is a fan tool in the spirit of Magelo profiles, for EverQuest Legends. `eqld` runs next to the game client, watches the game folder for the dumps the client writes (`/outputfile inventory`, the chat log, the log reader's JSON) and uploads them. `eqls` stores them in Postgres, looks the items up in a database scraped from the fan wiki, and serves a web UI that shows equipment, derived stats, recent events, fight history and quest progress, plus a layout designer whose skins eqld installs back into the game.

eql is an unofficial fan project, not affiliated with or endorsed by Daybreak Game Company; see [Trademarks](#license-and-trademarks).

## Features

- Profile page per character: equipment grid, HP/mana/AC/haste and attributes summed from item stats, resists, best-in-slot suggestions, bags and bank.
- Events from the chat log (`/who`, zone, loot, level, death) and per-fight history with damage, healing and per-ability breakdowns.
- Browser layout designer for the game's UI windows, with presets and per-resolution variants; eqld installs the result as a skin when the game is closed.
- An in-game `EQLD` social button that runs `/log on`, `/who` and the three `/outputfile` dumps in one click.
- Daemon builds for Windows, macOS (Apple silicon) and static Linux x86_64/aarch64; also runs under Wine and Winlator on Android.

## Quick start

### 1. Run the server

```yaml
# compose.yaml
services:
  eqls:
    image: ghcr.io/dorskfr/eqls:0.24.3
    ports: ["8080:8080"]
    environment:
      DATABASE_URL: postgres://eql:eql@db/eql
      EQLS_MACHINE_TOKEN: change-me
    depends_on: [db]
  db:
    image: pgautoupgrade/pgautoupgrade:18-alpine
    environment:
      POSTGRES_USER: eql
      POSTGRES_PASSWORD: eql
      POSTGRES_DB: eql
    volumes: ["./data:/var/lib/postgresql"]
```

```sh
docker compose up -d
curl http://localhost:8080/api/v1/version
```

```
{"version":"0.24.3"}
```

Open http://localhost:8080. There are no accounts: pages and the read API are open to anyone who can reach the server, and every write needs `EQLS_MACHINE_TOKEN`. The data is in `./data`; back up that folder.

To give items their stats, fill the item database from the fan wiki once (it re-fetches every item page, so it takes a while):

```sh
docker compose exec eqls eqls scrape
```

### 2. Run the daemon on the game machine

Download the binary for your platform from the [latest release](https://github.com/DorskFR/eql/releases/latest) (`eqld-windows-x86_64.exe`, `eqld-macos-aarch64`, `eqld-linux-x86_64`, `eqld-linux-aarch64`) and write an `eqld.toml` next to it:

```toml
[game]
root = 'C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest Legends'

[api]
url = "http://your-server:8080"
token = "change-me"
```

```sh
eqld eqld.toml
```

In game, type `/outputfile inventory`. eqld picks up the dump on its next scan:

```
INFO eqld::daemon: uploading inventory character=Ashvale server=erudin entries=2
INFO eqld::daemon: uploaded inventory character=Ashvale server=erudin entries=2 status=201
```

The character is now listed on the home page and at `/c/<server>/<name>`.

## Usage

### Put the dumps on one button

```sh
eqld eqld.toml install-social
```

With the game closed, this writes a social named `EQLD` (`/log on`, `/who`, `/outputfile inventory`, `/outputfile spellbook`, `/outputfile missingspells`) into every character's ini and places it on hotbar 1. Set `[socials] enabled = true` to have the daemon re-apply it after each session. See [the in-game EQLD button](docs/eqld.md#the-in-game-eqld-button).

### Design a UI layout and install it

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/designer-dark.png">
  <img alt="eql layout designer: the game's UI windows (buff, player, group, target, chat, hotbars) drawn as draggable boxes on a scaled 3840x2160 screen, with save, download bundle and copy /loadskin buttons" src="docs/assets/designer-light.png" width="900">
</picture>

Under **Layouts**, click **Clone default**, name it `myui`, and drag the windows into place (creating and saving need the machine token). Then point the daemon at it:

```toml
[skin]
enabled = true
layout = "myui"
```

The next time the game is closed, eqld installs it as `uifiles/myui/`; type `/loadskin myui` in game. Channels, per-resolution variants, presets and sending in-game changes back are in [docs/skins.md](docs/skins.md).

### Track fights, quests and lifetime DPS

```toml
[tools.log_reader]
enabled = true
overlays = ["dps"]
hidden = ["dps"]
```

eqld downloads and installs a patched build of the third-party EQL log reader, runs its replay every two minutes, and uploads fights, quest progress and the DPS meter's lifetime stats. See [docs/log-reader.md](docs/log-reader.md).

### Show item icons

```sh
eqld eqld.toml upload-icons
```

The server does not ship game art. This uploads the icon sheets from your own install once; the server crops them into the 40x40 icons the UI shows.

## Configuration

The keys most setups touch. Every key, hot reload, the single-instance lock and remote log reading are in [docs/eqld.md](docs/eqld.md).

| eqld key | Default | Effect |
|---|---|---|
| `game.root` | required | The game folder, the one holding `eqgame.exe` |
| `api.url`, `api.token` | required | eqls base URL and its machine token |
| `game.poll_secs` | `5` | Seconds between scans of the game folder |
| `socials.enabled` | `false` | Keep the `EQLD` button applied after each session |
| `skin.enabled`, `skin.layout` | `false`, none | Keep a layout from the server installed |
| `tools.log_reader.enabled` | `false` | Run the log reader and upload fights, quests and lifetime stats |
| `log.upload` | `true` | Send eqld's own log to the server, readable under **Devices** |

| eqls variable | Default | Effect |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string; migrations run at startup |
| `EQLS_MACHINE_TOKEN` | required | Bearer token for every write |
| `LISTEN_ADDR` | `0.0.0.0:8080` | Bind address |

## How to update

```sh
docker compose pull && docker compose up -d
```

Change the image tag in `compose.yaml` to the new release first. eqld does not update itself: replace the binary with the one from the same release.

## Limitations

- Item stats come from the fan wiki; an item the wiki lacks shows on the profile without stats.
- HP and mana are estimates from level, class and attributes; points allocated at character creation are not in the dumps.
- The social and the skin are only written while the game is closed, and `/loadskin` is still typed by hand.
- No user accounts: one machine token per server, and every profile is public to whoever can reach it.
- Phones: see [docs/winlator.md](docs/winlator.md) for running the client and eqld under Winlator.

## Development

```sh
git clone https://github.com/DorskFR/eql && cd eql
cargo test --workspace
(cd web && npm ci && npm test)
```

Crate layout, the full CI checks, running eqls from source and regenerating the screenshots are in [docs/development.md](docs/development.md).

## License and trademarks

MIT, as declared in `Cargo.toml`. The repository does not include a LICENSE file yet.

EverQuest is a registered trademark, and EverQuest Legends a trademark, of Daybreak Game Company LLC in the United States and/or other countries. eql is an unofficial fan tool; it is not affiliated with, endorsed by or sponsored by Daybreak Game Company LLC.

The repository contains no game art, textures or client binaries. Item icons only exist on a server after `upload-icons` sends them from your own install. The skin template (`crates/eqls/template/dorskui/` and its copy in `fixtures/equi/`) is a set of `EQUI_*.xml` window definitions edited from the client's stock UI files.

eqld reads the files the client writes (inventory dumps, chat logs). In the game folder it writes only the skin folder under `uifiles/` and the per-character ini files, backing each up first: the same files `/loadskin` and the in-game socials window change. It also installs and runs the third-party log reader in that tool's own directory. It does not touch the client binary, its memory or its network traffic. Using any third-party tool with the game is subject to Daybreak's [Terms of Service](https://www.daybreakgames.com/terms-of-service).
