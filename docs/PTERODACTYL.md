# Pterodactyl deployment

The release pipeline builds `server.jar` as a real Java 25 launcher. The JAR embeds the Rust engines for Linux x86_64 (static musl), Linux ARM64 (GNU libc), and Windows x86_64, selects the current OS/architecture, verifies the embedded SHA-256, extracts the engine under `libraries/native`, and forwards stdin/stdout/stderr and the native exit code. The ARM64 binary is built on Ubuntu 24.04 ARM and therefore requires a compatible GNU libc runtime.

Use a Java 25 Pterodactyl image and set the startup command to:

```bash
java -jar server.jar
```

On the first no-argument start, the launcher runs `sryzamc init` in the working directory if `sryza.toml` is absent, then starts `run`. Existing configuration is never overwritten. Keep the server's writable working directory on persistent storage. To inspect or initialize a specific directory, pass normal server CLI arguments, for example:

```bash
java -jar server.jar --dir /home/container init
java -jar server.jar --dir /home/container info
```

Build a launcher JAR from native binaries using JDK 25:

```bash
./launcher/build-jar.sh ./release-natives ./server.jar
```

The script requires all three native files and fails if any are missing. `build-jar.ps1` provides the equivalent PowerShell build. The partial-native switch exists only to smoke-test a local launcher; such a JAR is not a deployable cross-platform release.

**Current runtime limitation:** the Rust engine completes Login and Configuration but does not yet implement Play/world spawning, so it is not yet joinable as a Minecraft server. The launcher currently proves packaging, extraction, argument forwarding, and process exit handling; Pterodactyl connectivity/gameplay has not been verified.
