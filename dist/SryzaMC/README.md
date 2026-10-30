# SryzaMC development distribution

Requires Java 25 or newer. This Java launcher JAR embeds native engines for Linux x86_64 (static musl), Linux ARM64 (GNU libc), and Windows x86_64. The Linux ARM64 binary in this local development package was cross-compiled against glibc 2.43 and has not been executed on ARM hardware; the tag release build uses Ubuntu 24.04 ARM (glibc 2.39).

For Pterodactyl, use startup command:

```bash
java -jar server.jar
```

The first no-argument run initializes configuration if it is absent. Existing configuration is preserved. The native executable is extracted under `libraries/native` and SHA-256 checked before execution.

**This is not yet a playable server.** Status, Login, and Configuration are implemented; the connection closes after Configuration because Play packets, world data, and spawning remain incomplete. See the repository's `docs/PTERODACTYL.md` and `docs/COMPATIBILITY.md` for the exact scope.
