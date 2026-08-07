"use strict";

const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");

const { resolveProxyUrl, resolveCaFile } = require("./proxy");

const MAX_REDIRECTS = 5;

/* 303 is included because a GET that is redirected with 303 stays a GET. */
const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308]);

/**
 * Builds the proxy agent for a request to `url`, or undefined when no proxy
 * applies to that host.
 *
 * `https-proxy-agent` returns an `http.Agent` subclass. That is exactly what
 * `node:https`/`node:http` want, and exactly what global `fetch` (undici)
 * does *not* want -- undici needs a `Dispatcher` with a `.dispatch()` method,
 * so handing it an agent fails at request time with an opaque
 * "fetch failed". This module therefore uses `node:https` directly.
 *
 * @param {string} url
 * @param {NodeJS.ProcessEnv} env
 * @returns {import("node:http").Agent | undefined}
 */
function buildProxyAgent(url, env) {
  const proxyUrl = resolveProxyUrl(env, new URL(url).hostname);
  if (!proxyUrl) return undefined;
  const { HttpsProxyAgent } = require("https-proxy-agent");
  return new HttpsProxyAgent(proxyUrl);
}

/**
 * Reads the configured custom CA bundle, if any, so it can be passed to the
 * TLS layer as the `ca` request option. `NODE_EXTRA_CA_CERTS` also works via
 * Node's own startup handling, but `npm_config_cafile` has no such support
 * and only takes effect because it is read and applied here.
 *
 * @param {NodeJS.ProcessEnv} env
 * @param {(path: string) => Buffer} [readFile]
 * @returns {Buffer | undefined}
 * @throws {Error} when the configured bundle cannot be read.
 */
function loadCaBundle(env, readFile = fs.readFileSync) {
  const caFile = resolveCaFile(env);
  if (!caFile) return undefined;
  try {
    return readFile(caFile);
  } catch (err) {
    throw new Error(`scythe-cli: failed to read CA bundle ${caFile}: ${err.message}`);
  }
}

/**
 * Resolves a `Location` header against the URL it came from, rejecting
 * anything that is not http(s) -- a redirect must never be able to send the
 * downloader at a `file:` or other local-resource URL.
 *
 * @param {string} location
 * @param {string} currentUrl
 * @returns {string}
 */
function resolveRedirect(location, currentUrl) {
  let next;
  try {
    next = new URL(location, currentUrl);
  } catch {
    throw new Error(`scythe-cli: ${currentUrl} returned an unparseable redirect Location "${location}"`);
  }
  if (next.protocol !== "https:" && next.protocol !== "http:") {
    throw new Error(`scythe-cli: ${currentUrl} redirected to a non-http(s) URL "${next.toString()}"`);
  }
  return next.toString();
}

/**
 * Issues a single GET, without following redirects.
 *
 * @param {string} url
 * @param {{ env: NodeJS.ProcessEnv, ca?: Buffer }} opts
 * @returns {Promise<import("node:http").IncomingMessage>}
 */
function requestOnce(url, { env, ca }) {
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const transport = parsed.protocol === "http:" ? http : https;

    const options = {};
    const agent = buildProxyAgent(url, env);
    if (agent) options.agent = agent;
    if (ca) options.ca = ca;

    const request = transport.request(parsed, options, resolve);
    // Every transport-level failure (DNS, refused, TLS) surfaces here, and
    // undici/Node messages alone never name the URL that failed.
    request.on("error", (err) => reject(new Error(`scythe-cli: failed to download ${url}: ${err.message}`)));
    request.end();
  });
}

/**
 * @param {import("node:http").IncomingMessage} response
 * @param {string} url
 * @returns {Promise<Buffer>}
 */
function readBody(response, url) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    response.on("data", (chunk) => chunks.push(chunk));
    response.on("end", () => resolve(Buffer.concat(chunks)));
    response.on("error", (err) => reject(new Error(`scythe-cli: failed while reading ${url}: ${err.message}`)));
  });
}

/**
 * Downloads `url` into a Buffer, following redirects.
 *
 * Redirect following is mandatory, not a nicety: GitHub release download URLs
 * answer 302 with a signed asset-host Location, and `node:https` (unlike
 * `fetch`) does not follow redirects on its own.
 *
 * @param {string} url
 * @param {object} [opts]
 * @param {NodeJS.ProcessEnv} [opts.env]
 * @param {number} [opts.maxRedirects]
 * @param {(message: string) => void} [opts.log]
 * @returns {Promise<Buffer>}
 */
async function downloadBuffer(url, { env = process.env, maxRedirects = MAX_REDIRECTS, log } = {}) {
  const ca = loadCaBundle(env);
  if (ca && log) {
    log(`scythe-cli: using CA bundle from ${resolveCaFile(env)}\n`);
  }

  let currentUrl = url;

  for (let hop = 0; hop <= maxRedirects; hop += 1) {
    const response = await requestOnce(currentUrl, { env, ca });
    const status = response.statusCode ?? 0;

    if (REDIRECT_STATUS.has(status) && response.headers.location) {
      response.resume(); // drain, otherwise the socket is never released
      currentUrl = resolveRedirect(response.headers.location, currentUrl);
      continue;
    }

    if (status < 200 || status >= 300) {
      response.resume();
      throw new Error(`scythe-cli: failed to download ${describe(url, currentUrl)}: HTTP ${status}`);
    }

    return readBody(response, currentUrl);
  }

  throw new Error(`scythe-cli: failed to download ${url}: exceeded ${maxRedirects} redirects (last was ${currentUrl})`);
}

/**
 * Downloads `url` and decodes it as UTF-8 text.
 *
 * @param {string} url
 * @param {object} [opts] - same options as {@link downloadBuffer}.
 * @returns {Promise<string>}
 */
async function downloadText(url, opts) {
  const buffer = await downloadBuffer(url, opts);
  return buffer.toString("utf8");
}

/**
 * @param {string} url
 * @param {string} currentUrl
 * @returns {string}
 */
function describe(url, currentUrl) {
  return url === currentUrl ? url : `${url} (redirected to ${currentUrl})`;
}

module.exports = {
  buildProxyAgent,
  loadCaBundle,
  resolveRedirect,
  downloadBuffer,
  downloadText,
  MAX_REDIRECTS,
};
