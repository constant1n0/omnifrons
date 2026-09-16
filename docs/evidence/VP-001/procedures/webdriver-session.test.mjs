// `node --test` unit tests for `webdriver-session.mjs`'s pure builders --
// written RED, before that module exists (design.md D6, Testing Strategy).
// These exercise no network, process, or display: the `fetch`-based
// transport functions in the same module are the E2E-only path
// (design.md's Testing Strategy row), never covered here.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  buildElementLocator,
  buildEndpointUrl,
  buildExecuteScriptRequest,
  buildNewSessionRequest,
  toWebDriverError,
  unwrapElementId,
  unwrapValue,
  WebDriverProtocolError,
} from './webdriver-session.mjs';

test('buildNewSessionRequest', async (t) => {
  await t.test('builds a tauri:options capabilities body for the given application', () => {
    const body = buildNewSessionRequest('/path/to/app');
    assert.deepEqual(body, {
      capabilities: {
        alwaysMatch: {
          'tauri:options': { application: '/path/to/app' },
        },
      },
    });
  });

  await t.test('includes args only when at least one is given', () => {
    const body = buildNewSessionRequest('/path/to/app', { args: ['--flag'] });
    assert.deepEqual(body.capabilities.alwaysMatch['tauri:options'], {
      application: '/path/to/app',
      args: ['--flag'],
    });
  });

  await t.test('omits args entirely when none are given', () => {
    const body = buildNewSessionRequest('/path/to/app', { args: [] });
    assert.deepEqual(body.capabilities.alwaysMatch['tauri:options'], {
      application: '/path/to/app',
    });
  });

  await t.test('rejects an empty or missing application path', () => {
    assert.throws(() => buildNewSessionRequest(''), TypeError);
    assert.throws(() => buildNewSessionRequest(undefined), TypeError);
  });
});

test('buildElementLocator', async (t) => {
  await t.test('builds a using/value locator body', () => {
    assert.deepEqual(buildElementLocator('css selector', '#agent-adapter'), {
      using: 'css selector',
      value: '#agent-adapter',
    });
  });

  await t.test('rejects a missing strategy or a non-string value', () => {
    assert.throws(() => buildElementLocator('', '#id'), TypeError);
    assert.throws(() => buildElementLocator('css selector', 42), TypeError);
  });
});

test('buildEndpointUrl', async (t) => {
  await t.test('joins a base URL and segments with exactly one slash each', () => {
    assert.equal(
      buildEndpointUrl('http://127.0.0.1:4444', 'session', 'abc123', 'element'),
      'http://127.0.0.1:4444/session/abc123/element',
    );
  });

  await t.test(
    'tolerates a trailing slash on the base and leading/trailing slashes on segments',
    () => {
      assert.equal(
        buildEndpointUrl('http://127.0.0.1:4444/', '/session/', '/abc123/'),
        'http://127.0.0.1:4444/session/abc123',
      );
    },
  );

  await t.test('returns just the base when given no segments', () => {
    assert.equal(buildEndpointUrl('http://127.0.0.1:4444'), 'http://127.0.0.1:4444');
  });
});

test('unwrapValue', async (t) => {
  await t.test('returns the value field of a well-formed response', () => {
    assert.deepEqual(unwrapValue({ value: { sessionId: 'abc' } }), { sessionId: 'abc' });
  });

  await t.test('accepts an explicit null value (a legitimate W3C empty result)', () => {
    assert.equal(unwrapValue({ value: null }), null);
  });

  await t.test('rejects a body with no value field at all', () => {
    assert.throws(() => unwrapValue({}), WebDriverProtocolError);
    assert.throws(() => unwrapValue(null), WebDriverProtocolError);
  });
});

test('unwrapElementId', async (t) => {
  await t.test('extracts the W3C web-element identifier key', () => {
    const elementId = unwrapElementId({
      value: { 'element-6066-11e4-a52e-4f735466cecf': 'elem-1' },
    });
    assert.equal(elementId, 'elem-1');
  });

  await t.test('rejects a value carrying no element id', () => {
    assert.throws(() => unwrapElementId({ value: {} }), WebDriverProtocolError);
  });
});

test('buildExecuteScriptRequest', async (t) => {
  await t.test('builds a script/args body, defaulting args to empty', () => {
    assert.deepEqual(buildExecuteScriptRequest('return 1;'), { script: 'return 1;', args: [] });
  });

  await t.test('carries given args through verbatim', () => {
    const body = buildExecuteScriptRequest('return arguments[0];', ['#agent-adapter']);
    assert.deepEqual(body, { script: 'return arguments[0];', args: ['#agent-adapter'] });
  });

  await t.test('rejects a non-string script', () => {
    assert.throws(() => buildExecuteScriptRequest(''), TypeError);
    assert.throws(() => buildExecuteScriptRequest(undefined), TypeError);
  });
});

test('toWebDriverError', async (t) => {
  await t.test('maps a WebDriver protocol error body to error/message/stacktrace', () => {
    const error = toWebDriverError(404, {
      value: { error: 'no such element', message: 'not found', stacktrace: 'trace' },
    });
    assert.ok(error instanceof WebDriverProtocolError);
    assert.equal(error.status, 404);
    assert.equal(error.error, 'no such element');
    assert.equal(error.message, 'not found');
    assert.equal(error.stacktrace, 'trace');
  });

  await t.test('falls back to a generic message when the body carries none', () => {
    const error = toWebDriverError(500, {});
    assert.equal(error.error, 'unknown error');
    assert.equal(error.message, 'WebDriver request failed with HTTP 500');
  });
});
