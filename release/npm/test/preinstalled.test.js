"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { hasMatchingPathBinary } = require("../lib/preinstalled");

test("hasMatchingPathBinary returns true only on exact version equality", () => {
  const exec = () => Buffer.from("scythe 0.13.0\n");
  assert.equal(hasMatchingPathBinary("0.13.0", exec), true);
});

test("hasMatchingPathBinary rejects a newer PATH binary -- a pin must not be silently satisfied by >=", () => {
  const exec = () => Buffer.from("scythe 0.14.0\n");
  assert.equal(hasMatchingPathBinary("0.13.0", exec), false);
});

test("hasMatchingPathBinary returns false when scythe is not on PATH (ENOENT)", () => {
  const exec = () => {
    const err = new Error("ENOENT");
    err.code = "ENOENT";
    throw err;
  };
  assert.equal(hasMatchingPathBinary("0.13.0", exec), false);
});
