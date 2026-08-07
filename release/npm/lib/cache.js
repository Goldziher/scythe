"use strict";

const fs = require("node:fs");
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

/**
 * Decides whether a cached path may be reused without re-downloading.
 *
 * Mere existence is not enough. Binaries published by this package are
 * renamed into place only after a full write and a checksum match, but a
 * cache directory can still hold a truncated leftover from an older version
 * of this installer that wrote straight to the final path. An empty file, a
 * non-file, or (on POSIX) a binary with no executable bit are all signs of
 * exactly that, and must trigger a fresh, verified download instead.
 *
 * @param {string} binaryPath
 * @param {string} platform - `process.platform` value.
 * @param {(path: string) => import("node:fs").Stats} [statSync]
 * @returns {boolean}
 */
function isUsableCachedBinary(binaryPath, platform, statSync = fs.statSync) {
  let stats;
  try {
    stats = statSync(binaryPath);
  } catch {
    return false;
  }
  if (!stats.isFile() || stats.size === 0) return false;
  if (platform === "win32") return true;
  return (stats.mode & 0o111) !== 0;
}

module.exports = { resolveCacheRoot, cachedBinaryPath, isUsableCachedBinary };
