"use strict";

const fs = require("node:fs");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

/**
 * Finds the archive entry whose basename matches `binaryName`. The archive
 * also contains LICENSE/README (goreleaser default), so callers must not
 * assume a fixed path -- search by basename instead.
 *
 * @param {string[]} entryNames - archive-relative paths (may use `/` separators).
 * @param {string} binaryName - e.g. "scythe" or "scythe.exe".
 * @returns {string} the matching entry name.
 * @throws {Error} when no entry matches.
 */
function findBinaryEntry(entryNames, binaryName) {
  const match = entryNames.find((name) => name.split("/").pop() === binaryName);
  if (!match) {
    throw new Error(
      `scythe-cli: could not find "${binaryName}" inside the downloaded archive. ` +
        `Archive contained: ${entryNames.join(", ") || "(empty)"}`,
    );
  }
  return match;
}

/**
 * Extracts the `scythe` binary from a `.tar.gz` archive buffer to `destPath`.
 *
 * @param {Buffer} buffer
 * @param {string} binaryName
 * @param {string} destPath
 */
async function extractTarGz(buffer, binaryName, destPath) {
  const tar = require("tar");
  const { Readable } = require("node:stream");
  const tmpDir = await fsp.mkdtemp(path.join(os.tmpdir(), "scythe-cli-"));
  try {
    const entries = [];
    await new Promise((resolve, reject) => {
      Readable.from(buffer)
        .pipe(tar.x({ cwd: tmpDir, onentry: (entry) => entries.push(entry.path) }))
        .on("finish", resolve)
        .on("error", reject);
    });

    const entryName = findBinaryEntry(entries, binaryName);
    const extractedPath = path.join(tmpDir, entryName);
    await fsp.mkdir(path.dirname(destPath), { recursive: true });
    await fsp.copyFile(extractedPath, destPath);
  } finally {
    await fsp.rm(tmpDir, { recursive: true, force: true });
  }
}

/**
 * Extracts the `scythe.exe` binary from a `.zip` archive buffer to `destPath`.
 *
 * @param {Buffer} buffer
 * @param {string} binaryName
 * @param {string} destPath
 */
async function extractZip(buffer, binaryName, destPath) {
  const yauzl = require("yauzl");

  await fsp.mkdir(path.dirname(destPath), { recursive: true });

  await new Promise((resolve, reject) => {
    yauzl.fromBuffer(buffer, { lazyEntries: true }, (err, zipfile) => {
      if (err) return reject(err);

      let found = false;
      zipfile.readEntry();
      zipfile.on("entry", (entry) => {
        const basename = entry.fileName.split("/").pop();
        if (basename !== binaryName) {
          zipfile.readEntry();
          return;
        }
        found = true;
        zipfile.openReadStream(entry, (streamErr, readStream) => {
          if (streamErr) return reject(streamErr);
          const out = fs.createWriteStream(destPath);
          readStream.pipe(out);
          out.on("finish", () => {
            zipfile.close();
            resolve(undefined);
          });
          out.on("error", reject);
        });
      });
      zipfile.on("end", () => {
        if (!found) {
          reject(new Error(`scythe-cli: could not find "${binaryName}" inside the downloaded zip archive.`));
        }
      });
      zipfile.on("error", reject);
    });
  });
}

module.exports = { findBinaryEntry, extractTarGz, extractZip };
