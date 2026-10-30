# Architecture

Intended layers: Core → Server → Networking → World → Chunks → Entities → Scheduler → Plugin API → Compatibility → Configuration → Persistence. Current code covers only configuration, frame-length parsing, a timer, and a TCP listener.

The eventual world state should have one writer per world or region, with explicit cross-owner messages. Network input must be bounded and protocol-state checked before expensive work. Chunk I/O and generation can use workers, but world mutations and plugin events need deterministic ownership. Plugin callbacks need latency budgets and fault isolation before parallel ticks are considered.

Pin protocol support by Java Edition protocol version. Handshake parsing is not protocol support: status, authentication, encryption, compression, configuration, play packets, registries, and data fixes are still required. Do not write world formats until verified against real Anvil/NBT fixtures; save atomically and preserve unknown metadata.
