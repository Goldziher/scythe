# Release checks

`duckdb-binary-sizes.json` is the source of truth for the release matrix and for the release-binary size ratchet.
It lists the five triples the publish workflow builds, plus, per target, the pre-DuckDB baseline, the first observed
bundled-DuckDB size and a maximum with roughly 6–11% headroom over that observation.

Each triple is built natively on its own runner (Linux in a manylinux_2_28 container so the glibc floor stays at 2.28,
macOS on `macos-latest` and `macos-15-intel`, Windows on `windows-2022` with MSVC). There is no cross-compilation step:
an archive that cannot be produced on its own platform is a bug in the pipeline, not something a cross-toolchain should
paper over.

Run `task release:canary:target TARGET=<triple>` to build and size-check one target on a host that can build it, or
`task release:canary` to run every target the host can build. The publish workflow runs the per-target check before
packaging, so a binary above its maximum fails the release before any archive is uploaded.

`scripts/package-release.sh <triple>` turns a built binary into the published archive: `scythe`, the pinned
`scythe-pg15-parser`/`scythe-pg18-parser` helpers, `pg-parser-manifest.json`, `THIRD_PARTY_LICENSES.txt`,
`PARSER_PROTOCOL.md`, `CHANGELOG.md`, `LICENSE` and `README.md`, flat at the archive root. The Windows archive carries
`PARSER_HELPERS_UNAVAILABLE.txt` instead of the helpers, because upstream libpg_query requires POSIX socket headers
Windows does not provide. The script fails rather than producing a partial archive.

Update a target's `observed_bundled_bytes` and `max_bytes` deliberately when a reviewed dependency or code change
accounts for the increase; the Windows figures currently carried over from the previous `windows-gnu` target must be
re-recorded from the first MSVC canary observation.
