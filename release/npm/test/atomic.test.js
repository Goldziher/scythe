"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const { tempPathFor, writeAtomic } = require("../lib/atomic");

/**
 * @param {import("node:test").TestContext} t
 * @returns {string}
 */
function tempDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "scythe-cli-atomic-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("tempPathFor stays in the destination directory so rename is atomic", () => {
  const dest = path.join("/cache", "scythe", "0.13.0", "scythe");
  const temp = tempPathFor(dest);
  assert.equal(path.dirname(temp), path.dirname(dest));
  assert.notEqual(temp, dest);
  assert.ok(temp.includes(String(process.pid)), `expected the pid in ${temp}`);
});

test("tempPathFor never hands out the same path twice", () => {
  const dest = "/cache/scythe";
  assert.notEqual(tempPathFor(dest), tempPathFor(dest));
});

test("writeAtomic publishes nothing until the write has completed", async (t) => {
  const dest = path.join(tempDir(t), "nested", "scythe");
  let observedMidWrite = true;

  await writeAtomic(dest, async (temp) => {
    await fsp.writeFile(temp, "partial");
    observedMidWrite = fs.existsSync(dest);
    await fsp.appendFile(temp, "-complete");
  });

  assert.equal(observedMidWrite, false, "destination must not exist while the write is in flight");
  assert.equal(fs.readFileSync(dest, "utf8"), "partial-complete");
});

test("writeAtomic leaves no destination and no scratch file when the write fails", async (t) => {
  const dir = tempDir(t);
  const dest = path.join(dir, "scythe");

  await assert.rejects(
    () =>
      writeAtomic(dest, async (temp) => {
        await fsp.writeFile(temp, "truncated");
        throw new Error("disk full");
      }),
    /disk full/,
  );

  assert.equal(fs.existsSync(dest), false);
  assert.deepEqual(fs.readdirSync(dir), []);
});

test("writeAtomic applies the mode before publishing", async (t) => {
  const dest = path.join(tempDir(t), "scythe");
  await writeAtomic(dest, (temp) => fsp.writeFile(temp, "binary"), 0o755);
  assert.equal(fs.statSync(dest).mode & 0o777, 0o755);
});

test("writeAtomic replaces an existing file in place", async (t) => {
  const dest = path.join(tempDir(t), "scythe");
  fs.writeFileSync(dest, "");

  await writeAtomic(dest, (temp) => fsp.writeFile(temp, "fresh"), 0o755);

  assert.equal(fs.readFileSync(dest, "utf8"), "fresh");
});

test("concurrent writeAtomic calls do not corrupt each other", async (t) => {
  const dir = tempDir(t);
  const dest = path.join(dir, "scythe");
  const payload = "x".repeat(4096);

  await Promise.all([
    writeAtomic(dest, (temp) => fsp.writeFile(temp, payload)),
    writeAtomic(dest, (temp) => fsp.writeFile(temp, payload)),
    writeAtomic(dest, (temp) => fsp.writeFile(temp, payload)),
  ]);

  assert.equal(fs.readFileSync(dest, "utf8"), payload);
  assert.deepEqual(fs.readdirSync(dir), ["scythe"], "no scratch files may be left behind");
});
