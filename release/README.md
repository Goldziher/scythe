# Release checks

`duckdb-binary-sizes.json` is the release-binary size ratchet for the five targets published by GoReleaser.
The recorded baseline is the pre-DuckDB binary, and `observed_bundled_bytes` is the first successful bundled-DuckDB
canary. `max_bytes` allows roughly 6–11% headroom over that observation while still detecting an accidental large
increase.

Run `task release:canary` before a release, or use `task release:canary:target TARGET=<triple>` for one target. The
check builds the same `scythe-cli` release artifact as GoReleaser, verifies the target list against
`.goreleaser.yaml`, raises the build process's open-file limit to at least 4096, and rejects a binary above its
target-specific maximum. Update the observed and maximum sizes deliberately when a reviewed dependency or code
change accounts for the increase.
