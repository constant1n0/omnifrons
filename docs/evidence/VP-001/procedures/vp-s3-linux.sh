#!/usr/bin/env bash
#
# VP-S3 procedure driver (desktop-stack-verification-plan.md:124).
#
# AV1/AV2 are reused from vp-s6-linux.sh --mode=feasibility-check, run as a
# subprocess exactly as vp-s1-linux.sh does (its header says why).
#
# Usage: vp-s3-linux.sh <artifact-path> <build-channel-digest>
# Env: VP001_WEBDRIVER_BASE_URL (default: http://127.0.0.1:4444)
# Exit: 0 on completion; 1 on an AV1/AV2 rejection, a scenario blocker, or
# either leg wedging past its bound (AV_TIMEOUT, SCENARIO_TIMEOUT) or being
# SIGKILLed, each named by its own gate line; 2 on a usage error,
# including a digest that is not 64 hex digits -- the caller's lookup can
# yield an empty string, which must never reach AV1. Both the AV1/AV2 leg and
# the scenario are wall-clock bounded (AV_TIMEOUT, SCENARIO_TIMEOUT); neither
# can run to the job's own timeout.

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: vp-s3-linux.sh <artifact-path> <build-channel-digest>" >&2
  exit 2
fi
artifact_path="$1"
build_channel_digest="$2"

if [[ ! -f "${artifact_path}" ]]; then
  echo "vp-s3-linux.sh: artifact not found: ${artifact_path}" >&2
  exit 2
fi
if [[ ! "${build_channel_digest}" =~ ^[0-9a-f]{64}$ ]]; then
  echo "vp-s3-linux.sh: build-channel digest is not 64 hex digits: '${build_channel_digest}'" >&2
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

# The scenario leg maps its bound's statuses exactly like the AV1/AV2 leg:
# 124 is SCENARIO_TIMEOUT firing, 137 a SIGKILL, from --kill-after or
# outside. Both exit 1 with a named gate line, never a raw status outside
# this script's documented contract; the scenario's own exit passes through.
# Known gap, shared with vp-s1-linux.sh: the SIGTERM that ends a timed-out
# scenario skips its session delete, so the packaged app can outlive it.
status=0
timeout --kill-after=10s "${SCENARIO_TIMEOUT}" \
  node "${procedures_dir}/vp-s3-scenario.mjs" "${webdriver_base_url}" "${artifact_path}" linux \
  || status=$?
if [[ "${status}" -eq 124 ]]; then
  echo "gate=scenario-timeout"
  exit 1
fi
if [[ "${status}" -eq 137 ]]; then
  echo "gate=scenario-killed"
  exit 1
fi
exit "${status}"
