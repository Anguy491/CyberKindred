import http from "node:http";
import path from "node:path";

const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";
const MAX_RESPONSE_BYTES = 1_048_576;

export class WebDriverProtocolError extends Error {
  constructor(code, status) {
    super(`WebDriver request failed (${code}, HTTP ${status}).`);
    this.name = "WebDriverProtocolError";
    this.code = code;
    this.status = status;
  }
}

export class WebDriverClient {
  #endpoint;
  #sessionId;
  #timeoutMs;

  constructor(endpoint, { timeoutMs = 10_000, sessionId } = {}) {
    this.#endpoint = validateLoopbackEndpoint(endpoint);
    if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 60_000) {
      throw new Error("WebDriver request timeout must be an integer from 1 to 60000 milliseconds.");
    }
    this.#timeoutMs = timeoutMs;
    this.#sessionId = sessionId;
  }

  get sessionId() {
    return this.#sessionId;
  }

  async status() {
    return this.#request("GET", "/status");
  }

  async createSession(applicationPath) {
    if (this.#sessionId !== undefined) {
      throw new Error("The WebDriver client already owns a session.");
    }
    if (!path.isAbsolute(applicationPath)) {
      throw new Error("The Tauri application capability must be an absolute path.");
    }
    const response = await this.#request("POST", "/session", {
      capabilities: {
        alwaysMatch: {
          browserName: "wry",
          "tauri:options": { application: applicationPath },
        },
      },
    });
    const sessionId = response?.sessionId ?? response?.value?.sessionId;
    if (typeof sessionId !== "string" || sessionId.length === 0) {
      throw new WebDriverProtocolError("invalid_session_response", 200);
    }
    this.#sessionId = sessionId;
    return sessionId;
  }

  async deleteSession() {
    if (this.#sessionId === undefined) return;
    const sessionId = this.#sessionId;
    this.#sessionId = undefined;
    await this.#request("DELETE", `/session/${encodeURIComponent(sessionId)}`);
  }

  async findElement(selector) {
    const response = await this.#sessionRequest("POST", "/element", {
      using: "css selector",
      value: selector,
    });
    const elementId = response?.value?.[ELEMENT_KEY];
    if (typeof elementId !== "string" || elementId.length === 0) {
      throw new WebDriverProtocolError("invalid_element_response", 200);
    }
    return elementId;
  }

  async waitForElement(selector, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
      try {
        return await this.findElement(selector);
      } catch (error) {
        lastError = error;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
    }
    throw new Error(
      `Desktop element did not become available within ${timeoutMs}ms (${safeErrorCode(lastError)}).`,
    );
  }

  async click(elementId) {
    await this.#sessionRequest("POST", `/element/${encodeURIComponent(elementId)}/click`, {});
  }

  async elementText(elementId) {
    const response = await this.#sessionRequest(
      "GET",
      `/element/${encodeURIComponent(elementId)}/text`,
    );
    if (typeof response?.value !== "string") {
      throw new WebDriverProtocolError("invalid_text_response", 200);
    }
    return response.value;
  }

  async elementAttribute(elementId, name) {
    const response = await this.#sessionRequest(
      "GET",
      `/element/${encodeURIComponent(elementId)}/attribute/${encodeURIComponent(name)}`,
    );
    if (response?.value !== null && typeof response?.value !== "string") {
      throw new WebDriverProtocolError("invalid_attribute_response", 200);
    }
    return response.value;
  }

  async #sessionRequest(method, suffix, body) {
    if (this.#sessionId === undefined) {
      throw new Error("A WebDriver session has not been created.");
    }
    return this.#request(method, `/session/${encodeURIComponent(this.#sessionId)}${suffix}`, body);
  }

  async #request(method, pathname, body) {
    const url = new URL(pathname, this.#endpoint);
    const payload = body === undefined ? undefined : JSON.stringify(body);
    const parsed = await requestJson(url, method, payload, this.#timeoutMs);
    if (parsed.status < 200 || parsed.status >= 300) {
      const code =
        typeof parsed.body?.value?.error === "string" ? parsed.body.value.error : "request_failed";
      throw new WebDriverProtocolError(code, parsed.status);
    }
    return parsed.body;
  }
}

export function validateLoopbackEndpoint(endpoint) {
  const url = new URL(endpoint);
  const loopback = url.hostname === "127.0.0.1" || url.hostname === "[::1]";
  if (url.protocol !== "http:" || !loopback || url.username !== "" || url.password !== "") {
    throw new Error("WebDriver endpoint must be unauthenticated HTTP on an IP loopback address.");
  }
  url.pathname = "/";
  url.search = "";
  url.hash = "";
  return url;
}

function requestJson(url, method, payload, timeoutMs) {
  return new Promise((resolve, reject) => {
    let settled = false;
    let deadline;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(deadline);
      callback(value);
    };
    const request = http.request(
      url,
      {
        method,
        agent: false,
        headers:
          payload === undefined
            ? { Accept: "application/json" }
            : {
                Accept: "application/json",
                "Content-Type": "application/json; charset=utf-8",
                "Content-Length": Buffer.byteLength(payload),
              },
      },
      (response) => {
        const chunks = [];
        let bytes = 0;
        response.on("data", (chunk) => {
          bytes += chunk.length;
          if (bytes > MAX_RESPONSE_BYTES) {
            response.destroy(new Error("WebDriver response exceeded the bounded size."));
            return;
          }
          chunks.push(chunk);
        });
        response.on("end", () => {
          try {
            const text = Buffer.concat(chunks).toString("utf8");
            finish(resolve, {
              status: response.statusCode ?? 0,
              body: text.length === 0 ? {} : JSON.parse(text),
            });
          } catch {
            finish(
              reject,
              new WebDriverProtocolError("invalid_json_response", response.statusCode ?? 0),
            );
          }
        });
        response.on("error", (error) => finish(reject, error));
      },
    );
    deadline = setTimeout(() => {
      request.destroy(new Error(`WebDriver request timed out after ${timeoutMs}ms.`));
    }, timeoutMs);
    request.on("error", (error) => finish(reject, error));
    if (payload !== undefined) request.write(payload);
    request.end();
  });
}

function safeErrorCode(error) {
  return error instanceof WebDriverProtocolError ? error.code : "request_unavailable";
}
