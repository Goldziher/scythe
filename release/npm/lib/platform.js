"use strict";

/**
 * Platform+arch -> Rust target triple resolution for the scythe release assets.
 *
 * Release assets published by goreleaser (verified against the v0.12.0 release):
 *   scythe-x86_64-unknown-linux-gnu.tar.gz
 *   scythe-aarch64-unknown-linux-gnu.tar.gz
 *   scythe-x86_64-apple-darwin.tar.gz
 *   scythe-aarch64-apple-darwin.tar.gz
 *   scythe-x86_64-pc-windows-gnu.zip
 * There is no musl target and no aarch64 Windows asset.
 */

const TRIPLES = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-gnu",
};

/**
 * Detects whether the current Linux process is running against musl libc
 * (e.g. Alpine) rather than glibc. The gnu-targeted binary fails at exec
 * time with an opaque dynamic-loader error on musl systems, so this must be
 * checked *before* attempting a download.
 *
 * @param {() => { header?: { glibcVersionRuntime?: string } }} getReport
 * @returns {boolean}
 */
function isMuslLinux(getReport) {
  const report = getReport();
  return report?.header?.glibcVersionRuntime === undefined;
}

/**
 * Resolves the Rust target triple and archive extension for the current
 * platform/arch, or throws a descriptive Error naming the unsupported
 * platform.
 *
 * @param {object} opts
 * @param {string} opts.platform - `process.platform` value.
 * @param {string} opts.arch - `process.arch` value.
 * @param {() => boolean} [opts.isMusl] - returns true when running on musl libc (Linux only).
 * @param {(message: string) => void} [opts.warn] - called with a warning message on fallback.
 * @returns {{ target: string, archiveExt: "tar.gz" | "zip", binaryName: string }}
 */
function resolveTarget({ platform, arch, isMusl, warn }) {
  const warnFn = warn ?? ((message) => process.stderr.write(`${message}\n`));

  if (platform === "linux" && typeof isMusl === "function" && isMusl()) {
    throw new Error(
      `scythe-cli: unsupported platform "linux/${arch}" (musl libc, e.g. Alpine). ` +
        "No musl build is published. Install a glibc-based image, or install scythe " +
        'via "cargo install scythe-cli" instead.',
    );
  }

  if (platform === "win32" && arch === "arm64") {
    warnFn(
      "scythe-cli: no native Windows ARM64 build is published; falling back to the " +
        "x86_64-pc-windows-gnu binary, which runs under Windows 11 ARM64 x64 emulation.",
    );
    return { target: TRIPLES["win32-x64"], archiveExt: "zip", binaryName: "scythe.exe" };
  }

  const key = `${platform}-${arch}`;
  const target = TRIPLES[key];
  if (!target) {
    throw new Error(
      `scythe-cli: unsupported platform "${platform}/${arch}". Supported platforms: ` +
        `${Object.keys(TRIPLES).join(", ")} (plus win32/arm64 via x64 emulation). ` +
        'Install scythe via "cargo install scythe-cli" instead.',
    );
  }

  const archiveExt = platform === "win32" ? "zip" : "tar.gz";
  const binaryName = platform === "win32" ? "scythe.exe" : "scythe";
  return { target, archiveExt, binaryName };
}

module.exports = { resolveTarget, isMuslLinux, TRIPLES };
