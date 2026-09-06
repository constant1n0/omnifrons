/**
 * Vitest global setup (`vite.config.ts`'s `test.setupFiles`), run once per
 * test file before any of its tests. Centralizes two concerns every
 * renderer test file previously duplicated in its own `beforeAll`:
 *
 * - The jsdom WebCrypto polyfill `@tauri-apps/api/mocks`' `mockIPC`
 *   needs to mint `Channel` callback ids (see `installJsdomCrypto.ts`'s
 *   doc comment).
 * - React Testing Library's `cleanup()` after every test, unmounting any
 *   component `render()`ed by it. `@testing-library/react` already runs
 *   this automatically when it detects a global `afterEach` (which
 *   Vitest always provides, regardless of the `test.globals` config
 *   option), so this `afterEach(cleanup)` is a second, harmless
 *   (idempotent) call -- made explicit here so the test suite's teardown
 *   doesn't depend on that auto-detection behavior being understood or
 *   remembered (R3-010).
 */
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

import { installJsdomCryptoPolyfill } from './installJsdomCrypto'

installJsdomCryptoPolyfill()

afterEach(() => {
  cleanup()
})
