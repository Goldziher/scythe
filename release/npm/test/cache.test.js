"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const { test } = require("node:test");

const { resolveCacheRoot, cachedBinaryPath } = require("../lib/cache");

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
