# Migration

`sryzamc --dir <destination> migrate <source>` creates a migration inventory: common configuration files and plugin JAR count. It does not import settings, copy worlds, or transform plugin data, and it leaves the source untouched.

Before a future migration, stop the old server cleanly and make a verified backup. A complete migration must preserve region files, player data, dimensions, advancements, statistics, data packs, permissions, and plugin databases. Test a copy and compare integrity before cutover.
