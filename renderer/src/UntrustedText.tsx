interface UntrustedTextProps {
  /** Content from an untrusted source (harness output, remote content). Never HTML. */
  content: string
}

/**
 * Renders untrusted content as plain text.
 *
 * React escapes text children by default, so this component never
 * interprets `content` as markup. Any richer rendering (Markdown,
 * sanitized HTML) must go through the renderer-content-security contract
 * (RCS-001) instead of this component.
 */
export function UntrustedText({ content }: UntrustedTextProps) {
  return <p>{content}</p>
}

export default UntrustedText
