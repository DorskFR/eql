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
