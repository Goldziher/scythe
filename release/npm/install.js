#!/usr/bin/env node
"use strict";

const os = require("node:os");

const pkg = require("./package.json");
const { resolveTarget, isMuslLinux } = require("./lib/platform");
const { cachedBinaryPath, isUsableCachedBinary } = require("./lib/cache");
const { assertRealVersion } = require("./lib/version");
const { parseChecksums, expectedChecksum, verifyChecksum } = require("./lib/checksum");
const { downloadBuffer, downloadText } = require("./lib/download");
const { extractTarGz, extractZip } = require("./lib/extract");
const { hasMatchingPathBinary } = require("./lib/preinstalled");

const REPO = "https://github.com/Goldziher/scythe";

async function main() {
  if (process.env.SCYTHE_SKIP_DOWNLOAD === "1") {
    process.stderr.write("scythe-cli: SCYTHE_SKIP_DOWNLOAD=1 set, skipping binary download.\n");
    return;
  }
  if (process.env.SCYTHE_BINARY) {
    process.stderr.write(`scythe-cli: SCYTHE_BINARY=${process.env.SCYTHE_BINARY} set, skipping binary download.\n`);
    return;
  }

  const version = pkg.version;
  assertRealVersion(version);

  if (hasMatchingPathBinary(version)) {
    process.stderr.write(`scythe-cli: found scythe ${version} already on PATH, skipping download.\n`);
    return;
  }

  const { target, archiveExt, binaryName } = resolveTarget({
    platform: process.platform,
    arch: process.arch,
    isMusl: () => (process.platform === "linux" ? isMuslLinux(() => process.report.getReport()) : false),
  });

  const destPath = cachedBinaryPath({
    env: process.env,
    platform: process.platform,
    homedir: os.homedir(),
    version,
    binaryName,
  });

  if (isUsableCachedBinary(destPath, process.platform)) {
    process.stderr.write(`scythe-cli: scythe ${version} already cached at ${destPath}.\n`);
    return;
  }

  const assetName = `scythe-${target}.${archiveExt}`;
  const checksumsName = `scythe_${version}_checksums.txt`;
  const baseUrl = `${REPO}/releases/download/v${version}`;
  const assetUrl = `${baseUrl}/${assetName}`;
  const checksumsUrl = `${baseUrl}/${checksumsName}`;

  process.stderr.write(`scythe-cli: downloading scythe ${version} for ${target}...\n`);

  const log = (message) => process.stderr.write(message);

  const checksumsText = await downloadText(checksumsUrl, { log });
  const checksums = parseChecksums(checksumsText);
  const expected = expectedChecksum(checksums, assetName, checksumsUrl);

  const assetBuffer = await downloadBuffer(assetUrl, { log });
  verifyChecksum(assetBuffer, expected, assetUrl);

  // Extraction publishes the binary atomically, already executable, so no
  // partially written or non-runnable file is ever visible at destPath.
  const mode = process.platform === "win32" ? undefined : 0o755;
  if (archiveExt === "zip") {
    await extractZip(assetBuffer, binaryName, destPath, mode);
  } else {
    await extractTarGz(assetBuffer, binaryName, destPath, mode);
  }

  process.stderr.write(`scythe-cli: installed scythe ${version} to ${destPath}\n`);
}

main().catch((err) => {
  process.stderr.write(`${err.message}\n`);
  process.exit(1);
});
