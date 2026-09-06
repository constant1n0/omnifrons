/**
 * jsdom has no WebCrypto implementation, but `@tauri-apps/api/mocks`'
 * channel registry calls `window.crypto.getRandomValues` to mint callback
 * ids for every `Channel` a test constructs. This installs a minimal,
 * non-cryptographic fill -- adequate for generating unique-enough test
 * callback ids, never appropriate for anything security sensitive.
 *
 * Implemented with no Node built-ins so it type-checks under
 * `tsconfig.app.json` (which does not include Node's ambient types).
 */
export function installJsdomCryptoPolyfill(): void {
  Object.defineProperty(window, 'crypto', {
    value: {
      getRandomValues: <T extends ArrayBufferView>(buffer: T): T => {
        const view = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength)
        for (let index = 0; index < view.length; index += 1) {
          view[index] = Math.floor(Math.random() * 256)
        }
        return buffer
      },
    },
    configurable: true,
  })
}
