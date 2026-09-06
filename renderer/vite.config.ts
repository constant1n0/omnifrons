/// <reference types="vitest/config" />
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/testSupport/setup.ts'],
    // Explicit rather than relying on Vitest's own default of the same
    // value: a stray `.only` left in a commit must never silently skip
    // the rest of the suite in CI (R3-014).
    allowOnly: !process.env.CI,
  },
})
