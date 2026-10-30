# Pterodactyl egg notes

Use a Java 25 container and set the server startup command to `java -jar server.jar`. The launcher initializes default files on the first no-argument run, extracts the OS/architecture-specific engine into `libraries/native`, forwards console streams, and propagates the engine exit status. SIGTERM and SIGINT are forwarded to the Rust engine and stop its accept loop.

Upload the universal `server.jar` from the SryzaMC distribution. It contains Linux x86_64, Linux ARM64, and Windows x86_64 engines. Keep the panel's server directory persistent and writable so the extracted native and generated configuration remain available between starts.

This build is a development distribution, not a playable release: Minecraft Login and Configuration work, but the Play state, world engine, and player spawn are not implemented. A panel can start and stop it, but players cannot join a world yet.
