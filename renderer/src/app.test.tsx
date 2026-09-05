import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import App, { DEFAULT_UNTRUSTED_CONTENT } from './App'

describe('App', () => {
  it('renders untrusted content as plain text by default', () => {
    const untrustedContent = 'Reported note: <b>bold</b> should stay literal.'

    const { container } = render(<App untrustedContent={untrustedContent} />)

    expect(container.textContent).toContain(untrustedContent)
    expect(container.querySelector('b')).toBeNull()
  })

  it('shows the default placeholder when no content was received', () => {
    const { container } = render(<App />)

    expect(container.textContent).toContain(DEFAULT_UNTRUSTED_CONTENT)
  })

  it('never interprets untrusted content as markup', () => {
    const payloads = [
      '<script>alert(1)</script>',
      '"><img src=x onerror=alert(1)>',
      '<a href="javascript:alert(1)">x</a>',
    ]

    for (const payload of payloads) {
      const { container, unmount } = render(<App untrustedContent={payload} />)

      expect(container.querySelector('script')).toBeNull()
      expect(container.querySelector('img')).toBeNull()
      expect(container.querySelector('a')).toBeNull()
      expect(container.textContent).toContain(payload)

      unmount()
    }
  })

  it('renders an explicit empty string as empty content, not the default placeholder', () => {
    const { container } = render(<App untrustedContent="" />)

    expect(container.textContent).not.toContain(DEFAULT_UNTRUSTED_CONTENT)
    expect(container.querySelector('p')?.textContent).toBe('')
  })
})
