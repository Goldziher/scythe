"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { extractVersion, assertRealVersion } = require("../lib/version");

test("extractVersion parses the real scythe --version output", () => {
  assert.equal(extractVersion("scythe 0.12.0\n"), "0.12.0");
});

test("extractVersion handles pre-release and build metadata permissively", () => {
  assert.equal(extractVersion("scythe 0.13.0-rc.1"), "0.13.0-rc.1");
  assert.equal(extractVersion("scythe 0.13.0+abcdef"), "0.13.0+abcdef");
});

test("extractVersion returns null when no version-shaped token is present", () => {
  assert.equal(extractVersion("command not found"), null);
});

test("assertRealVersion throws on the 0.0.0 placeholder", () => {
  assert.throws(() => assertRealVersion("0.0.0"), /built incorrectly/);
});

test("assertRealVersion accepts a real version", () => {
  assert.doesNotThrow(() => assertRealVersion("0.13.0"));
});
