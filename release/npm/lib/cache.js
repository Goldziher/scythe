"use strict";

const os = require("node:os");
const path = require("node:path");

/**
 * Resolves the root cache directory used to store downloaded scythe
 * binaries, honouring `SCYTHE_CACHE_DIR` and XDG conventions.
 *
 * @param {object} opts
 * @param {NodeJS.ProcessEnv} opts.env
 * @param {string} opts.platform - `process.platform` value.
 * @param {string} opts.homedir
 * @returns {string}
 */
function resolveCacheRoot({ env, platform, homedir }) {
  if (env.SCYTHE_CACHE_DIR) {
    return env.SCYTHE_CACHE_DIR;
  }
  if (platform === "win32") {
    const base = env.LOCALAPPDATA || path.join(homedir, "AppData", "Local");
    return path.join(base, "scythe", "cache");
  }
  if (env.XDG_CACHE_HOME) {
    return path.join(env.XDG_CACHE_HOME, "scythe");
  }
  return path.join(homedir, ".cache", "scythe");
}

/**
 * Resolves the cached binary path for a given version.
 *
 * @param {object} opts
 * @param {NodeJS.ProcessEnv} opts.env
 * @param {string} opts.platform
 * @param {string} opts.homedir
 * @param {string} opts.version
 * @param {string} opts.binaryName
 * @returns {string}
 */
function cachedBinaryPath({ env, platform, homedir, version, binaryName }) {
  return path.join(resolveCacheRoot({ env, platform, homedir: homedir ?? os.homedir() }), version, binaryName);
}

module.exports = { resolveCacheRoot, cachedBinaryPath };
