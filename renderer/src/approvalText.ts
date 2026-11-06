import type { Approval } from './ipc/harness'

/**
 * What the approval lists say about an active approval whose executable is
 * gone or changed (#184), or `null` when there is nothing to say: the file
 * was present at the recorded size when the shell listed it, or the
 * approval is revoked. Fixed copy, shared by the Executable approval region
 * and the Agent panel's select so both name the same fact the same way.
 */
export function executablePresenceNote(approval: Approval): string | null {
  if (approval.status !== 'active') return null
  switch (approval.presence) {
    case 'missing':
      return 'executable missing'
    case 'changed':
      return 'executable changed since approval'
    default:
      return null
  }
}

/**
 * Whether the Agent panel may offer `approval` for a run: active, and with
 * nothing seen against its executable at listing time (#184). The launch
 * gate's re-probe still decides every launch; this only keeps the operator
 * from picking an approval the gate is certain to deny.
 */
export function isLaunchableApproval(approval: Approval): boolean {
  return approval.status === 'active' && approval.presence === 'present'
}
