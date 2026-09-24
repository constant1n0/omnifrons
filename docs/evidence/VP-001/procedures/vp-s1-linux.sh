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
# Exit: 0 on completion; 1 on an AV1/AV2 rejection or a scenario blocker;
# 2 on a usage error, including a digest that is not 64 hex digits -- the
# caller's lookup can yield an empty string, which must never reach AV1.
# The scenario itself is bounded by SCENARIO_TIMEOUT.

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

webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# vp-s6-linux.sh's own transcript lines (its AV1/AV2 gates, plus its own F1
# gate=f1-session-created / gate=session-closed pair) pass through to this
# script's stdout unchanged -- a superset, never a rewrite, of this file's own.
"${procedures_dir}/vp-s6-linux.sh" --mode=feasibility-check "${artifact_path}" "${build_channel_digest}"

timeout --kill-after=10s "${SCENARIO_TIMEOUT}" \
  node "${procedures_dir}/vp-s1-scenario.mjs" "${webdriver_base_url}" "${artifact_path}"
