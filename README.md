# SryzaMC

SryzaMC is an independent Rust Minecraft Java Edition server project. The current executable accepts Minecraft handshake connections and implements server-list status and ping for protocol 777 (26.3). Login and gameplay are not implemented yet, so players cannot join a world. No Bukkit, Spigot, or Paper plugin compatibility is claimed.

## Java plugin compatibility

Bukkit/Spigot/Paper plugins are JVM bytecode linked against large Java APIs and server internals. Rust cannot load these JARs as native libraries. Reaching the no-rewrite goal requires a supported JVM runtime with compatible API classes and event/state marshalling, plus a compatibility suite against real plugin versions. This project does not claim that compatibility today. Many plugins use CraftBukkit/Paper internals, reflection, native libraries, or version-specific behavior and may remain incompatible.

## Build and run

Install Rust stable (edition 2021), then run `cargo build --release --workspace`. Use `cargo test --workspace` and `cargo bench -p sryzamc --bench scheduler` when changing code. Rust/Cargo are not installed in the current environment, so these commands have not been verified here.

```text
sryzamc --dir ./server init
sryzamc --dir ./server info
sryzamc --dir ./server run
```

Status and ping work for protocol 777 (26.3); login is not implemented, so players cannot join yet. See [configuration reference](docs/CONFIGURATION.md) and [compatibility contract](docs/COMPATIBILITY.md).

## Layout

- `crates/sryza`: CLI, configuration, frame parser, tick clock, TCP listener.
- `config`: starter `server.properties` and complete operator configuration.
- `docs`: architecture, compatibility, operations, migration, roadmap.
- `.github/workflows`: formatting, lint, test, and release build.
- `benchmarks`: benchmark methodology; the current microbenchmark is not a Paper comparison.

AGPL-3.0-or-later; see `LICENSE`.
