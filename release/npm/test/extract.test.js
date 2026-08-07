"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const { findBinaryEntry, extractTarGz } = require("../lib/extract");

test("findBinaryEntry finds the binary among LICENSE/README siblings", () => {
  assert.equal(findBinaryEntry(["LICENSE", "README.md", "scythe"], "scythe"), "scythe");
});

test("findBinaryEntry matches by basename when nested in a directory", () => {
  assert.equal(
    findBinaryEntry(["scythe-x86_64-unknown-linux-gnu/scythe"], "scythe"),
    "scythe-x86_64-unknown-linux-gnu/scythe",
  );
});

test("findBinaryEntry throws listing the archive contents when absent", () => {
  assert.throws(() => findBinaryEntry(["LICENSE", "README.md"], "scythe"), /LICENSE, README\.md/);
});

test("findBinaryEntry distinguishes scythe from scythe.exe", () => {
  assert.equal(findBinaryEntry(["scythe.exe"], "scythe.exe"), "scythe.exe");
  assert.throws(() => findBinaryEntry(["scythe.exe"], "scythe"));
});

/**
 * @param {import("node:test").TestContext} t
 * @returns {string}
 */
function tempDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "scythe-cli-extract-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/**
 * Builds a real `.tar.gz` buffer laid out like the released archive.
 *
 * @param {import("node:test").TestContext} t
 * @param {Record<string, string>} files
 * @returns {Promise<Buffer>}
 */
async function makeTarGz(t, files) {
  const tar = require("tar");
  const dir = tempDir(t);
  for (const [name, contents] of Object.entries(files)) {
    fs.writeFileSync(path.join(dir, name), contents);
  }
  const chunks = [];
  const stream = tar.c({ gzip: true, cwd: dir }, Object.keys(files));
  for await (const chunk of stream) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

test("extractTarGz publishes the binary complete and already executable", async (t) => {
  const archive = await makeTarGz(t, { LICENSE: "MIT", scythe: "#!/bin/sh\necho scythe\n" });
  const dest = path.join(tempDir(t), "0.13.0", "scythe");

  await extractTarGz(archive, "scythe", dest, 0o755);

  assert.equal(fs.readFileSync(dest, "utf8"), "#!/bin/sh\necho scythe\n");
  assert.equal(fs.statSync(dest).mode & 0o777, 0o755, "the binary must never be published without its mode");
});

test("extractTarGz replaces a truncated cache entry and leaves no scratch files", async (t) => {
  const archive = await makeTarGz(t, { scythe: "complete-binary" });
  const dir = tempDir(t);
  const dest = path.join(dir, "scythe");
  fs.writeFileSync(dest, "");

  await extractTarGz(archive, "scythe", dest, 0o755);

  assert.equal(fs.readFileSync(dest, "utf8"), "complete-binary");
  assert.deepEqual(fs.readdirSync(dir), ["scythe"]);
});

test("extractTarGz writes nothing when the archive lacks the binary", async (t) => {
  const archive = await makeTarGz(t, { LICENSE: "MIT" });
  const dir = tempDir(t);
  const dest = path.join(dir, "scythe");

  await assert.rejects(() => extractTarGz(archive, "scythe", dest, 0o755), /could not find "scythe"/);

  assert.deepEqual(fs.readdirSync(dir), []);
});
