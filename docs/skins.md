# Skins and layouts

How eqld installs a layout designed in the web UI, how one UI is shared across resolutions, and how in-game changes are sent back. The `[skin]` keys are listed in [eqld.md](eqld.md#configuration).

## Keeping the skin installed

```toml
[skin]
enabled = true
layout = "dorskui"
# name = "v4"
export = true               # also send the in-game arrangement back
# screen = [1280, 720]      # only if eqclient.ini cannot be trusted
```

## Channels

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
| `hidden` | Ini sections written `Show=0` |
| `bare` | Ini sections written `BGType=0` and `Border=0` |

That matters more than resolution: 1280x720 on a 6.8" phone wants the *large*
fonts, the same 1280x720 on a laptop wants small ones. It is DPI, not pixels,
so the variant states it rather than deriving it.

## Hiding windows, and losing their frames

`hidden` and `bare` name **ini sections**, not layout windows, so they reach the
panels the skin never positions — `EQMainWnd`, `StanceWnd`, `MapViewWnd`,
`HotButtonWnd3`..`11`. That is how a phone whose toolbars are replaced by
Winlator's on-screen controls gets rid of them for good: the client rewrites the
ini on exit, and the next install asserts the layout again.

`bare` is what the client's own right-click "Background: none" writes, so a
window keeps its text and loses its frame. Buffs and chat over the world, no box.

A section that the template ini does not carry — or carries without the
`Show=`/`BGType=` line the change needs — is **refused** when the bundle is
built, rather than silently doing nothing. `OverseerWnd` has no `Show=`, so
asking to hide it is an error, not a no-op.

## Presets

```
GET  /api/v1/layout-presets
POST /api/v1/layouts/{name}/clone/{preset}
```

| Preset | Screen | What |
|--------|--------|------|
| `default` | 3840x2160 | The dorskui template, all 13 windows |
| `light-16x9` | 1600x900 | Buffs and songs across the top, chat along the bottom, both transparent; toolbars hidden |
| `light-16x10` | 1440x900 | The same, refitted for the narrower shape |

The light presets keep player, target, group, pet, one hotbar and the spell gems
and hide the rest, which leaves the middle of the screen empty to play in. Build
a channel out of them in two calls:

```sh
curl -X POST -H "authorization: Bearer $TOKEN" \
  $EQLS_URL/api/v1/layouts/light%401600x900/clone/light-16x9
curl -X POST -H "authorization: Bearer $TOKEN" \
  $EQLS_URL/api/v1/layouts/light%401440x900/clone/light-16x10
```

Then point the machine at `channel = "light"` and `/loadskin light`.

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
