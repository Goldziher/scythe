"use strict";

const fsp = require("node:fs/promises");
const path = require("node:path");

let sequence = 0;

/**
 * Builds a scratch path beside `destPath`.
 *
 * Same directory on purpose: `rename()` is only atomic within a filesystem,
 * so writing to the OS temp dir and renaming across devices would not be. The
 * pid and a per-process counter keep two concurrent installs sharing one
 * cache directory from writing to the same scratch file.
 *
 * @param {string} destPath
 * @returns {string}
 */
function tempPathFor(destPath) {
  sequence += 1;
  const name = `.${path.basename(destPath)}.tmp-${process.pid}-${sequence}`;
  return path.join(path.dirname(destPath), name);
}

/**
 * Writes `destPath` via a scratch file that is renamed into place, so a
 * crash, OOM or full disk mid-write can never leave a truncated file that
 * later runs would mistake for a complete, verified binary.
 *
 * The mode is applied before the rename so the file is never observable at
 * its final path without its permissions.
 *
 * @param {string} destPath
 * @param {(tempPath: string) => Promise<void>} write - writes the full contents to the given path.
 * @param {number} [mode] - chmod applied before publishing, e.g. 0o755.
 * @returns {Promise<void>}
 */
async function writeAtomic(destPath, write, mode) {
  await fsp.mkdir(path.dirname(destPath), { recursive: true });
  const tempPath = tempPathFor(destPath);

  try {
    await write(tempPath);
    if (mode !== undefined) {
      await fsp.chmod(tempPath, mode);
    }
    await fsp.rename(tempPath, destPath);
  } catch (err) {
    await fsp.rm(tempPath, { force: true });
    throw err;
  }
}

module.exports = { tempPathFor, writeAtomic };
