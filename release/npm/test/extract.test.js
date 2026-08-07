"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { findBinaryEntry } = require("../lib/extract");

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
