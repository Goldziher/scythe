"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { hasMatchingPathBinary } = require("../lib/preinstalled");

const execV13 = () => Buffer.from("scythe 0.13.0\n");
const execV14 = () => Buffer.from("scythe 0.14.0\n");
const execEnoent = () => {
  const err = new Error("ENOENT");
  err.code = "ENOENT";
  throw err;
};

test("hasMatchingPathBinary returns true only on exact version equality", () => {
  assert.equal(hasMatchingPathBinary("0.13.0", execV13), true);
});

test("hasMatchingPathBinary rejects a newer PATH binary -- a pin must not be silently satisfied by >=", () => {
  assert.equal(hasMatchingPathBinary("0.13.0", execV14), false);
});

test("hasMatchingPathBinary returns false when scythe is not on PATH (ENOENT)", () => {
  assert.equal(hasMatchingPathBinary("0.13.0", execEnoent), false);
});
