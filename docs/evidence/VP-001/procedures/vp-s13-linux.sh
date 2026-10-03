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
# scenario exits, i.e. after its session delete has ended the app -- except
# on a timeout or a kill (raw status 124 or 137 -- leg-status.sh's own
# `<leg>-timeout`/`<leg>-killed` statuses): a SIGKILL there may race the
# app's own teardown, so the session delete is not proven to have ended it,
# and an unchanged snapshot is then reported as
# gate=outside-snapshot-indeterminate with NO observation line at all --
# that absence, not a false value, is what tells derive_ipc_boundary
# (tools/evidence-validator) outside_snapshot_taken is false, i.e. "not
# observed" rather than "observed as unmodified". A snapshot that still
# differs is reported as a demonstrated change regardless of exit status
# (gate=outside-snapshot-taken, value=true): the difference itself stands. A
# vanished outside directory is itself a modification
# (gate=outside-snapshot-taken, value=true); any other after-snapshot
# failure stays gate=outside-snapshot-failed, exit 1. A workspace GTK
# bookmark is seeded and restored the same way as vp-s6-linux.sh's own
# (--mode=scenario), so the chooser's bookmark-jump strategy can reach it; a
# symlinked bookmarks path, or one this script fails to back up, is left
# untouched (gate=bookmark-seed-skipped), the chooser then falling back to
# typing the path.
# Usage: vp-s13-linux.sh <artifact-path> <build-channel-digest>
# Env: VP001_WEBDRIVER_BASE_URL (default: http://127.0.0.1:4444),
# VP001_AV_TIMEOUT (default: 120s), VP001_SCENARIO_TIMEOUT (default: 300s)
# Exit: 0, 1, or 2 (a usage error, including a non-64-hex digest). Each
# `timeout`-wrapped leg's status is mapped through leg-status.sh's
# `map_leg_status`, for leg=av-feasibility-check/scenario (vp-s3-linux.sh's
# own contract). A missing, non-plain, or dot-segment (`.`/`..`) TRAP_NAMES
# entry is gate=trap-names-invalid, exit 1, before any trap or scenario.

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

# One scratch root holds canary and trap workspace as siblings. The same
# EXIT trap also restores (or removes) the GTK3 places-sidebar bookmark
# seeded below, the way vp-s6-linux.sh --mode=scenario restores its own: a
# caller's own bookmarks are their data and must be left exactly as found.
scratch_root="$(mktemp -d)"
bookmarks_file="${HOME}/.config/gtk-3.0/bookmarks"
bookmarks_backup=""
bookmarks_seeded="0"
cleanup() {
  rm -rf "${scratch_root}"
  if [[ -n "${bookmarks_backup}" ]]; then
    mv -f "${bookmarks_backup}" "${bookmarks_file}"
  elif [[ "${bookmarks_seeded}" == "1" ]]; then
    rm -f "${bookmarks_file}"
  fi
}
trap cleanup EXIT

outside_dir="${scratch_root}/outside"
workspace_dir="${scratch_root}/workspace"
mkdir -p "${outside_dir}" "${workspace_dir}"

# Seeded for the chooser's bookmark-jump strategy (vp-s6-xdotool.mjs, tried
# first); typing the path is still the fallback whenever seeding is
# skipped below. A symlinked bookmarks path is left untouched: seeding
# through it would write through to wherever it points, not to a file
# this script owns. `bookmarks_backup` is assigned only once `cp -p` has
# actually succeeded -- never from the bare `mktemp` result -- so a failed
# backup can never leave the EXIT trap restoring an empty or partial file
# over the caller's real one; seeding itself is skipped in that case too.
mkdir -p "$(dirname "${bookmarks_file}")"
if [[ -L "${bookmarks_file}" ]]; then
  echo "gate=bookmark-seed-skipped reason=symlink"
elif [[ -f "${bookmarks_file}" ]]; then
  bookmarks_backup_candidate="$(mktemp "${bookmarks_file}.vp-s13-backup.XXXXXX")"
  if cp -p "${bookmarks_file}" "${bookmarks_backup_candidate}"; then
    bookmarks_backup="${bookmarks_backup_candidate}"
    printf 'file://%s\n' "${workspace_dir}" > "${bookmarks_file}"
    bookmarks_seeded="1"
  else
    rm -f "${bookmarks_backup_candidate}"
    echo "gate=bookmark-seed-skipped reason=backup-failed"
  fi
else
  printf 'file://%s\n' "${workspace_dir}" > "${bookmarks_file}"
  bookmarks_seeded="1"
fi

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
# Two plain names only: an import failure leaves fewer, a separator escapes
# the workspace, and a bare "." or ".." names the directory itself rather
# than a file inside it.
if [[ "${#trap_names[@]}" -ne 2 || -z "${trap_names[0]}" || -z "${trap_names[1]}" \
  || "${trap_names[0]}${trap_names[1]}" == */* \
  || "${trap_names[0]}" == "." || "${trap_names[0]}" == ".." \
  || "${trap_names[1]}" == "." || "${trap_names[1]}" == ".." ]]; then
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
# status 124/137 are leg-status.sh's own timeout/kill statuses: an
# unchanged snapshot under either does not prove nothing happened, since
# the session delete may not have ended the app, so it is indeterminate --
# and reported with NO observation line, never a false one, so
# derive_ipc_boundary can never read this as a proven negative. A detected
# difference always stands as a demonstrated change. A vanished outside
# directory is itself a modification, not a snapshot failure.
if after_snapshot="$(snapshot_outside_dir "${outside_dir}")"; then
  if [[ "${before_snapshot}" != "${after_snapshot}" ]]; then
    echo "gate=outside-snapshot-taken"
    echo "observation=outside_target_modified value=true"
  elif [[ "${status}" -eq 124 || "${status}" -eq 137 ]]; then
    echo "gate=outside-snapshot-indeterminate"
  else
    echo "gate=outside-snapshot-taken"
    echo "observation=outside_target_modified value=false"
  fi
elif [[ ! -d "${outside_dir}" ]]; then
  echo "gate=outside-snapshot-taken"
  echo "observation=outside_target_modified value=true"
else
  echo "gate=outside-snapshot-failed"
  exit 1
fi

if [[ "${status}" -ne 0 ]]; then
  map_leg_status scenario "${status}" || exit $?
fi
