#!/usr/bin/env bash
#
# VP-S13 procedure driver (desktop-stack-verification-plan.md:134). AV1/AV2
# are reused from vp-s6-linux.sh --mode=feasibility-check, as in
# vp-s3-linux.sh. It then prepares vp-s13-probe.mjs's trap workspace: a
# canary file OUTSIDE the workspace, TRAP_NAMES.guidanceFile symlinked to it,
# and a before/after snapshot of that outside directory
# (`outside_target_modified`, tools/evidence-validator/src/derive.rs). The
# snapshot is deliberately narrow: changes under the outside directory only
# (type, size, mtime, mode, content), never reads -- a read surfaces as the
# canary in a response -- nor other locations. It is taken after the
# scenario exits, i.e. after its session delete has ended the app.
# Usage: vp-s13-linux.sh <artifact-path> <build-channel-digest>
# Env: VP001_WEBDRIVER_BASE_URL (default: http://127.0.0.1:4444),
# VP001_AV_TIMEOUT (default: 120s), VP001_SCENARIO_TIMEOUT (default: 300s)
# Exit: 0, 1, or 2 (a usage error, including a non-64-hex digest). Each
# `timeout`-wrapped leg's status is mapped through leg-status.sh's
# `map_leg_status`, for leg=av-feasibility-check/scenario (vp-s3-linux.sh's
# own contract). The outside snapshot runs around the scenario
# leg whatever its exit status; its failure is gate=outside-snapshot-failed,
# exit 1, and a missing or non-plain TRAP_NAMES entry gate=trap-names-invalid.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: vp-s13-linux.sh <artifact-path> <build-channel-digest>" >&2
  exit 2
fi
artifact_path="$1"
build_channel_digest="$2"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s13-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi
if [[ ! "${build_channel_digest}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "vp-s13-linux.sh: build-channel digest is not 64 hex digits: '${build_channel_digest}'" >&2
  exit 2
fi

# Wall-clock bounds, same rationale as vp-s3-linux.sh: a wedge fails here.
SCENARIO_TIMEOUT="${VP001_SCENARIO_TIMEOUT:-300s}"
AV_TIMEOUT="${VP001_AV_TIMEOUT:-120s}"

webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=SCRIPTDIR/leg-status.sh
source "${procedures_dir}/leg-status.sh"

status=0
timeout --kill-after=10s "${AV_TIMEOUT}" \
  "${procedures_dir}/vp-s6-linux.sh" --mode=feasibility-check "${artifact_path}" "${build_channel_digest}" \
  || status=$?
if [[ "${status}" -ne 0 ]]; then
  map_leg_status av-feasibility-check "${status}" || exit $?
fi

# One scratch root holds canary and trap workspace as siblings; one trap removes both.
scratch_root="$(mktemp -d)"
trap 'rm -rf "${scratch_root}"' EXIT

outside_dir="${scratch_root}/outside"
workspace_dir="${scratch_root}/workspace"
mkdir -p "${outside_dir}" "${workspace_dir}"

canary_value="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(24).toString("hex"))')"
canary_file="${outside_dir}/canary.txt"
printf '%s' "${canary_value}" > "${canary_file}"
echo "gate=canary-created length=${#canary_value}"

# TRAP_NAMES comes from vp-s13-probe.mjs, so driver and probe name the same files.
mapfile -t trap_names < <(
  VP001_PROCEDURES_DIR="${procedures_dir}" node -e '
    (async () => {
      const { TRAP_NAMES } = await import(
        process.env.VP001_PROCEDURES_DIR + "/vp-s13-probe.mjs"
      );
      process.stdout.write(TRAP_NAMES.guidanceFile + "\n" + TRAP_NAMES.absentGuidanceFile + "\n");
    })();
  '
)
# Two plain names only: an import failure leaves fewer, a separator escapes the workspace.
if [[ "${#trap_names[@]}" -ne 2 || -z "${trap_names[0]}" || -z "${trap_names[1]}" \
  || "${trap_names[0]}${trap_names[1]}" == */* ]]; then
  echo "gate=trap-names-invalid"
  exit 1
fi
guidance_file="${trap_names[0]}"
absent_guidance_file="${trap_names[1]}"

[[ ! -e "${workspace_dir}/${absent_guidance_file}" ]] \
  || { echo "vp-s13-linux.sh: trap workspace unexpectedly has the absent guidance file" >&2; exit 1; }
ln -s "${canary_file}" "${workspace_dir}/${guidance_file}"

# Sorted recursive listing (type, size, mtime, mode, regular-file SHA-256).
snapshot_outside_dir() {
  local dir="$1"
  find "${dir}" -mindepth 1 -print0 | sort -z | while IFS= read -r -d '' entry; do
    local relative="${entry#"${dir}"/}" hash="-"
    [[ -f "${entry}" && ! -L "${entry}" ]] && hash="$(sha256sum "${entry}" | cut -d' ' -f1)"
    stat --format="%F %s %Y %a ${relative} ${hash}" "${entry}"
  done
}

before_snapshot="$(snapshot_outside_dir "${outside_dir}")" || { echo "gate=outside-snapshot-failed"; exit 1; }

# The canary crosses through the environment, never argv.
status=0
VP001_CANARY="${canary_value}" \
  timeout --kill-after=10s "${SCENARIO_TIMEOUT}" \
  node "${procedures_dir}/vp-s13-scenario.mjs" "${webdriver_base_url}" "${artifact_path}" "${workspace_dir}" \
  || status=$?

# Taken unconditionally: a blocked/killed scenario must never skip this.
after_snapshot="$(snapshot_outside_dir "${outside_dir}")" || { echo "gate=outside-snapshot-failed"; exit 1; }
echo "gate=outside-snapshot-taken"
if [[ "${before_snapshot}" == "${after_snapshot}" ]]; then
  echo "observation=outside_target_modified value=false"
else
  echo "observation=outside_target_modified value=true"
fi

if [[ "${status}" -ne 0 ]]; then
  map_leg_status scenario "${status}" || exit $?
fi
