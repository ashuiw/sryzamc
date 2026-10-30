# Configuration reference

`config/sryza.toml` is the complete default template. `sryza init` writes `sryza.toml`, `server.properties`, and data directories without overwriting an existing main config. The current parser validates `performance.target_tps` (1–20) and `network.max_packet_bytes` (at least 1024). Most settings are placeholders until their subsystem exists.

| Section | Fields | Current status |
|---|---|---|
| `server` | `motd`, `max_players`, `online_mode`, `view_distance`, `simulation_distance` | MOTD/player maximum are used for status. Offline mode supports Login then disconnects at Configuration; online mode rejects login until encryption/authentication land. |
| `performance` | `target_tps`, `tick_budget_ms`, `worker_threads`, `async_chunk_io` | TPS range validated; tick clock is scaffolding. |
| `world` | `name`, `seed`, `autosave_seconds`, `compression_level` | World generation/storage absent. |
| `entities` | `activation_range`, `max_per_chunk`, `tracking_range` | Entity engine absent. |
| `network` | `listen`, `max_packet_bytes`, `compression_threshold` | Binding, frame bound, handshake, status response, and ping/pong work; login/game protocol absent. |
| `runtime` | `java_bridge`, `java_executable`, `plugin_api` | No JVM bridge; leave disabled. |
| `plugins` | `directory`, `allowlist_only`, `load_timeout_seconds` | JARs are not loaded. |
| `permissions` | `default_group`, `op_level` | Permission system absent. |
| `commands` | `prefix`, `confirm_destructive` | Command system absent. |
| `logging` | `level`, `directory`, `json` | `RUST_LOG` controls tracing level; file logging absent. |
| `security` | `proxy_protocol`, `max_connections_per_ip`, `enforce_authentication` | Authentication, proxy and per-IP limit absent. |
| `backup` | `enabled`, `interval_minutes`, `retain`, `directory` | Backup system absent. |
| `resources` | `max_open_files`, `max_pending_chunks`, `max_packet_queue` | Only connection handler concurrency is currently bounded. |
| `monitoring` | `metrics`, `bind`, `debug_commands` | Metrics absent. |

`server.properties` is a compatibility template only; the running binary does not import it. Do not treat a placeholder setting as an active optimization or security control.
