---
title: Installation
description: Install scythe via cargo, npm, pip, Homebrew, from source, or as a pre-commit hook.
---

## Cargo (Rust)

```bash
cargo install scythe-cli
```

`cargo-binstall` is also supported and downloads the prebuilt binary instead of compiling from source:

```bash
cargo binstall scythe-cli
```

## npm (Node.js)

For Node.js projects, install scythe as a pinned dev dependency -- no Rust toolchain required. A
postinstall step downloads the prebuilt binary matching your platform and verifies its checksum.

```bash
npm install --save-dev scythe-cli
npx scythe --version
```

Supports Linux (x64/arm64, glibc only), macOS (x64/arm64), and Windows (x64, with x64 emulation on
ARM64). See the [scythe-cli README](https://github.com/Goldziher/scythe/tree/main/release/npm) for
supported environment variables (proxy configuration, `SCYTHE_BINARY`, `SCYTHE_CACHE_DIR`).

## pip (Python)

```bash
pip install scythe-sql
scythe --version
```

`pip install` does not run code for a wheel install, so the binary is downloaded (and its checksum
verified) on first invocation of `scythe`, then cached and reused. In a Dockerfile, warm the cache
at build time with `pip install scythe-sql && scythe --version`. See the
[scythe-sql README](https://github.com/Goldziher/scythe/tree/main/release/pypi) for supported
environment variables.

## Homebrew (macOS/Linux)

```bash
brew install Goldziher/tap/scythe
```

Pre-built binaries are available for macOS (arm64, x86_64) and Linux (x86_64, arm64). No Rust toolchain needed.

## From Source

```bash
git clone https://github.com/Goldziher/scythe.git
cd scythe
cargo install --path crates/scythe-cli
```

## GitHub Actions

Use the moving `v0` action to install the latest scythe release, verify its checksum, and add it to
`PATH`:

```yaml
steps:
  - uses: actions/checkout@v4
  - name: Install scythe
    id: scythe
    uses: Goldziher/scythe@v0
  - run: scythe --version
```

`v0` tracks the latest compatible action implementation. By default, the action resolves the latest
scythe GitHub release using `github.token` and caches the installed binary by version, operating
system, and runner architecture. Pin the CLI version independently when reproducibility matters:

```yaml
- name: Install scythe 0.18.1 without caching
  id: scythe
  uses: Goldziher/scythe@v0
  with:
    version: 0.18.1
    cache: false
- run: scythe check
```

The `version` input accepts `0.18.1` or `v0.18.1`; omit it or pass `latest` to resolve the latest
release. Set `github-token` only when the default workflow token cannot read release metadata.

Every download is checked against the release's SHA-256 checksum file before installation. The
action supports native x64 and ARM64 Linux and macOS runners. Windows x64 and ARM64 runners install
the x64 Windows binary; Windows ARM64 therefore requires x64 emulation.

The step exposes these outputs:

| Output | Value |
| --- | --- |
| `version` | Installed version without the `v` prefix |
| `target` | Installed Rust target triple |
| `install-dir` | Directory added to `PATH` |

Reference them through the step id, for example `${{ steps.scythe.outputs.version }}`.

## Pre-commit / prek

If you only need scythe for pre-commit hooks, add it directly to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.18.1
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
```

See [Pre-commit Hooks](/scythe/guide/pre-commit-hooks/) for all available hooks and configuration.

## Verify Installation

```bash
scythe --version
```
