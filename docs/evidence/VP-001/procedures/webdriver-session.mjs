// Zero-dependency W3C WebDriver client speaking to `tauri-driver` over the
// platform's global `fetch` (design.md D6: Node 22 is already pinned by
// `package.json`'s `devEngines`, and `pnpm-workspace.yaml` covers only
// `renderer`, so this file joins no workspace). WebdriverIO/Selenium and
// the Rust `fantoccini` crate were both rejected as heavier than this
// script needs (large dependency tree, or new workspace-member deps
// compiled and linted on all three CI runners).
//
// The pure request/response builders below are exported separately from
// the `fetch`-based transport functions so `webdriver-session.test.mjs`
// can exercise them with `node --test` and no network, process, or
// display -- the transport functions are the E2E-only path
// (design.md's Testing Strategy row: "the live browser-driving path
// only ... is evidence machinery, not a repository test").

/** Raised for any non-2xx WebDriver response or malformed response body. */
export class WebDriverProtocolError extends Error {
  constructor(message, { status, error, stacktrace } = {}) {
    super(message);
    this.name = 'WebDriverProtocolError';
    this.status = status;
    this.error = error;
    this.stacktrace = stacktrace;
  }
}

// -- Pure builders --

/**
 * The W3C "New Session" request body for `tauri-driver`: a `tauri:options`
 * vendor capability naming the application binary to launch (design.md
 * D5/D7's session-creation step). `args`, when non-empty, are appended
 * verbatim to the launched binary's own argv.
 */
export function buildNewSessionRequest(applicationPath, { args = [] } = {}) {
  if (typeof applicationPath !== 'string' || applicationPath.length === 0) {
    throw new TypeError('buildNewSessionRequest requires a non-empty applicationPath');
  }
  return {
    capabilities: {
      alwaysMatch: {
        'tauri:options': {
          application: applicationPath,
          ...(args.length > 0 ? { args } : {}),
        },
      },
    },
  };
}

/**
 * A W3C element-locator request body (`POST /session/:id/element`). `using`
 * is one of the W3C locator strategies (`css selector`, `xpath`, `link
 * text`, `partial link text`, `tag name`); this builder does not validate
 * the strategy name, matching the protocol's own permissiveness.
 */
export function buildElementLocator(using, value) {
  if (typeof using !== 'string' || using.length === 0 || typeof value !== 'string') {
    throw new TypeError('buildElementLocator requires a using strategy and a string value');
  }
  return { using, value };
}

/**
 * Join `baseUrl` and any number of path segments into one endpoint URL,
 * without the double/missing slash bugs a naive template literal invites.
 */
export function buildEndpointUrl(baseUrl, ...segments) {
  const trimmedBase = baseUrl.replace(/\/+$/, '');
  const trimmedSegments = segments
    .map((segment) => String(segment).replace(/^\/+|\/+$/g, ''))
    .filter((segment) => segment.length > 0);
  return [trimmedBase, ...trimmedSegments].join('/');
}

/** The W3C "web element reference" object key (WebDriver spec § 12). */
const WEB_ELEMENT_IDENTIFIER = 'element-6066-11e4-a52e-4f735466cecf';

/**
 * Unwrap a W3C response body's mandatory `value` field. Every successful
 * WebDriver response is `{ value: ... }`; a body with no `value` key at all
 * is malformed -- never a legitimate empty result, since W3C already has
 * `value: null` for that.
 */
export function unwrapValue(responseBody) {
  if (
    responseBody === null ||
    typeof responseBody !== 'object' ||
    !Object.hasOwn(responseBody, 'value')
  ) {
    throw new WebDriverProtocolError('WebDriver response body has no "value" field');
  }
  return responseBody.value;
}

/** Unwrap a `POST /session/:id/element` response into its bare element id. */
export function unwrapElementId(responseBody) {
  const value = unwrapValue(responseBody);
  const elementId = value?.[WEB_ELEMENT_IDENTIFIER];
  if (typeof elementId !== 'string') {
    throw new WebDriverProtocolError('WebDriver element response carries no element id');
  }
  return elementId;
}

/**
 * Map a non-2xx WebDriver HTTP response into a `WebDriverProtocolError`
 * carrying the protocol's own `error`/`message`/`stacktrace` triple
 * (W3C WebDriver "Handling Errors"), never a bare HTTP status.
 */
export function toWebDriverError(status, responseBody) {
  const value =
    responseBody !== null && typeof responseBody === 'object' ? responseBody.value : undefined;
  const errorCode = value?.error ?? 'unknown error';
  const message = value?.message ?? `WebDriver request failed with HTTP ${status}`;
  return new WebDriverProtocolError(message, {
    status,
    error: errorCode,
    stacktrace: value?.stacktrace,
  });
}

// -- fetch-based transport (E2E-only; not covered by node --test) --

async function request(baseUrl, method, pathSegments, body) {
  const url = buildEndpointUrl(baseUrl, ...pathSegments);
  const response = await fetch(url, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  const parsed = text.length > 0 ? JSON.parse(text) : null;
  if (!response.ok) {
    throw toWebDriverError(response.status, parsed);
  }
  return parsed;
}

/** Create a new WebDriver session against `applicationPath`; returns the session id. */
export async function createSession(baseUrl, applicationPath, options) {
  const body = buildNewSessionRequest(applicationPath, options);
  const responseBody = await request(baseUrl, 'POST', ['session'], body);
  const value = unwrapValue(responseBody);
  if (typeof value?.sessionId !== 'string') {
    throw new WebDriverProtocolError('New Session response carries no sessionId');
  }
  return value.sessionId;
}

/** End a session. */
export async function deleteSession(baseUrl, sessionId) {
  await request(baseUrl, 'DELETE', ['session', sessionId], undefined);
}

/** Locate one element by `using`/`value`; returns its element id. */
export async function findElement(baseUrl, sessionId, using, value) {
  const body = buildElementLocator(using, value);
  const responseBody = await request(baseUrl, 'POST', ['session', sessionId, 'element'], body);
  return unwrapElementId(responseBody);
}

/** Click a previously located element. */
export async function clickElement(baseUrl, sessionId, elementId) {
  await request(baseUrl, 'POST', ['session', sessionId, 'element', elementId, 'click'], {});
}

/** Send keystrokes to a previously located element (e.g. a text input). */
export async function sendKeysToElement(baseUrl, sessionId, elementId, text) {
  await request(baseUrl, 'POST', ['session', sessionId, 'element', elementId, 'value'], { text });
}

/** Read an element's rendered text. */
export async function getElementText(baseUrl, sessionId, elementId) {
  const responseBody = await request(
    baseUrl,
    'GET',
    ['session', sessionId, 'element', elementId, 'text'],
    undefined,
  );
  return unwrapValue(responseBody);
}
