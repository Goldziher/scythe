# Release checks

`duckdb-binary-sizes.json` is the source of truth for the release matrix and for the release-binary size ratchet.
It lists the five triples the publish workflow builds, plus, per target, the pre-DuckDB baseline, the first observed
bundled-DuckDB size and a maximum with roughly 6–11% headroom over that observation.

Each triple is built natively on its own runner (Linux in a manylinux_2_28 container so the glibc floor stays at 2.28,
macOS on `macos-26` and `macos-26-intel`, Windows on `windows-2022` with MSVC). There is no cross-compilation step:
an archive that cannot be produced on its own platform is a bug in the pipeline, not something a cross-toolchain should
paper over.

Run `mise run release:canary:target <triple>` to build and size-check one target on a host that can build it, or
`mise run release:canary` to run every target the host can build. The publish workflow runs the per-target check before
packaging, so a binary above its maximum fails the release before any archive is uploaded.

`scripts/package-release.sh <triple>` turns a built binary into the published archive: `scythe`, the pinned
`scythe-pg15-parser`/`scythe-pg18-parser` helpers, `pg-parser-manifest.json`, `THIRD_PARTY_LICENSES.txt`,
`PARSER_PROTOCOL.md`, `CHANGELOG.md`, `LICENSE` and `README.md`, flat at the archive root. The Windows archive carries
`PARSER_HELPERS_UNAVAILABLE.txt` instead of the helpers, because upstream libpg_query requires POSIX socket headers
Windows does not provide. The script fails rather than producing a partial archive.

Update a target's `observed_bundled_bytes` and `max_bytes` deliberately when a reviewed dependency or code change
accounts for the increase. Each target records `"measured": true` with a provenance note only after a native canary
observation on that platform; an entry still carried over from an earlier observation carries `"measured": false` and
its note names the runner that has to record it. The Windows figures currently carried over from the previous
`windows-gnu` target must be re-recorded from the first MSVC canary observation.

# Publish order

`.github/workflows/publish.yaml` only moves in one direction, so a failure leaves the release in a state that can be
re-run rather than one that has to be repaired by hand:

1. `meta` resolves the tag and refuses to continue unless every crate version agrees with it.
2. `create_release` creates the tag's release as a **draft**. A platform build that fails therefore leaves a hidden
   draft, not a published version with no archives.
3. `build_linux_assets` and `build_unix_assets` build, size-check, smoke-test and upload one archive per target;
   `checksums` refuses to run until all five are present, then uploads `scythe_<version>_checksums.txt`.
4. `verify_archive` downloads the Linux archive, checks it against that file, asserts the bundled members and runs the
   cluster-scope gate from the extracted copy.
5. `finalize_release` promotes the draft to published once every archive and the checksums file exist.
6. Homebrew, crates.io, npm and PyPI publish only after that promotion — assets first, registries second, because a
   registry upload that never happened can be repeated and a release nobody can download cannot be fixed. npm and PyPI
   are smoke-tested before they are published.
7. `verify_channels` fails the run unless crates.io, the GitHub release, npm and PyPI all carry the version, and
   `update_major_tag` moves `v0` only after that check passes.
