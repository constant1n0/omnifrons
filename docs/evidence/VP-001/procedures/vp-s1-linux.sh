#!/usr/bin/env bash
#
# VP-S1 procedure driver (desktop-stack-verification-plan.md:122).
#
# AV1/AV2 are reused from vp-s6-linux.sh's own --mode=feasibility-check
# rather than duplicated here: that script has no
# `[[ "${BASH_SOURCE[0]}" == "$0" ]]` guard around its mode dispatch, so
# sourcing it would also run its own F1 session as a side effect -- not a
# clean function import. Invoking it as a subprocess is the smaller diff
# (this file only), at the cost of one extra, harmless session create/close.
#
# Usage: vp-s1-linux.sh <artifact-path> <build-channel-digest>
# Env: VP001_WEBDRIVER_BASE_URL (default: http://127.0.0.1:4444)
# Exit: 0 on completion; 1 on an AV1/AV2 rejection, an AV1/AV2 leg that wedged
# past AV_TIMEOUT or was SIGKILLed, or a scenario blocker; 2 on a usage error,
# including a digest that is not 64 hex digits -- the caller's lookup can
# yield an empty string, which must never reach AV1. Both the AV1/AV2 leg and
# the scenario are wall-clock bounded (AV_TIMEOUT, SCENARIO_TIMEOUT); neither
# can run to the job's own timeout.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: vp-s1-linux.sh <artifact-path> <build-channel-digest>" >&2
  exit 2
fi
artifact_path="$1"
build_channel_digest="$2"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s1-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi
if [[ ! "${build_channel_digest}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "vp-s1-linux.sh: build-channel digest is not 64 hex digits: '${build_channel_digest}'" >&2
  exit 2
fi

# Wall-clock bound on the scenario: a wedged WebDriver call must fail here
# in minutes, never run to the job's own timeout.
SCENARIO_TIMEOUT="${VP001_SCENARIO_TIMEOUT:-300s}"
# Wall-clock bound on the reused AV1/AV2 leg (vp-s6-linux.sh's own gate plus
# its F1 session create/close) -- normally seconds, never a full scenario --
# so a wedge there fails loudly here too, instead of holding this step alive
# until the job's own timeout.
AV_TIMEOUT="${VP001_AV_TIMEOUT:-120s}"

webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# vp-s6-linux.sh's own transcript lines (its AV1/AV2 gates, plus its own F1
# gate=f1-session-created / gate=session-closed pair) pass through to this
# script's stdout unchanged -- a superset, never a rewrite, of this file's own.
# The status is captured with `|| status=$?`, never inside `if ! ...; then`:
# there `$?` is the negation's own 0, which would turn a rejection into exit 0.
status=0
timeout --kill-after=10s "${AV_TIMEOUT}" \
  "${procedures_dir}/vp-s6-linux.sh" --mode=feasibility-check "${artifact_path}" "${build_channel_digest}" \
  || status=$?
if [[ "${status}" -ne 0 ]]; then
  # vp-s6-linux.sh exits only 0/1/2 and runs no `timeout` of its own, so
  # 124 is this bound firing: a wedge, named as one. 137 (128+SIGKILL) is
  # either --kill-after forcing the issue or a SIGKILL from outside (the
  # OOM killer, say) -- the status cannot tell which, so it is named a kill,
  # never a timeout. Both exit 1, the status this script documents for
  # them; an AV1/AV2 rejection already printed its own `gate=<token>` line.
  # Any other status passes through unchanged, preserving vp-s6-linux.sh's
  # own exit contract.
  if [[ "${status}" -eq 124 ]]; then
    echo "gate=av-feasibility-check-timeout"
    exit 1
  fi
  if [[ "${status}" -eq 137 ]]; then
    echo "gate=av-feasibility-check-killed"
    exit 1
  fi
  exit "${status}"
fi

timeout --kill-after=10s "${SCENARIO_TIMEOUT}" \
  node "${procedures_dir}/vp-s1-scenario.mjs" "${webdriver_base_url}" "${artifact_path}"
