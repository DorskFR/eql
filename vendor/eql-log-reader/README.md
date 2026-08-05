# Vendored patch: headless `eql-log-reader`

`eql-log-reader` is [blastlaster/eql-log-reader](https://github.com/blastlaster/eql-log-reader),
MIT licensed; `LICENSE` here is upstream's, copyright preserved. Nothing of
upstream's source is copied into this repo — only `headless.patch`, which is
ours and applies on top of the commit pinned in `upstream.env`.

There is no fork repository. CI checks upstream out at the pinned commit,
applies the patch and publishes the built tools as assets on **this** repo's
release, which is what `eqld install-tools` fetches by default.

## What the patch adds

Upstream's only headless entry point is `eql_atlas --replay`; the DPS meter's
lifetime stats and quest credit both needed its tkinter GUI on screen.

| File | |
|---|---|
| `eql_headless.py` | new — the DPS meter's `open_log`/`poll`/`quit_app` driven by a plain loop, no tkinter. Writes the same `eql_alltime_*.json`. |
| `eql_quest_cli.py` | new — `list \| search \| add \| track \| remove \| move \| have` against the tracked-quest list. |
| `eql_dps_meter.py` | 5 lines: `--headless` dispatches to `eql_headless`. |
| `eql_atlas.py` | `attach_quest_layer()`, called from `replay()`, plus a `--quest` verb. |
| `eql_suite.spec` | builds the two new tools. `console=True` for both — a windowed PyInstaller exe has no valid stdout on Windows, so a headless tool built that way is mute. `hiddenimports` because both are reached only through in-function imports. |
| `packaging/linux/*` | the two new files in the runtime file lists. |
| `tools/compare_*.sh` | the harnesses that proved headless output equals the GUI's. |

`eql_headless.py` mirrors `run_overlay`'s `open_log` / `poll` / `quit_app` and
names them with their upstream line numbers in its docstring; those three
functions are what to re-read on every upstream bump.

## Applying

```sh
vendor/eql-log-reader/apply.sh <checkout-dir> [remote]
```

Clones the pinned tag if `<checkout-dir>` is not already a checkout of the
pinned commit, applies the patch, and fails if either step does not come out
clean. `cargo test -p eqld` asserts the patch is well formed and lists the
files it touches; the release workflow runs the script itself.

## Regenerating after an upstream release

```sh
git clone https://github.com/blastlaster/eql-log-reader fork && cd fork
git checkout -b eqld <new tag>
git apply /path/to/vendor/eql-log-reader/headless.patch   # fix up conflicts
git add -A && git commit -m headless
git diff --binary <new tag> > /path/to/vendor/eql-log-reader/headless.patch
```

Then update `upstream.env` and `[tools.log_reader] version` in the eqld config.
Never squash the patch into a vendored copy of upstream: it stays a diff so it
stays rebasable.
