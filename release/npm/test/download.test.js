"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const { buildProxyAgent, loadCaBundle, resolveRedirect, downloadBuffer, downloadText } = require("../lib/download");

/**
 * Starts a loopback HTTP origin server.
 *
 * @param {import("node:http").RequestListener} handler
 * @returns {Promise<{ origin: string, close: () => Promise<void> }>}
 */
async function startServer(handler) {
  const server = http.createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

/**
 * Starts a minimal CONNECT-tunnelling proxy, recording every tunnel target so
 * a test can prove the request really travelled through it.
 *
 * @returns {Promise<{ url: string, tunnels: string[], close: () => Promise<void> }>}
 */
async function startConnectProxy() {
  const tunnels = [];
  const server = net.createServer((socket) => {
    socket.once("data", (chunk) => {
      const match = chunk
        .toString("utf8")
        .split("\r\n")[0]
        .match(/^CONNECT (\S+):(\d+)/);
      if (!match) {
        socket.destroy();
        return;
      }
      tunnels.push(`${match[1]}:${match[2]}`);
      const upstream = net.connect(Number(match[2]), match[1], () => {
        socket.write("HTTP/1.1 200 Connection established\r\n\r\n");
        socket.pipe(upstream);
        upstream.pipe(socket);
      });
      upstream.on("error", () => socket.destroy());
    });
    socket.on("error", () => {});
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    tunnels,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

test("downloadBuffer returns the response bytes from a real server", async (t) => {
  const server = await startServer((req, res) => {
    res.writeHead(200, { "content-type": "application/octet-stream" });
    res.end(Buffer.from([0, 1, 2, 253, 254, 255]));
  });
  t.after(() => server.close());

  const buffer = await downloadBuffer(`${server.origin}/asset.tar.gz`, { env: {} });
  assert.deepEqual([...buffer], [0, 1, 2, 253, 254, 255]);
});

test("downloadText decodes the body as UTF-8", async (t) => {
  const body = "abc123  scythe-x86_64-apple-darwin.tar.gz\n";
  const server = await startServer((req, res) => res.end(body));
  t.after(() => server.close());

  assert.equal(await downloadText(`${server.origin}/checksums.txt`, { env: {} }), body);
});

test("downloadBuffer follows a 302 redirect, as GitHub release URLs require", async (t) => {
  let assetHits = 0;
  const asset = await startServer((req, res) => {
    assetHits += 1;
    res.end("redirected-payload");
  });
  t.after(() => asset.close());

  const release = await startServer((req, res) => {
    res.writeHead(302, { location: `${asset.origin}/signed-asset` });
    res.end();
  });
  t.after(() => release.close());

  const buffer = await downloadBuffer(`${release.origin}/releases/download/v1/asset`, { env: {} });
  assert.equal(buffer.toString(), "redirected-payload");
  assert.equal(assetHits, 1);
});

test("downloadBuffer resolves relative redirect targets against the current URL", async (t) => {
  const server = await startServer((req, res) => {
    if (req.url === "/start") {
      res.writeHead(302, { location: "/final" });
      res.end();
      return;
    }
    res.end("relative-ok");
  });
  t.after(() => server.close());

  const buffer = await downloadBuffer(`${server.origin}/start`, { env: {} });
  assert.equal(buffer.toString(), "relative-ok");
});

test("downloadBuffer gives up on a redirect loop instead of hanging", async (t) => {
  const server = await startServer((req, res) => {
    res.writeHead(302, { location: "/again" });
    res.end();
  });
  t.after(() => server.close());

  await assert.rejects(
    () => downloadBuffer(`${server.origin}/start`, { env: {}, maxRedirects: 3 }),
    /exceeded 3 redirects/,
  );
});

test("downloadBuffer refuses a redirect that leaves http(s)", async (t) => {
  const server = await startServer((req, res) => {
    res.writeHead(302, { location: "file:///etc/passwd" });
    res.end();
  });
  t.after(() => server.close());

  await assert.rejects(() => downloadBuffer(`${server.origin}/start`, { env: {} }), /non-http\(s\) URL/);
});

test("downloadBuffer names the URL and status on an HTTP error", async (t) => {
  const server = await startServer((req, res) => {
    res.writeHead(404);
    res.end();
  });
  t.after(() => server.close());

  const url = `${server.origin}/missing.tar.gz`;
  await assert.rejects(
    () => downloadBuffer(url, { env: {} }),
    (err) => {
      assert.match(err.message, /HTTP 404/);
      assert.ok(err.message.includes(url), `expected the URL in: ${err.message}`);
      return true;
    },
  );
});

test("downloadBuffer names the URL on a transport failure, not a bare 'fetch failed'", async () => {
  const server = await startServer((req, res) => res.end());
  await server.close(); // nothing is listening on this port any more

  const url = `${server.origin}/asset.tar.gz`;
  await assert.rejects(
    () => downloadBuffer(url, { env: {} }),
    (err) => {
      assert.ok(err.message.includes(url), `expected the URL in: ${err.message}`);
      assert.match(err.message, /ECONNREFUSED/);
      assert.doesNotMatch(err.message, /^fetch failed$/);
      return true;
    },
  );
});

test("downloadBuffer names the redirect target when the failure happens after a hop", async (t) => {
  const dead = await startServer((req, res) => res.end());
  await dead.close();

  const release = await startServer((req, res) => {
    res.writeHead(302, { location: `${dead.origin}/signed` });
    res.end();
  });
  t.after(() => release.close());

  await assert.rejects(
    () => downloadBuffer(`${release.origin}/start`, { env: {} }),
    (err) => {
      assert.ok(err.message.includes(`${dead.origin}/signed`), `expected the hop URL in: ${err.message}`);
      return true;
    },
  );
});

test("downloadBuffer routes the request through a configured proxy", async (t) => {
  const origin = await startServer((req, res) => res.end("through-the-proxy"));
  t.after(() => origin.close());
  const proxy = await startConnectProxy();
  t.after(() => proxy.close());

  const originPort = new URL(origin.origin).port;
  const buffer = await downloadBuffer(`${origin.origin}/asset`, { env: { HTTPS_PROXY: proxy.url } });

  assert.equal(buffer.toString(), "through-the-proxy");
  assert.deepEqual(proxy.tunnels, [`127.0.0.1:${originPort}`]);
});

test("downloadBuffer bypasses the proxy for NO_PROXY hosts", async (t) => {
  const origin = await startServer((req, res) => res.end("direct"));
  t.after(() => origin.close());
  const proxy = await startConnectProxy();
  t.after(() => proxy.close());

  const buffer = await downloadBuffer(`${origin.origin}/asset`, {
    env: { HTTPS_PROXY: proxy.url, NO_PROXY: "127.0.0.1" },
  });

  assert.equal(buffer.toString(), "direct");
  assert.deepEqual(proxy.tunnels, []);
});

test("buildProxyAgent returns an http.Agent, which is what the request layer requires", () => {
  // The last assertion guards the original defect: this object used to be
  // handed to global fetch as a `dispatcher`, but undici dispatchers need a
  // `.dispatch()` method, which an http.Agent does not and will not have.
  const agent = buildProxyAgent("https://github.com/x", { HTTPS_PROXY: "http://proxy.local:3128" });

  assert.ok(agent instanceof http.Agent);
  assert.equal(typeof agent.addRequest, "function");
  assert.equal(typeof agent.dispatch, "undefined");
});

test("buildProxyAgent returns undefined when no proxy applies", () => {
  assert.equal(buildProxyAgent("https://github.com/x", {}), undefined);
  assert.equal(
    buildProxyAgent("https://github.com/x", { HTTPS_PROXY: "http://p:1", NO_PROXY: "github.com" }),
    undefined,
  );
});

test("loadCaBundle reads the file named by npm_config_cafile", () => {
  const bundle = loadCaBundle({ npm_config_cafile: "/ca.pem" }, (file) => Buffer.from(`pem:${file}`));
  assert.equal(bundle.toString(), "pem:/ca.pem");
  assert.equal(
    loadCaBundle({}, () => assert.fail("must not read anything")),
    undefined,
  );
});

test("loadCaBundle reports an unreadable CA bundle instead of silently ignoring it", () => {
  const missing = path.join(os.tmpdir(), "scythe-cli-no-such-ca.pem");
  assert.throws(() => loadCaBundle({ npm_config_cafile: missing }), new RegExp(`failed to read CA bundle ${missing}`));
});

test("downloadBuffer surfaces a broken npm_config_cafile rather than logging a false reassurance", async (t) => {
  const server = await startServer((req, res) => res.end("never-reached"));
  t.after(() => server.close());

  await assert.rejects(
    () => downloadBuffer(`${server.origin}/x`, { env: { npm_config_cafile: "/definitely/not/here.pem" } }),
    /failed to read CA bundle/,
  );
});

test("resolveRedirect resolves against the current URL and rejects junk", () => {
  assert.equal(resolveRedirect("/b", "https://a.test/a"), "https://a.test/b");
  assert.equal(resolveRedirect("https://c.test/d", "https://a.test/a"), "https://c.test/d");
  assert.throws(() => resolveRedirect("file:///etc/passwd", "https://a.test/a"), /non-http\(s\)/);
  assert.throws(() => resolveRedirect("", "not a url"), /unparseable redirect Location/);
});

/**
 * Generates a throwaway self-signed cert. Returns null when openssl is
 * unavailable, so the suite degrades to a skip rather than a false failure.
 *
 * @param {string} dir
 * @returns {{ key: Buffer, cert: Buffer, certPath: string } | null}
 */
function generateSelfSignedCert(dir) {
  const keyPath = path.join(dir, "key.pem");
  const certPath = path.join(dir, "cert.pem");
  try {
    const args = ["req", "-x509", "-newkey", "rsa:2048", "-nodes"];
    args.push("-keyout", keyPath, "-out", certPath, "-days", "1");
    args.push("-subj", "/CN=localhost");
    args.push("-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1");
    execFileSync("openssl", args, { stdio: "ignore" });
  } catch {
    return null;
  }
  return { key: fs.readFileSync(keyPath), cert: fs.readFileSync(certPath), certPath };
}

test("downloadBuffer actually applies npm_config_cafile to the TLS handshake", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "scythe-cli-ca-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));

  const generated = generateSelfSignedCert(dir);
  if (!generated) {
    t.skip("openssl is not available to mint a test certificate");
    return;
  }

  const server = https.createServer({ key: generated.key, cert: generated.cert }, (req, res) => res.end("tls-ok"));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));

  const url = `https://localhost:${server.address().port}/asset`;

  // Trusting only the system roots, this self-signed cert must be rejected.
  await assert.rejects(() => downloadBuffer(url, { env: {} }), /self.signed|SELF_SIGNED|unable to verify/i);

  // With the bundle configured, the same request must succeed -- which is
  // only possible if the file is read and handed to the TLS layer.
  const buffer = await downloadBuffer(url, { env: { npm_config_cafile: generated.certPath } });
  assert.equal(buffer.toString(), "tls-ok");
});
