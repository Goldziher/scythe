"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { resolveProxyUrl, isNoProxyMatch, resolveCaFile } = require("../lib/proxy");

test("resolveProxyUrl prefers HTTPS_PROXY", () => {
  assert.equal(resolveProxyUrl({ HTTPS_PROXY: "http://a:1", npm_config_proxy: "http://b:2" }), "http://a:1");
});

test("resolveProxyUrl falls back to npm's own config vars", () => {
  assert.equal(resolveProxyUrl({ npm_config_https_proxy: "http://c:3" }), "http://c:3");
  assert.equal(resolveProxyUrl({ npm_config_proxy: "http://d:4" }), "http://d:4");
});

test("resolveProxyUrl returns null when nothing is configured", () => {
  assert.equal(resolveProxyUrl({}), null);
});

test("resolveProxyUrl honours NO_PROXY for the target host", () => {
  assert.equal(resolveProxyUrl({ HTTPS_PROXY: "http://a:1", NO_PROXY: "github.com" }, "github.com"), null);
  assert.equal(
    resolveProxyUrl({ HTTPS_PROXY: "http://a:1", NO_PROXY: "github.com" }, "objects.githubusercontent.com"),
    "http://a:1",
  );
});

test("isNoProxyMatch matches exact host and suffix entries", () => {
  assert.equal(isNoProxyMatch("github.com", "github.com"), true);
  assert.equal(isNoProxyMatch("api.github.com", "github.com"), true);
  assert.equal(isNoProxyMatch("evilgithub.com", "github.com"), false);
  assert.equal(isNoProxyMatch("anything.local", "*"), true);
  assert.equal(isNoProxyMatch("example.com", ""), false);
});

test("resolveCaFile honours npm's cafile config and NODE_EXTRA_CA_CERTS", () => {
  assert.equal(resolveCaFile({ npm_config_cafile: "/ca.pem" }), "/ca.pem");
  assert.equal(resolveCaFile({ NODE_EXTRA_CA_CERTS: "/ca2.pem" }), "/ca2.pem");
  assert.equal(resolveCaFile({}), null);
});
