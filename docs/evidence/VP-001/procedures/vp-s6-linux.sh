#!/usr/bin/env bash
#
# VP-S6 procedure driver (design.md § D5/D7, § Honesty machinery).
#
# Two modes:
#   --mode=feasibility-check   AV1/AV2 gate, then one session create/close
#                              (F1). No scenario logic (Slice 3a).
#   --mode=scenario            AV1/AV2 gate, then the full VP-S6 scenario --
#                              approve the fixture, pick a workspace, select
#                              adapter/approval, Start, observe, Stop, a
#                              bounded wait, re-enumerate, and a key=value
#                              observation transcript on stdout (Slice 3b,
#                              delegated to vp-s6-scenario.mjs). This mode
#                              never names an outcome -- see that script's
#                              own header.
#
# Usage:
#   vp-s6-linux.sh --mode=feasibility-check <artifact-path> <build-channel-digest>
#   vp-s6-linux.sh --mode=scenario <artifact-path> <build-channel-digest> <fixture-path> <workspace-dir>
#
# Environment:
#   VP001_WEBDRIVER_BASE_URL   tauri-driver's own HTTP endpoint
#                              (default: http://127.0.0.1:4444)
#
# Exit codes: 0 once the AV1/AV2 gate passes and the mode's own steps run to
# completion (a `--mode=scenario` run that observes `uncertain` or `fail`
# still exits 0 -- only the AV1/AV2 gate and usage errors are fatal here,
# per design.md's Honesty machinery: this script never turns an honest
# observation into a nonzero exit); 1 on an AV1/AV2 gate rejection
# (`gate=<rejection-token>` is always printed to stdout first); 2 on a usage
# error.

set -euo pipefail

mode=""
if [[ "${1:-}" == --mode=* ]]; then
  mode="${1#--mode=}"
  shift
fi

usage() {
  echo "usage: vp-s6-linux.sh --mode=feasibility-check <artifact-path> <build-channel-digest>" >&2
  echo "       vp-s6-linux.sh --mode=scenario <artifact-path> <build-channel-digest> <fixture-path> <workspace-dir>" >&2
}

if [[ "${mode}" != "feasibility-check" && "${mode}" != "scenario" ]]; then
  echo "vp-s6-linux.sh: unknown or missing --mode (must be feasibility-check or scenario)" >&2
  usage
  exit 2
fi

webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# AV1 (design.md § Honesty machinery): re-sha256sum the exact file about to
# be handed to tauri-driver and compare it to the value the CI `Digest
# artifact` step recorded. A mismatch aborts before any session exists and
# files no row (`artifact-digest-mismatch`).
run_av1() {
  local artifact_path="$1"
  local build_channel_digest="$2"
  local observed_artifact_digest
  observed_artifact_digest="$(sha256sum "${artifact_path}" | cut -d' ' -f1)"
  if [[ "${observed_artifact_digest}" != "${build_channel_digest}" ]]; then
    echo "gate=artifact-digest-mismatch expected=${build_channel_digest} observed=${observed_artifact_digest}"
    exit 1
  fi
  echo "gate=av1-digest-match digest=${observed_artifact_digest}"
}

# AV2: extract the digest-verified AppImage's payload with
# `--appimage-extract` (works without FUSE) and byte-scan it for the literal
# `--demo-harness` string -- the same technique
# `src-tauri/tests/feature_gate.rs` uses against a raw binary. The AppImage
# container is compressed, so scanning it directly would be a false
# negative.
run_av2() {
  local artifact_path="$1"
  local extract_dir
  extract_dir="$(mktemp -d)"
  # `extract_dir` is `local` to this function: an `exit 1` below still fires
  # this trap while that binding is in scope (a mid-function `exit`, not a
  # `return`, so cleanup still runs on the rejection paths). On the success
  # path, the trap is cleared explicitly before returning -- otherwise it
  # would dangle past this function's own scope and fire again at the
  # script's real exit, when `extract_dir` is no longer bound anywhere
  # (`set -u` then rejects it).
  trap 'rm -rf "${extract_dir}"' EXIT

  (
    cd "${extract_dir}"
    "${artifact_path}" --appimage-extract >/dev/null
  )

  local payload_binary
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
  rm -rf "${extract_dir}"
  trap - EXIT
}

if [[ "${mode}" == "feasibility-check" ]]; then
  if [[ $# -ne 2 ]]; then
    usage
    exit 2
  fi
  artifact_path="$1"
  build_channel_digest="$2"
  if [[ ! -f "${artifact_path}" ]]; then
    echo "vp-s6-linux.sh: artifact not found: ${artifact_path}" >&2
    exit 2
  fi

  run_av1 "${artifact_path}" "${build_channel_digest}"
  run_av2 "${artifact_path}"

  # F1: tauri-driver creates a session against the gated file (the
  # original, digest-verified artifact -- never the extracted payload;
  # design.md's Open Questions covers the FUSE/APPIMAGE_EXTRACT_AND_RUN
  # branch, both of which launch this exact file).
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
  exit 0
fi

# --mode=scenario
if [[ $# -ne 4 ]]; then
  usage
  exit 2
fi
artifact_path="$1"
build_channel_digest="$2"
fixture_path="$3"
workspace_dir="$4"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s6-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi
if [[ ! -x "${fixture_path}" ]]; then
  echo "vp-s6-linux.sh: fixture not found or not executable: ${fixture_path}" >&2
  exit 2
fi
if [[ ! -d "${workspace_dir}" ]]; then
  echo "vp-s6-linux.sh: workspace directory not found: ${workspace_dir}" >&2
  exit 2
fi

run_av1 "${artifact_path}" "${build_channel_digest}"
run_av2 "${artifact_path}"

# Delegated to vp-s6-scenario.mjs (design.md's own sequence: AV1/AV2 ->
# create session -> dialog driving -> Start -> observe -> Stop -> bounded
# wait -> re-enumerate -> transcript). That script owns its own session and
# emits observations only; this script's own exit code stays 0 for any
# scenario outcome (design.md Honesty machinery: an honest `uncertain` is
# not a script failure).
VP001_PROCEDURES_DIR="${procedures_dir}" \
  VP001_WEBDRIVER_BASE_URL="${webdriver_base_url}" \
  VP001_APPLICATION_PATH="${artifact_path}" \
  VP001_FIXTURE_PATH="${fixture_path}" \
  VP001_WORKSPACE_DIR="${workspace_dir}" \
  node "${procedures_dir}/vp-s6-scenario.mjs"
