/** The last code point of the C0 control range (inclusive). */
const C0_MAX = 0x1f
/** Tab's code point -- the sole C0 control character kept. */
const TAB = 0x09
/** DEL's code point. */
const DEL = 0x7f
/** The C1 control range (inclusive): U+0080-U+009F. */
const C1_MIN = 0x80
const C1_MAX = 0x9f
/** Zero-width space, non-joiner, and joiner: U+200B-U+200D. */
const ZERO_WIDTH_MIN = 0x200b
const ZERO_WIDTH_MAX = 0x200d
/** Left-to-right mark / right-to-left mark: U+200E, U+200F. */
const DIRECTIONAL_MARK_MIN = 0x200e
const DIRECTIONAL_MARK_MAX = 0x200f
/** Explicit bidi embeddings/overrides (LRE, RLE, PDF, LRO, RLO): U+202A-U+202E. */
const BIDI_EMBEDDING_MIN = 0x202a
const BIDI_EMBEDDING_MAX = 0x202e
/** Explicit bidi isolates (LRI, RLI, FSI, PDI): U+2066-U+2069. */
const BIDI_ISOLATE_MIN = 0x2066
const BIDI_ISOLATE_MAX = 0x2069

function isInRange(codePoint: number, min: number, max: number): boolean {
  return codePoint >= min && codePoint <= max
}

/**
 * Format controls stripped alongside C0/C1/DEL: invisible or
 * reordering-only code points that carry no visible textual content but
 * can spoof or hide surrounding text (a bidi override can make "evil.exe"
 * display as "exe.evil", a zero-width joiner can hide inside a lookalike
 * filename). RCS-001-R18 forbids removing *textual* content, but these
 * are format controls, not text -- the same distinction RCS-001 already
 * draws explicitly for OSC 0/2 titles and OSC 9 notifications ("control
 * and bidirectional-override characters stripped").
 *
 * Stripping the zero-width joiner/non-joiner (U+200C/U+200D) is a
 * deliberate trade-off, not a free lunch: some scripts (Arabic ligature
 * control, Indic conjuncts) rely on ZWJ/ZWNJ for correct shaping, and
 * stripping them can change how a word renders. This slice accepts that
 * cost for a uniform invisible-character defense across all untrusted
 * text; a future slice may need a narrower, script-aware allowance.
 */
function isFormatControlCodePoint(codePoint: number): boolean {
  return (
    isInRange(codePoint, ZERO_WIDTH_MIN, ZERO_WIDTH_MAX) ||
    isInRange(codePoint, DIRECTIONAL_MARK_MIN, DIRECTIONAL_MARK_MAX) ||
    isInRange(codePoint, BIDI_EMBEDDING_MIN, BIDI_EMBEDDING_MAX) ||
    isInRange(codePoint, BIDI_ISOLATE_MIN, BIDI_ISOLATE_MAX)
  )
}

function isStrippableControlCodePoint(codePoint: number): boolean {
  const isC0ControlExceptTab = codePoint <= C0_MAX && codePoint !== TAB
  const isDel = codePoint === DEL
  const isC1Control = isInRange(codePoint, C1_MIN, C1_MAX)
  return isC0ControlExceptTab || isDel || isC1Control || isFormatControlCodePoint(codePoint)
}

/**
 * Strips C0 control characters (U+0000-U+001F, excluding tab U+0009), DEL
 * (U+007F), C1 control characters (U+0080-U+009F), and the bidirectional
 * and other format controls documented on {@link isFormatControlCodePoint}
 * from `text`. Every other character passes through unchanged, including
 * any byte that is only "control-like" as part of a larger escape
 * sequence (e.g. an OSC 8 hyperlink's payload) -- this function strips
 * individual control code points, it never recognizes or interprets a
 * sequence.
 */
export function stripControlCharacters(text: string): string {
  return Array.from(text)
    .filter((char) => {
      const codePoint = char.codePointAt(0)
      return codePoint === undefined || !isStrippableControlCodePoint(codePoint)
    })
    .join('')
}
