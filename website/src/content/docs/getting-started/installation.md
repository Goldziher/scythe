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

## Pre-commit / prek

If you only need scythe for pre-commit hooks, add it directly to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.16.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
```

See [Pre-commit Hooks](/scythe/guide/pre-commit-hooks/) for all available hooks and configuration.

## Verify Installation

```bash
scythe --version
```
