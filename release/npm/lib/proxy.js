"use strict";

/**
 * Resolves the proxy URL to use for HTTPS requests, honouring (in priority
 * order) explicit env proxy vars and npm's own config vars, since a
 * corporate user has typically already configured npm. `NO_PROXY` disables
 * proxying for matching hosts.
 *
 * Neither `node:https` nor Node's built-in `fetch` honours `HTTPS_PROXY` on
 * its own -- callers must pass an explicit agent built from this URL.
 *
 * @param {NodeJS.ProcessEnv} env
 * @param {string} [targetHost]
 * @returns {string | null}
 */
function resolveProxyUrl(env, targetHost) {
  const proxy = env.HTTPS_PROXY || env.https_proxy || env.npm_config_https_proxy || env.npm_config_proxy || null;

  if (!proxy) return null;

  const noProxy = env.NO_PROXY || env.no_proxy || "";
  if (targetHost && isNoProxyMatch(targetHost, noProxy)) {
    return null;
  }

  return proxy;
}

/**
 * @param {string} host
 * @param {string} noProxyList - comma-separated list, e.g. "localhost,.corp.internal"
 * @returns {boolean}
 */
function isNoProxyMatch(host, noProxyList) {
  const entries = noProxyList
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);

  return entries.some((entry) => {
    if (entry === "*") return true;
    const pattern = entry.startsWith(".") ? entry : `.${entry}`;
    return host === entry || host.endsWith(pattern);
  });
}

/**
 * Resolves the CA bundle file path to trust, honouring npm's own config.
 * The file is read and passed to the TLS layer by `lib/download.js`;
 * returning a path here has no effect on its own.
 *
 * @param {NodeJS.ProcessEnv} env
 * @returns {string | null}
 */
function resolveCaFile(env) {
  return env.npm_config_cafile || env.NODE_EXTRA_CA_CERTS || null;
}

module.exports = { resolveProxyUrl, isNoProxyMatch, resolveCaFile };
