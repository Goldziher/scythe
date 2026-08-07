# scythe-cli

npm wrapper for [scythe](https://github.com/Goldziher/scythe), the polyglot SQL-to-code generator.
Installs a pinned `scythe` binary without requiring a Rust toolchain.

```bash
npm install --save-dev scythe-cli
npx scythe --version
```

On install, a postinstall script downloads the matching prebuilt binary for your platform from the
matching GitHub release, verifies its SHA-256 checksum, and caches it under `~/.cache/scythe/<version>/`
(`%LOCALAPPDATA%\scythe\cache` on Windows). The `scythe` bin shim then execs that cached binary,
forwarding arguments and the exit code.

## Supported platforms

- Linux x64 / arm64 (glibc only -- musl/Alpine is not supported; use `cargo install scythe-cli`)
- macOS x64 / arm64
- Windows x64 (Windows on ARM64 falls back to the x64 binary via emulation)

## Environment variables

- `SCYTHE_BINARY` -- absolute path to a `scythe` binary to use instead of downloading.
- `SCYTHE_SKIP_DOWNLOAD=1` -- skip the postinstall download entirely.
- `SCYTHE_CACHE_DIR` -- override the cache root directory.
- `HTTPS_PROXY` / `npm_config_https_proxy` / `npm_config_proxy` / `NO_PROXY` -- proxy configuration.
- `npm_config_cafile` / `NODE_EXTRA_CA_CERTS` -- custom CA bundle.

## Docker

Warm the cache at build time:

```dockerfile
RUN npm install --save-dev scythe-cli && npx scythe --version
```
