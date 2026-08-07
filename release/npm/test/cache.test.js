"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const { resolveCacheRoot, cachedBinaryPath, isUsableCachedBinary } = require("../lib/cache");

test("resolveCacheRoot honours SCYTHE_CACHE_DIR override on any platform", () => {
  assert.equal(
    resolveCacheRoot({ env: { SCYTHE_CACHE_DIR: "/custom" }, platform: "linux", homedir: "/home/u" }),
    "/custom",
  );
});

test("resolveCacheRoot uses XDG_CACHE_HOME on linux when set", () => {
  assert.equal(
    resolveCacheRoot({ env: { XDG_CACHE_HOME: "/xdg" }, platform: "linux", homedir: "/home/u" }),
    path.join("/xdg", "scythe"),
  );
});

test("resolveCacheRoot defaults to ~/.cache/scythe on linux/darwin", () => {
  assert.equal(
    resolveCacheRoot({ env: {}, platform: "linux", homedir: "/home/u" }),
    path.join("/home/u", ".cache", "scythe"),
  );
  assert.equal(
    resolveCacheRoot({ env: {}, platform: "darwin", homedir: "/Users/u" }),
    path.join("/Users/u", ".cache", "scythe"),
  );
});

test("resolveCacheRoot uses LOCALAPPDATA on windows", () => {
  assert.equal(
    resolveCacheRoot({
      env: { LOCALAPPDATA: "C:\\Users\\u\\AppData\\Local" },
      platform: "win32",
      homedir: "C:\\Users\\u",
    }),
    path.join("C:\\Users\\u\\AppData\\Local", "scythe", "cache"),
  );
});

test("resolveCacheRoot falls back on windows without LOCALAPPDATA", () => {
  assert.equal(
    resolveCacheRoot({ env: {}, platform: "win32", homedir: "C:\\Users\\u" }),
    path.join("C:\\Users\\u", "AppData", "Local", "scythe", "cache"),
  );
});

test("cachedBinaryPath nests by version and binary name", () => {
  assert.equal(
    cachedBinaryPath({ env: {}, platform: "linux", homedir: "/home/u", version: "0.13.0", binaryName: "scythe" }),
    path.join("/home/u", ".cache", "scythe", "0.13.0", "scythe"),
  );
});

/**
 * @param {import("node:test").TestContext} t
 * @returns {string}
 */
function tempDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "scythe-cli-cache-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("isUsableCachedBinary accepts a complete, executable binary", (t) => {
  const file = path.join(tempDir(t), "scythe");
  fs.writeFileSync(file, "binary", { mode: 0o755 });
  assert.equal(isUsableCachedBinary(file, "linux"), true);
});

test("isUsableCachedBinary rejects a zero-length file left by an interrupted write", (t) => {
  const file = path.join(tempDir(t), "scythe");
  fs.writeFileSync(file, "", { mode: 0o755 });
  assert.equal(isUsableCachedBinary(file, "linux"), false, "an empty cache entry must not be trusted");
});

test("isUsableCachedBinary rejects a truncated non-executable leftover", (t) => {
  // The pre-atomic installer wrote the binary first and chmodded afterwards,
  // so a crash in between left exactly this: partial content, mode 0o644.
  const file = path.join(tempDir(t), "scythe");
  fs.writeFileSync(file, "\x7fELF-partial", { mode: 0o644 });
  assert.equal(isUsableCachedBinary(file, "linux"), false);
  assert.equal(isUsableCachedBinary(file, "win32"), true, "windows has no executable bit to check");
});

test("isUsableCachedBinary rejects a missing path and a directory", (t) => {
  const dir = tempDir(t);
  assert.equal(isUsableCachedBinary(path.join(dir, "absent"), "linux"), false);
  assert.equal(isUsableCachedBinary(dir, "linux"), false);
});
