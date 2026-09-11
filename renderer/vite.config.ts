/// <reference types="vitest/config" />
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  server: {
    fs: {
      // `src/ipc/harness.test.ts` reads the shell's own `src-tauri/src/ipc/
      // dto.rs` through Vite's `?raw`, and that file sits outside this
      // package. Vite serves a file outside the project root only when
      // `server.fs.allow` lists it, and it is listed today only because
      // Vite *infers* the pnpm workspace root from the manifests above
      // this directory. That inference is not a contract: with the root
      // manifest absent the import is denied -- `Denied ID
      // …/src-tauri/src/ipc/dto.rs?raw` -- and the whole file loads with
      // **no tests at all**, taking the cross-language DTO guard and
      // every other test in that file offline in one line (R3-105,
      // measured).
      // Explicit rather than inferred, the same reasoning as `allowOnly`
      // below.
      allow: ['..'],
    },
  },
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/testSupport/setup.ts'],
    // Explicit rather than relying on Vitest's own default of the same
    // value: a stray `.only` left in a commit must never silently skip
    // the rest of the suite in CI (R3-014).
    allowOnly: !process.env.CI,
    // Every `vi.spyOn(console, 'error')` in this suite is restored on the
    // last line of its own test body, which is exactly the line a failing
    // assertion above it never reaches. The silenced `console.error` then
    // outlived the test and took the console-error-as-failure signal with
    // it for the rest of the file -- one red test quietly disarming the
    // checks in every test after it (R3-046). Restoring after each test is
    // the harness's job, not each test body's.
    restoreMocks: true,
  },
})
