"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { parseChecksums, expectedChecksum, sha256Hex, verifyChecksum } = require("../lib/checksum");

const SAMPLE = `\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  scythe-x86_64-unknown-linux-gnu.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  scythe-aarch64-apple-darwin.tar.gz
CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC  scythe-x86_64-pc-windows-gnu.zip
`;

test("parseChecksums parses filename -> lowercase hash rows", () => {
  const map = parseChecksums(SAMPLE);
  assert.equal(map.get("scythe-x86_64-unknown-linux-gnu.tar.gz"), "a".repeat(64));
  assert.equal(map.get("scythe-aarch64-apple-darwin.tar.gz"), "b".repeat(64));
  assert.equal(map.get("scythe-x86_64-pc-windows-gnu.zip"), "c".repeat(64));
});

test("parseChecksums skips blank lines", () => {
  const map = parseChecksums(`\n${SAMPLE}\n\n`);
  assert.equal(map.size, 3);
});

test("expectedChecksum returns the hash for a known asset", () => {
  const map = parseChecksums(SAMPLE);
  assert.equal(expectedChecksum(map, "scythe-x86_64-unknown-linux-gnu.tar.gz", "url"), "a".repeat(64));
});

test("expectedChecksum throws on a missing row rather than skipping verification", () => {
  const map = parseChecksums(SAMPLE);
  assert.throws(
    () => expectedChecksum(map, "scythe-riscv64-unknown-linux-gnu.tar.gz", "https://example/checksums.txt"),
    /no checksum entry.*https:\/\/example\/checksums\.txt/s,
  );
});

test("sha256Hex matches a known digest", () => {
  assert.equal(sha256Hex(Buffer.from("")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
});

test("verifyChecksum passes on matching hash, case-insensitively", () => {
  const buf = Buffer.from("hello");
  const hex = sha256Hex(buf);
  assert.doesNotThrow(() => verifyChecksum(buf, hex.toUpperCase(), "https://example/asset.tar.gz"));
});

test("verifyChecksum throws naming both hashes and the url on mismatch", () => {
  const buf = Buffer.from("hello");
  assert.throws(
    () => verifyChecksum(buf, "0".repeat(64), "https://example/asset.tar.gz"),
    (err) => {
      assert.match(err.message, /https:\/\/example\/asset\.tar\.gz/);
      assert.match(err.message, /expected: 0{64}/);
      assert.match(err.message, new RegExp(`actual:\\s+${sha256Hex(buf)}`));
      return true;
    },
  );
});
