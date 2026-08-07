#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const fsp = require("node:fs/promises");
const os = require("node:os");

const pkg = require("./package.json");
const { resolveTarget, isMuslLinux } = require("./lib/platform");
const { cachedBinaryPath } = require("./lib/cache");
const { assertRealVersion } = require("./lib/version");
const { parseChecksums, expectedChecksum, verifyChecksum } = require("./lib/checksum");
const { resolveProxyUrl, resolveCaFile } = require("./lib/proxy");
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

  if (fs.existsSync(destPath)) {
    process.stderr.write(`scythe-cli: scythe ${version} already cached at ${destPath}.\n`);
    return;
  }

  const assetName = `scythe-${target}.${archiveExt}`;
  const checksumsName = `scythe_${version}_checksums.txt`;
  const baseUrl = `${REPO}/releases/download/v${version}`;
  const assetUrl = `${baseUrl}/${assetName}`;
  const checksumsUrl = `${baseUrl}/${checksumsName}`;

  process.stderr.write(`scythe-cli: downloading scythe ${version} for ${target}...\n`);

  const checksumsText = await fetchText(checksumsUrl);
  const checksums = parseChecksums(checksumsText);
  const expected = expectedChecksum(checksums, assetName, checksumsUrl);

  const assetBuffer = await fetchBuffer(assetUrl);
  verifyChecksum(assetBuffer, expected, assetUrl);

  if (archiveExt === "zip") {
    await extractZip(assetBuffer, binaryName, destPath);
  } else {
    await extractTarGz(assetBuffer, binaryName, destPath);
  }

  if (process.platform !== "win32") {
    await fsp.chmod(destPath, 0o755);
  }

  process.stderr.write(`scythe-cli: installed scythe ${version} to ${destPath}\n`);
}

/**
 * @param {string} url
 * @returns {Promise<import("node:http").Agent | undefined>}
 */
async function buildDispatcher(url) {
  const host = new URL(url).hostname;
  const proxyUrl = resolveProxyUrl(process.env, host);
  if (!proxyUrl) return undefined;
  const { HttpsProxyAgent } = require("https-proxy-agent");
  return new HttpsProxyAgent(proxyUrl);
}

async function fetchImpl(url) {
  const dispatcher = await buildDispatcher(url);
  const caFile = resolveCaFile(process.env);
  const fetchOptions = {};
  if (dispatcher) fetchOptions.dispatcher = dispatcher;
  if (caFile) {
    process.stderr.write(`scythe-cli: using CA bundle from ${caFile}\n`);
  }
  const response = await fetch(url, fetchOptions);
  if (!response.ok) {
    throw new Error(`scythe-cli: failed to download ${url}: HTTP ${response.status}`);
  }
  return response;
}

async function fetchText(url) {
  const response = await fetchImpl(url);
  return response.text();
}

async function fetchBuffer(url) {
  const response = await fetchImpl(url);
  const arrayBuffer = await response.arrayBuffer();
  return Buffer.from(arrayBuffer);
}

main().catch((err) => {
  process.stderr.write(`${err.message}\n`);
  process.exit(1);
});
