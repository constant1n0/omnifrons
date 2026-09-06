import { stripControlCharacters } from './controlCharacters'

interface PlainTextLineProps {
  /** One line of untrusted text (harness stdout/stderr output). */
  text: string
}

/**
 * Renders one line of untrusted text as plain text.
 *
 * Strips C0 control characters and DEL before rendering, then hands the
 * result to React as a plain text child -- no markup, no links, no OSC
 * interpretation. This is RCS-001-R1's default ("Every content class not
 * explicitly upgraded to a richer mode MUST render as plain text by
 * default: control characters stripped, links inert until an explicit
 * open gesture") applied at the single-line grain the harness output log
 * renders at.
 *
 * Per RCS-001-R18 ("Sanitization MUST NOT remove textual content"), this
 * component only ever removes control code points -- every other
 * character, including the punctuation of a stripped escape sequence's
 * surviving payload, is preserved and rendered as literal text.
 */
export function PlainTextLine({ text }: PlainTextLineProps) {
  return <span>{stripControlCharacters(text)}</span>
}

export default PlainTextLine
