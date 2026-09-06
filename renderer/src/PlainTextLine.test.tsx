import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { stripControlCharacters } from './controlCharacters'
import { PlainTextLine } from './PlainTextLine'

/** Every C0 control character (U+0000..U+001F) except tab (U+0009). */
function allC0ExceptTab(): string {
  let chars = ''
  for (let code = 0x00; code <= 0x1f; code += 1) {
    if (code === 0x09) continue
    chars += String.fromCharCode(code)
  }
  return chars
}

/** Every code point in `[start, end]` (inclusive), concatenated as one string. */
function allInRange(start: number, end: number): string {
  let chars = ''
  for (let code = start; code <= end; code += 1) {
    chars += String.fromCharCode(code)
  }
  return chars
}

describe('stripControlCharacters', () => {
  it('strips every C0 control character except tab', () => {
    const input = `a${allC0ExceptTab()}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('strips DEL (U+007F)', () => {
    const input = `a${String.fromCharCode(0x7f)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('keeps tab (U+0009)', () => {
    const input = `a${String.fromCharCode(0x09)}b`

    expect(stripControlCharacters(input)).toBe(input)
  })

  it('keeps non-ASCII characters unchanged', () => {
    const input = 'café 日本語 emoji: 🎉'

    expect(stripControlCharacters(input)).toBe(input)
  })

  it('strips every C1 control character (U+0080-U+009F)', () => {
    const input = `a${allInRange(0x80, 0x9f)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('strips the left-to-right and right-to-left marks (U+200E, U+200F)', () => {
    const input = `a${String.fromCharCode(0x200e)}${String.fromCharCode(0x200f)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('strips the explicit bidi embedding/override controls (U+202A-U+202E: LRE, RLE, PDF, LRO, RLO)', () => {
    const input = `a${allInRange(0x202a, 0x202e)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('strips the explicit bidi isolate controls (U+2066-U+2069: LRI, RLI, FSI, PDI)', () => {
    const input = `a${allInRange(0x2066, 0x2069)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })

  it('strips the zero-width space, non-joiner, and joiner (U+200B-U+200D), documented as a spoofing-resistance trade-off', () => {
    const input = `a${allInRange(0x200b, 0x200d)}b`

    expect(stripControlCharacters(input)).toBe('ab')
  })
})

describe('PlainTextLine', () => {
  it('renders text literally with no markup interpretation', () => {
    const { container } = render(<PlainTextLine text="<b>bold</b>" />)

    expect(container.querySelector('b')).toBeNull()
    expect(container.textContent).toBe('<b>bold</b>')
  })

  it('strips control characters from the rendered text while keeping tab', () => {
    const input = `a${String.fromCharCode(0x00)}b${String.fromCharCode(0x09)}c`

    const { container } = render(<PlainTextLine text={input} />)

    expect(container.textContent).toBe(`ab${String.fromCharCode(0x09)}c`)
  })
})
