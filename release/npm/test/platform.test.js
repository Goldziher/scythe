"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { resolveTarget, isMuslLinux, TRIPLES } = require("../lib/platform");

test("resolveTarget maps all five known platform/arch pairs", () => {
  assert.equal(resolveTarget({ platform: "linux", arch: "x64" }).target, "x86_64-unknown-linux-gnu");
  assert.equal(resolveTarget({ platform: "linux", arch: "arm64" }).target, "aarch64-unknown-linux-gnu");
  assert.equal(resolveTarget({ platform: "darwin", arch: "x64" }).target, "x86_64-apple-darwin");
  assert.equal(resolveTarget({ platform: "darwin", arch: "arm64" }).target, "aarch64-apple-darwin");
  assert.equal(resolveTarget({ platform: "win32", arch: "x64" }).target, "x86_64-pc-windows-gnu");
});

test("resolveTarget picks tar.gz for unix and zip for windows, with correct binary name", () => {
  assert.deepEqual(resolveTarget({ platform: "linux", arch: "x64" }), {
    target: "x86_64-unknown-linux-gnu",
    archiveExt: "tar.gz",
    binaryName: "scythe",
  });
  assert.deepEqual(resolveTarget({ platform: "win32", arch: "x64" }), {
    target: "x86_64-pc-windows-gnu",
    archiveExt: "zip",
    binaryName: "scythe.exe",
  });
});

test("resolveTarget hard-fails on musl linux, naming the platform and cargo fallback", () => {
  assert.throws(
    () => resolveTarget({ platform: "linux", arch: "x64", isMusl: () => true }),
    /musl libc.*cargo install scythe-cli/s,
  );
});

test("resolveTarget does not consult isMusl on non-linux platforms", () => {
  const isMusl = () => {
    throw new Error("should not be called");
  };
  assert.doesNotThrow(() => resolveTarget({ platform: "darwin", arch: "arm64", isMusl }));
});

test("resolveTarget falls back to x64 with a warning on win32/arm64", () => {
  const warnings = [];
  const result = resolveTarget({ platform: "win32", arch: "arm64", warn: (msg) => warnings.push(msg) });
  assert.deepEqual(result, { target: "x86_64-pc-windows-gnu", archiveExt: "zip", binaryName: "scythe.exe" });
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /ARM64/);
});

test("resolveTarget hard-errors on unknown platform/arch, naming both", () => {
  assert.throws(() => resolveTarget({ platform: "freebsd", arch: "x64" }), /freebsd\/x64/);
  assert.throws(() => resolveTarget({ platform: "linux", arch: "ia32" }), /linux\/ia32/);
});

test("isMuslLinux detects absence of glibcVersionRuntime", () => {
  assert.equal(
    isMuslLinux(() => ({ header: {} })),
    true,
  );
  assert.equal(
    isMuslLinux(() => ({ header: { glibcVersionRuntime: "2.35" } })),
    false,
  );
  assert.equal(
    isMuslLinux(() => ({})),
    true,
  );
});

test("TRIPLES has no musl or windows-arm64 entries", () => {
  const keys = Object.keys(TRIPLES);
  assert.equal(
    keys.some((k) => k.includes("musl")),
    false,
  );
  assert.equal(keys.includes("win32-arm64"), false);
});
