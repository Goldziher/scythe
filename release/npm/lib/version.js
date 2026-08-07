"use strict";

const VERSION_RE = /(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)/;

/**
 * Permissively extracts a semver-ish version string from arbitrary CLI
 * output such as `scythe --version`. The exact output format of the real
 * binary has not been independently pinned by design, so this deliberately
 * does not anchor to the full line -- it just finds the first
 * `MAJOR.MINOR.PATCH[-pre][+build]` token.
 *
 * @param {string} output
 * @returns {string | null}
 */
function extractVersion(output) {
  const match = output.match(VERSION_RE);
  return match ? match[1] : null;
}

/**
 * Refuses to proceed when the package was published with the placeholder
 * `0.0.0` version, which means it was built from a dirty/unversioned
 * checkout and any download URL derived from it would be nonsense.
 *
 * @param {string} version
 * @throws {Error}
 */
function assertRealVersion(version) {
  if (version === "0.0.0") {
    throw new Error(
      "scythe-cli: this package was built incorrectly -- it still carries the placeholder " +
        'version "0.0.0". The publish workflow must inject the real release version before ' +
        "packing. Please report this at https://github.com/Goldziher/scythe/issues.",
    );
  }
}

module.exports = { extractVersion, assertRealVersion, VERSION_RE };
