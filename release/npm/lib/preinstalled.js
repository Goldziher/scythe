"use strict";

const { execFileSync } = require("node:child_process");
const { extractVersion } = require("./version");

/**
 * Checks whether a `scythe` binary already on PATH matches `wantVersion`
 * exactly. Returns null (never on PATH, or a different version) rather than
 * throwing, since this is a soft short-circuit, not a required condition.
 *
 * Exact equality only: a newer binary on PATH does not satisfy a pin, since
 * silently upgrading defeats the point of pinning the devDependency version.
 *
 * @param {string} wantVersion
 * @param {(cmd: string, args: string[]) => Buffer} [execFn]
 * @returns {boolean}
 */
function hasMatchingPathBinary(wantVersion, execFn = defaultExec) {
  let output;
  try {
    output = execFn("scythe", ["--version"]).toString("utf8");
  } catch {
    return false;
  }
  const found = extractVersion(output);
  return found === wantVersion;
}

function defaultExec(cmd, args) {
  return execFileSync(cmd, args, { stdio: ["ignore", "pipe", "ignore"] });
}

module.exports = { hasMatchingPathBinary };
