import assert from "node:assert/strict";
import http from "node:http";
import path from "node:path";
import test from "node:test";

import {
  WebDriverClient,
  WebDriverProtocolError,
  validateLoopbackEndpoint,
} from "./protocol.mjs";

const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

test("protocol refuses non-loopback endpoints [NFR-SEC-002]", () => {
  assert.throws(() => validateLoopbackEndpoint("https://127.0.0.1:4444"), /IP loopback/u);
  assert.throws(() => validateLoopbackEndpoint("http://example.test:4444"), /IP loopback/u);
  assert.equal(validateLoopbackEndpoint("http://127.0.0.1:4444").origin, "http://127.0.0.1:4444");
});

test("protocol sends standard Tauri W3C capabilities and cleans session [NFR-COMPAT-003]", async () => {
  const requests = [];
  const server = await startServer((request, body) => {
    requests.push({ method: request.method, url: request.url, body });
    if (request.method === "GET" && request.url === "/status") return { value: { ready: true } };
    if (request.method === "POST" && request.url === "/session") {
      return { value: { sessionId: "session-fixture", capabilities: {} } };
    }
    if (request.method === "POST" && request.url === "/session/session-fixture/element") {
      return { value: { [ELEMENT_KEY]: "element-fixture" } };
    }
    if (request.method === "GET" && request.url?.endsWith("/text")) {
      return { value: "CyberKindred" };
    }
    return { value: null };
  });
  const client = new WebDriverClient(server.endpoint);
  const application = path.resolve("target", "debug", "cyberkindred.exe");

  try {
    await client.status();
    assert.equal(await client.createSession(application), "session-fixture");
    const element = await client.findElement("#radio-title");
    assert.equal(await client.elementText(element), "CyberKindred");
    await client.click(element);
    await client.deleteSession();
  } finally {
    await server.close();
  }

  assert.deepEqual(requests[1].body, {
    capabilities: {
      alwaysMatch: {
        browserName: "wry",
        "tauri:options": { application },
      },
    },
  });
  assert.equal(requests.at(-1)?.method, "DELETE");
  assert.equal(requests.at(-1)?.url, "/session/session-fixture");
});

test("protocol errors never relay remote secret or path text [NFR-SEC-001]", async () => {
  const server = await startServer(() => ({
    status: 500,
    body: {
      value: {
        error: "session not created",
        message: "sk-protocol-canary C:\\Users\\Canary\\private.mp3",
      },
    },
  }));
  const client = new WebDriverClient(server.endpoint);

  try {
    await assert.rejects(
      client.status(),
      (error) =>
        error instanceof WebDriverProtocolError &&
        error.code === "session not created" &&
        !error.message.includes("sk-protocol-canary") &&
        !error.message.includes("Canary"),
    );
  } finally {
    await server.close();
  }
});

test("protocol enforces an absolute deadline against drip responses [NFR-REL-001]", async () => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "Content-Type": "application/json" });
    response.write('{"value":"');
    const drip = setInterval(() => response.write("x"), 5);
    response.once("close", () => clearInterval(drip));
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(typeof address === "object" && address !== null);
  const client = new WebDriverClient(`http://127.0.0.1:${address.port}`, { timeoutMs: 40 });

  try {
    await assert.rejects(client.status(), /timed out after 40ms/u);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

async function startServer(responder) {
  const server = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const text = Buffer.concat(chunks).toString("utf8");
    const body = text.length === 0 ? undefined : JSON.parse(text);
    const result = await responder(request, body);
    const status = result?.status ?? 200;
    const payload = JSON.stringify(result?.body ?? result);
    response.writeHead(status, {
      "Content-Type": "application/json",
      "Content-Length": Buffer.byteLength(payload),
    });
    response.end(payload);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  assert.ok(typeof address === "object" && address !== null);
  return {
    endpoint: `http://127.0.0.1:${address.port}`,
    close: () => new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  };
}
