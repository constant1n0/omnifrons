#!/usr/bin/env bash
#
# VP-S14 procedure driver (desktop-stack-verification-plan.md:135). AV1/AV2
# are reused from vp-s6-linux.sh --mode=feasibility-check, exactly as
# vp-s13-linux.sh and vp-s3-linux.sh do. It then prepares
# vp-s14-scenario.mjs's three fixture files -- copies of the real
# `true`/`false` binaries, named by FIXTURE_NAMES (vp-s14-scenario.mjs
# itself; read here via node the same way vp-s13-linux.sh reads
# TRAP_NAMES from vp-s13-probe.mjs) -- before running the scenario, which
# mutates `approvedExe` in place between launches (case (b): a
# bytes-only rewrite; case (c): a symlink swap to `shadowTargetExe`). The
# scratch directory is removed unconditionally by an EXIT trap: `rm -rf`
# never dereferences a symlink into its target when deleting a tree, so
# the dangling `approvedExe` symlink case (c) leaves behind is itself
# unlinked there, never the file (inside or outside the scratch dir) it
# points to.
# Usage: vp-s14-linux.sh <artifact-path> <build-channel-digest>
# Env: VP001_WEBDRIVER_BASE_URL (default: http://127.0.0.1:4444),
# VP001_AV_TIMEOUT (default: 120s), VP001_SCENARIO_TIMEOUT (default: 300s),
# VP001_FIXTURE_NAMES_TIMEOUT (default: 10s),
# VP001_FIXTURE_BEHAVIOUR_TIMEOUT (default: 5s)
# Exit: 0, 1, or 2 (a usage error, including a non-64-hex digest). Each
# `timeout`-wrapped leg's status is mapped through leg-status.sh's
# `map_leg_status`, for leg=av-feasibility-check/scenario (vp-s3-linux.sh's
# own contract). A missing `true`/`false` binary is
# gate=fixture-source-missing, exit 1, before any fixture is created. The
# FIXTURE_NAMES read (a `node -e` dynamic import, bounded by
# VP001_FIXTURE_NAMES_TIMEOUT) outliving or being killed at that bound is
# gate=fixture-names-timeout, exit 1 -- distinct from a missing,
# non-plain, dot-segment (`.`/`..`), or non-unique entry, which is
# gate=fixture-names-invalid, exit 1, before any trap or scenario. A
# multi-call `true`/`false` (busybox, uutils) whose two applets are
# byte-identical is gate=fixture-sources-indistinct, exit 1, before any
# fixture is copied; one that copies fine but does not behave as its role
# requires when run directly is gate=fixture-behaviour-mismatch
# name=<basename> status=<n>, exit 1 -- both before the scenario runs.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: vp-s14-linux.sh <artifact-path> <build-channel-digest>" >&2
  exit 2
fi
artifact_path="$1"
build_channel_digest="$2"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s14-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi
if [[ ! "${build_channel_digest}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "vp-s14-linux.sh: build-channel digest is not 64 hex digits: '${build_channel_digest}'" >&2
  exit 2
fi

# Wall-clock bounds, same rationale as vp-s3-linux.sh/vp-s13-linux.sh: a wedge fails here.
SCENARIO_TIMEOUT="${VP001_SCENARIO_TIMEOUT:-300s}"
AV_TIMEOUT="${VP001_AV_TIMEOUT:-120s}"
# The FIXTURE_NAMES read below is a plain dynamic import with no dialog,
# network, or process to wait on -- short, but still a real wall-clock
# bound rather than none at all, since a wedged import (e.g. a module
# whose top level never finishes evaluating) would otherwise hang this
# script forever before the AV/scenario legs' own bounds ever apply.
FIXTURE_NAMES_TIMEOUT="${VP001_FIXTURE_NAMES_TIMEOUT:-10s}"

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

# FIXTURE_NAMES lives on vp-s14-scenario.mjs itself (there is no separate
# probe-constants module for this VP), so driver and scenario always agree
# on these three names -- read the same way vp-s13-linux.sh reads
# TRAP_NAMES from vp-s13-probe.mjs. Bounded by FIXTURE_NAMES_TIMEOUT: a
# process-substitution `mapfile < <(...)` can't itself surface the inner
# command's exit status (pipefail does not reach into `<()`), so the
# output is captured as a plain command substitution instead, whose own
# `$?` this script can read directly.
status=0
fixture_names_output="$(
  VP001_PROCEDURES_DIR="${procedures_dir}" timeout --kill-after=5s "${FIXTURE_NAMES_TIMEOUT}" node -e '
    (async () => {
      const { FIXTURE_NAMES } = await import(
        process.env.VP001_PROCEDURES_DIR + "/vp-s14-scenario.mjs"
      );
      process.stdout.write(
        FIXTURE_NAMES.approvedExe + "\n" + FIXTURE_NAMES.replacementExe + "\n" + FIXTURE_NAMES.shadowTargetExe + "\n"
      );
    })();
  '
)" || status=$?
if [[ "${status}" -eq 124 || "${status}" -eq 137 ]]; then
  echo "gate=fixture-names-timeout"
  exit 1
fi
# Any other nonzero status (e.g. the import itself throwing) falls through
# to the ordinary invalid-names check below: mapfile then reads fewer than
# three lines from an empty/partial output, which that check already
# rejects -- no separate gate needed for "the import failed outright".
mapfile -t fixture_names <<< "${fixture_names_output}"

# One rule, applied once per name, rather than three ad hoc conditions:
# empty, a bare "." or ".." (naming the scratch directory itself, not a
# file inside it), or containing "/" (which would let the name escape the
# scratch directory).
fixture_names_invalid=0
if [[ "${#fixture_names[@]}" -ne 3 ]]; then
  fixture_names_invalid=1
else
  for name in "${fixture_names[@]}"; do
    if [[ -z "${name}" || "${name}" == "." || "${name}" == ".." || "${name}" == */* ]]; then
      fixture_names_invalid=1
      break
    fi
  done
  # The three names must also be pairwise distinct: two equal names would
  # name the same file, so the second `cp` below would silently overwrite
  # the first and only two fixture files (not three) would ever exist.
  if [[ "${fixture_names_invalid}" -eq 0 ]] \
    && [[ "${fixture_names[0]}" == "${fixture_names[1]}" || "${fixture_names[0]}" == "${fixture_names[2]}" || "${fixture_names[1]}" == "${fixture_names[2]}" ]]; then
    fixture_names_invalid=1
  fi
fi
if [[ "${fixture_names_invalid}" -eq 1 ]]; then
  echo "gate=fixture-names-invalid"
  exit 1
fi
approved_exe="${fixture_names[0]}"
replacement_exe="${fixture_names[1]}"
shadow_target_exe="${fixture_names[2]}"

# `type -P` resolves the real external binary, bypassing bash's own
# `true`/`false` builtins -- which have no file on disk to copy.
true_bin="$(type -P true || true)"
false_bin="$(type -P false || true)"
if [[ -z "${true_bin}" || -z "${false_bin}" ]]; then
  echo "gate=fixture-source-missing"
  exit 1
fi

# Guard against a multi-call `true`/`false` (busybox, uutils): both
# applets can live in the very same binary, dispatching on argv[0]/its own
# basename, which this driver's `cp`-then-rename approach never
# preserves. If the two sources are byte-identical, case (b)'s own
# bytes-only rewrite would then change nothing at all, and the scenario
# would misread a launch of unchanged bytes as a false Fail
# (LaunchedChanged) rather than the mechanism it actually means to prove.
# Checked before any fixture is copied.
true_sha="$(sha256sum "${true_bin}" | cut -d' ' -f1)"
false_sha="$(sha256sum "${false_bin}" | cut -d' ' -f1)"
if [[ "${true_sha}" == "${false_sha}" ]]; then
  echo "gate=fixture-sources-indistinct"
  exit 1
fi

scratch_root="$(mktemp -d)"
cleanup() {
  # `rm -rf` unlinks a symlink entry itself; it never dereferences it into
  # whatever it points to, inside or outside this tree -- so the dangling
  # `approved_exe` symlink case (c) may have left behind is always safe to
  # remove this way.
  rm -rf "${scratch_root}"
}
trap cleanup EXIT

cp "${true_bin}" "${scratch_root}/${approved_exe}"
cp "${false_bin}" "${scratch_root}/${replacement_exe}"
cp "${true_bin}" "${scratch_root}/${shadow_target_exe}"
chmod 755 "${scratch_root}/${approved_exe}" "${scratch_root}/${replacement_exe}" "${scratch_root}/${shadow_target_exe}"

# A second half of the same multi-call-binary guard: distinguishable
# source bytes are not proof of distinguishable behavior once copied and
# renamed -- a multi-call binary can also fail its renamed applet lookup
# outright (busybox's own "applet not found") rather than behave like
# true/false at all. Each fixture is run directly, by its own role, under
# a short bound.
FIXTURE_BEHAVIOUR_TIMEOUT="${VP001_FIXTURE_BEHAVIOUR_TIMEOUT:-5s}"
for fixture_role_pair in "${approved_exe}:0" "${shadow_target_exe}:0" "${replacement_exe}:1"; do
  fixture_name="${fixture_role_pair%%:*}"
  expected_status="${fixture_role_pair##*:}"
  fixture_status=0
  timeout --kill-after=2s "${FIXTURE_BEHAVIOUR_TIMEOUT}" "${scratch_root}/${fixture_name}" || fixture_status=$?
  if [[ "${fixture_status}" -ne "${expected_status}" ]]; then
    echo "gate=fixture-behaviour-mismatch name=${fixture_name} status=${fixture_status}"
    exit 1
  fi
done

echo "gate=fixtures-prepared"

# The scratch root crosses through argv, never the transcript: the
# scenario itself never prints a full path (basenames only), and neither
# does this driver.
status=0
timeout --kill-after=10s "${SCENARIO_TIMEOUT}" \
  node "${procedures_dir}/vp-s14-scenario.mjs" "${webdriver_base_url}" "${artifact_path}" "${scratch_root}" \
  || status=$?

if [[ "${status}" -ne 0 ]]; then
  map_leg_status scenario "${status}" || exit $?
fi
