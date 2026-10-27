import {
  isShellError,
  type DefinitionAppliedFile,
  type DefinitionFingerprint,
  type DefinitionOperation,
} from './ipc/harness'
import type { DefinitionSectionError } from './DefinitionErrorLine'

/** Turns any rejection into the error the definition surfaces render. */
export function toSectionError(reason: unknown): DefinitionSectionError {
  return isShellError(reason) ? reason : 'unexpected'
}

/** How many hex characters of a digest a line shows; the plan id is always shown whole. */
const SHORT_HEX_LENGTH = 12

function shortHex(hex: string): string {
  return hex.slice(0, SHORT_HEX_LENGTH)
}

function fingerprintText(fingerprint: DefinitionFingerprint): string {
  switch (fingerprint.kind) {
    case 'sha256':
      return `sha256 ${shortHex(fingerprint.hex)}`
    case 'symlink':
      return `symlink ${fingerprint.target}`
    case 'irregular':
    case 'absent':
      return fingerprint.kind
  }
}

export function operationText(operation: DefinitionOperation): string {
  const before = fingerprintText(operation.before)
  const after = shortHex(operation.after)
  switch (operation.op) {
    case 'write-agents':
      return `${operation.op} ${operation.path} from ${operation.source}; before: ${before}; after: ${after}`
    case 'write-generated':
      return `${operation.op} ${operation.path} (${operation.templateVersion}); before: ${before}; after: ${after}`
    case 'write-wrapper':
      return `${operation.op} ${operation.path}; before: ${before}; after: ${after}`
    case 'write-block':
      return `${operation.op} ${operation.path} (${operation.mode}); before: ${before}; after: ${after}`
  }
}

export function appliedFileText(file: DefinitionAppliedFile): string {
  const after = file.after === null ? 'none' : shortHex(file.after)
  const snapshot = file.snapshot === null ? 'no snapshot' : `snapshot ${file.snapshot}`
  return `${file.path}: ${fingerprintText(file.before)} -> ${after} (${snapshot})`
}

/** The hint a stale plan gets in place of the shell's own wording. */
export const PLAN_STALE_HINT = 'the plan no longer matches the definition; preview again'
