# SryzaMC

SryzaMC is an independent Rust Minecraft Java Edition server project. The code implements status/ping and protocol 777 (26.3) Login in online and offline modes, including online Mojang session verification, encryption, compression, known-pack negotiation, vanilla feature flags, tags, a validated Configuration Keep Alive, and the Configuration acknowledgement. After Configuration it currently closes the connection because Play/world spawning is not implemented. No Bukkit, Spigot, or Paper plugin compatibility is claimed.

## Java plugin compatibility

Bukkit/Spigot/Paper plugins are JVM bytecode linked against large Java APIs and server internals. Rust cannot load these JARs as native libraries. Reaching the no-rewrite goal requires a supported JVM runtime with compatible API classes and event/state marshalling, plus a compatibility suite against real plugin versions. This project does not claim that compatibility today. Many plugins use CraftBukkit/Paper internals, reflection, native libraries, or version-specific behavior and may remain incompatible.

## Build and run

See [build requirements](docs/BUILDING.md) for Rust/MSVC setup and the future JVM bridge baseline. Windows MSVC compilation and the current test suite pass. No real Minecraft client verification has been performed.

The release build includes a Java 25 launcher JAR for Pterodactyl; see [Pterodactyl deployment](docs/PTERODACTYL.md). Local smoke-test JARs can be built from available natives, but a partial-native JAR is not a deployable release.

The current complete three-platform development package is at `dist/SryzaMC/server.jar` (also archived as `dist/SryzaMC-0.1.0-dev.zip`). It starts the Rust engine through Java, initializes missing defaults, verifies/extracts the native engine, and handles Pterodactyl-style SIGTERM shutdown. It is still not playable: after Configuration the engine closes the connection because Play and world spawning have not been implemented.

```text
sryzamc --dir ./server init
sryzamc --dir ./server info
sryzamc --dir ./server run
```

Status/ping, online/offline Login, and the Configuration packet exchange exist for protocol 777 (26.3), but Play and world spawning are incomplete and players cannot join a world yet. See [configuration reference](docs/CONFIGURATION.md) and [compatibility contract](docs/COMPATIBILITY.md).

## Layout

- `crates/sryza`: CLI, configuration, frame parser, tick clock, TCP listener.
- `config`: starter `server.properties` and complete operator configuration.
- `docs`: architecture, compatibility, operations, migration, roadmap.
- `.github/workflows`: formatting, lint, test, and release build.
- `benchmarks`: benchmark methodology; the current microbenchmark is not a Paper comparison.

AGPL-3.0-or-later; see `LICENSE`.
