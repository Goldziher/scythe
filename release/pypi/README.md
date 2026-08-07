# scythe-sql

PyPI wrapper for [scythe](https://github.com/Goldziher/scythe), the polyglot SQL-to-code generator.
Installs a pinned `scythe` binary without requiring a Rust toolchain.

```bash
pip install scythe-sql
scythe --version
```

`pip install` does not run any code for a wheel install, so there is no download-on-install step.
Instead, the installed `scythe` console script downloads the matching prebuilt binary for your
platform on first run (verifying its SHA-256 checksum) and caches it under
`~/.cache/scythe/<version>/` (`%LOCALAPPDATA%\scythe\cache` on Windows). Every subsequent invocation
execs the cached binary directly. The script never writes into `site-packages`, so it works under
read-only installs, containers, and `pip install --user`.

## Warming the cache in a Dockerfile

```dockerfile
RUN pip install scythe-sql && scythe --version
```

## Supported platforms

- Linux x64 / arm64 (glibc only -- musl/Alpine is not supported; use `cargo install scythe-cli`)
- macOS x64 / arm64
- Windows x64 (Windows on ARM64 falls back to the x64 binary via emulation)

## Environment variables

- `SCYTHE_BINARY` -- absolute path to a `scythe` binary to use instead of downloading.
- `SCYTHE_SKIP_DOWNLOAD=1` -- never download; fail if no cached or PATH binary is available.
- `SCYTHE_CACHE_DIR` -- override the cache root directory.
- `PIP_CERT` / `REQUESTS_CA_BUNDLE` / `SSL_CERT_FILE` -- custom CA bundle.

Proxy configuration (`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`) is honoured natively via
`urllib.request`.
