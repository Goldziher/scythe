"use strict";

const crypto = require("node:crypto");

/**
 * Parses a goreleaser `scythe_<version>_checksums.txt` file into a map of
 * `filename -> lowercase hex sha256`.
 *
 * Format: `<hex-digest>  <filename>` (two spaces), one per line.
 *
 * @param {string} contents
 * @returns {Map<string, string>}
 */
function parseChecksums(contents) {
  const map = new Map();
  for (const line of contents.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const match = trimmed.match(/^([0-9a-fA-F]{64})\s+\*?(\S+)$/);
    if (!match) continue;
    const [, hash, filename] = match;
    map.set(filename, hash.toLowerCase());
  }
  return map;
}

/**
 * Looks up the expected checksum for `assetFilename`. Throws if the row is
 * missing -- a missing row means the release is malformed and verification
 * must not be silently skipped.
 *
 * @param {Map<string, string>} checksums
 * @param {string} assetFilename
 * @param {string} checksumsUrl
 * @returns {string} lowercase hex sha256
 */
function expectedChecksum(checksums, assetFilename, checksumsUrl) {
  const expected = checksums.get(assetFilename);
  if (!expected) {
    throw new Error(
      `scythe-cli: no checksum entry for "${assetFilename}" in ${checksumsUrl}. ` +
        "The release appears malformed; refusing to install without verification.",
    );
  }
  return expected;
}

/**
 * Computes the lowercase hex sha256 digest of a buffer.
 *
 * @param {Buffer} buffer
 * @returns {string}
 */
function sha256Hex(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

/**
 * Verifies `buffer` against the expected checksum, case-insensitively.
 * Throws naming both hashes and the source URL on mismatch.
 *
 * @param {Buffer} buffer
 * @param {string} expectedHex
 * @param {string} assetUrl
 */
function verifyChecksum(buffer, expectedHex, assetUrl) {
  const actual = sha256Hex(buffer);
  if (actual.toLowerCase() !== expectedHex.toLowerCase()) {
    throw new Error(
      `scythe-cli: checksum mismatch for ${assetUrl}\n` +
        `  expected: ${expectedHex.toLowerCase()}\n` +
        `  actual:   ${actual}`,
    );
  }
}

module.exports = { parseChecksums, expectedChecksum, sha256Hex, verifyChecksum };
