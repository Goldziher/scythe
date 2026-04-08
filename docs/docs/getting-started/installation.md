# Installation

## Cargo (Rust)

```bash
cargo install scythe-cli
```

## Homebrew (macOS/Linux)

```bash
brew install Goldziher/tap/scythe
```

Pre-built binaries are available for macOS (arm64, x86_64) and Linux (x86_64). No Rust toolchain needed.

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
    rev: v0.5.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
```

See [Pre-commit Hooks](../guide/pre-commit-hooks.md) for all available hooks and configuration.

## Verify Installation

```bash
scythe --version
```
