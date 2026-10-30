# Plugin compatibility contract

The goal is running Bukkit, Spigot, and Paper plugins without edits. **No plugin runs on this build.** There is no JVM, API classes, plugin descriptor scanner, classloader, lifecycle, scheduler integration, event dispatcher, command map, permission service, or CraftBukkit/Paper implementation.

## Bridge work required

1. Select a Java baseline and concrete API release.
2. Run bytecode in an embedded JVM or supervised sidecar; define bounded, versioned RPC messages.
3. Provide matching API classes and translate lifecycle, events, commands, permissions, scheduler calls, configuration, and logging.
4. Preserve main-thread semantics for world mutations; specify event ordering, cancellation, exceptions, and timeouts.
5. Test real JARs in a versioned compatibility harness and publish per-plugin results.

Plugins using server internals, reflection, native libraries, NMS, undocumented ordering, or timing may remain incompatible. A JAR loading successfully is not proof of API compatibility.

| Area | State |
|---|---|
| JAR discovery/class loading | Not implemented |
| Bukkit API/lifecycle | Not implemented |
| Spigot/Paper extensions | Not implemented |
| Events/commands/permissions | Not implemented |
| Protocol | Protocol 777 status and ping; login/play are not implemented and clients cannot join |
