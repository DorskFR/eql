# Development

## Layout

| Path | What |
|---|---|
| `crates/eql-core` | Shared types and parsers (inventory dumps, layouts) |
| `crates/eqld` | Daemon: watches the game folder, uploads, installs skins and the in-game social |
| `crates/eqls` | Server: ingest API, item DB, stat engine, serves the web UI |
| `web/` | SvelteKit UI (profile pages, layout designer, device logs), built to `web/build` |
| `fixtures/` | Sample game artifacts (skin XMLs, layout, ini, wiki pages, harvest and fight dumps) used by tests |
| `vendor/eql-log-reader` | The patch applied to the upstream log reader, and the commit it applies to |

## Checks

These are what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd web && npm ci && npm run check && npm test && npm run build)
vendor/eql-log-reader/apply.sh /tmp/eql-log-reader
```

## Running eqls locally

eqls needs Postgres and two environment variables:

| Variable | Default | Effect |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string; migrations run at startup |
| `EQLS_MACHINE_TOKEN` | required | Bearer token every write (and the device log reads) must carry |
| `LISTEN_ADDR` | `0.0.0.0:8080` | Address the HTTP server binds |
| `WEB_DIST` | `web/build` | Directory of the built web UI |
| `RUST_LOG` | `info` | Log filter |

Besides serving, the binary has three one-shot subcommands that fill the item database:

```sh
eqls scrape [--limit N] [--page <title>]   # item pages from the fan wiki
eqls reparse                               # re-run the wiki parser over stored pages
eqls item-dump [--url <url>] [--file <path>]  # icon ids from the quarmdb SQL dump
```

## README screenshots

`scripts/readme-demo.sh up` starts a throwaway Postgres in Docker on port 5410, builds and runs eqls on
port 5411, and seeds one fake character (items, inventory, events, harvests, fights and a layout).
`scripts/readme-screenshots.mjs` then writes `docs/assets/{profile,designer}-{light,dark}.png`:

```sh
scripts/readme-demo.sh up
NODE_PATH=/path/to/node_modules node scripts/readme-screenshots.mjs
scripts/readme-demo.sh down
```

`NODE_PATH` points at any install of `playwright` 1.63 with Chromium downloaded.
