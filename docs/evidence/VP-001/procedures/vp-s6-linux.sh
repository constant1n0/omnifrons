#!/usr/bin/env bash
#
# VP-S6 procedure driver (design.md § D5/D7, § Honesty machinery).
#
# `--mode=feasibility-check` is the only mode this slice implements: it runs
# the AV1/AV2 artifact-identity gate against the retained, digest-addressable
# artifact, then creates one `tauri-driver` session against it (F1). It
# performs no VP-S6 scenario logic yet -- no dialog driving, no Start/Stop, no
# `/proc` enumeration, no transcript. Slice 3b extends this script with the
# full scenario mode; the feasibility gate's own F2/F3 facts (both GTK
# choosers completing, an approved executable reaching an active state) are
# recorded as a disclosed, non-repository-test procedure per design.md's
# Testing Strategy ("the live browser-driving path only ... is evidence
# machinery, not a repository test, and never gates CI").
#
# Usage:
#   vp-s6-linux.sh --mode=feasibility-check <artifact-path> <build-channel-digest>
#
# Environment:
#   VP001_WEBDRIVER_BASE_URL   tauri-driver's own HTTP endpoint
#                              (default: http://127.0.0.1:4444)
#
# Exit codes: 0 on every gate passing (including F1's session creation); 1 on
# any gate rejection (`gate=<rejection-token>` is always printed to stdout
# before exiting); 2 on a usage error.

set -euo pipefail

mode=""
if [[ "${1:-}" == --mode=* ]]; then
  mode="${1#--mode=}"
  shift
fi

usage() {
  echo "usage: vp-s6-linux.sh --mode=feasibility-check <artifact-path> <build-channel-digest>" >&2
}

if [[ "${mode}" != "feasibility-check" ]]; then
  echo "vp-s6-linux.sh: only --mode=feasibility-check exists in this slice (Slice 3b adds the full scenario mode)" >&2
  usage
  exit 2
fi

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

artifact_path="$1"
build_channel_digest="$2"
webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s6-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi

# AV1 (design.md § Honesty machinery): re-sha256sum the exact file about to
# be handed to tauri-driver and compare it to the value the CI `Digest
# artifact` step recorded. A mismatch aborts before any session exists and
# files no row (`artifact-digest-mismatch`).
observed_artifact_digest="$(sha256sum "${artifact_path}" | cut -d' ' -f1)"
if [[ "${observed_artifact_digest}" != "${build_channel_digest}" ]]; then
  echo "gate=artifact-digest-mismatch expected=${build_channel_digest} observed=${observed_artifact_digest}"
  exit 1
fi
echo "gate=av1-digest-match digest=${observed_artifact_digest}"

# AV2: extract the digest-verified AppImage's payload with
# `--appimage-extract` (works without FUSE) and byte-scan it for the literal
# `--demo-harness` string -- the same technique
# `src-tauri/tests/feature_gate.rs` uses against a raw binary. The AppImage
# container is compressed, so scanning it directly would be a false
# negative.
extract_dir="$(mktemp -d)"
cleanup_extract_dir() {
  rm -rf "${extract_dir}"
}
trap cleanup_extract_dir EXIT

(
  cd "${extract_dir}"
  "${artifact_path}" --appimage-extract >/dev/null
)

payload_binary="$(find "${extract_dir}/squashfs-root/usr/bin" -maxdepth 1 -type f -executable 2>/dev/null | head -n1)"
if [[ -z "${payload_binary}" ]]; then
  echo "gate=feature-enabled-variant reason=payload-binary-not-found"
  exit 1
fi
if grep -q -a -- '--demo-harness' "${payload_binary}"; then
  echo "gate=feature-enabled-variant reason=literal-string-found"
  exit 1
fi
echo "gate=av2-variant-scan-absent binary=${payload_binary}"

# F1: tauri-driver creates a session against the gated file (the original,
# digest-verified artifact -- never the extracted payload; design.md's Open
# Questions covers the FUSE/APPIMAGE_EXTRACT_AND_RUN branch, both of which
# launch this exact file).
session_id="$(
  VP001_PROCEDURES_DIR="${procedures_dir}" \
  VP001_WEBDRIVER_BASE_URL="${webdriver_base_url}" \
  VP001_APPLICATION_PATH="${artifact_path}" \
  node -e '
    (async () => {
      const { createSession } = await import(
        process.env.VP001_PROCEDURES_DIR + "/webdriver-session.mjs"
      );
      const sessionId = await createSession(
        process.env.VP001_WEBDRIVER_BASE_URL,
        process.env.VP001_APPLICATION_PATH,
      );
      process.stdout.write(sessionId);
    })();
  '
)"
echo "gate=f1-session-created session_id=${session_id}"

VP001_PROCEDURES_DIR="${procedures_dir}" \
  VP001_WEBDRIVER_BASE_URL="${webdriver_base_url}" \
  VP001_SESSION_ID="${session_id}" \
  node -e '
    (async () => {
      const { deleteSession } = await import(
        process.env.VP001_PROCEDURES_DIR + "/webdriver-session.mjs"
      );
      await deleteSession(process.env.VP001_WEBDRIVER_BASE_URL, process.env.VP001_SESSION_ID);
    })();
  '
echo "gate=session-closed session_id=${session_id}"
