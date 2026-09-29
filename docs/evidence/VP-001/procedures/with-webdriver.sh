#!/usr/bin/env bash
#
# Shared tauri-driver bootstrap for the VP-001 Linux steps in
# .github/workflows/tauri-build.yml (feasibility gate, VP-S6, VP-S1, VP-S3):
# starts the driver, waits for its /status endpoint, runs the given command,
# and always stops the driver. One readiness loop, so the four steps cannot
# drift on how a driver that never answers, or exits at once (a missing
# binary, a port already in use), is reported.
#
# Usage: with-webdriver.sh <command> [args...]
# Env: VP001_TAURI_DRIVER (default: tauri-driver), VP001_NATIVE_DRIVER
# (default: /usr/bin/WebKitWebDriver), VP001_WEBDRIVER_BASE_URL (default:
# http://127.0.0.1:4444 -- polled at "<base>/status"), VP001_DRIVER_READY_POLLS
# (default: 10, 1 s apart, each poll bounded to STATUS_POLL_MAX_TIME, 2 s)
# Exit: 2 on a usage error (no command given). 1 and gate=driver-exited
# status=<n> if the driver process itself exits before answering ready,
# <n> being its own exit status (from `wait`). 1 and gate=driver-not-ready
# if the driver stays alive but never answers within VP001_DRIVER_READY_POLLS
# polls. <command> never runs in either case. Once the driver answers ready,
# <command> runs in the foreground and this script exits with its exact exit
# code, so a wrapped driver's own exit contract (e.g. vp-s1-linux.sh /
# vp-s3-linux.sh's own 0/1/2) passes through unchanged.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: with-webdriver.sh <command> [args...]" >&2
  exit 2
fi

tauri_driver="${VP001_TAURI_DRIVER:-tauri-driver}"
native_driver="${VP001_NATIVE_DRIVER:-/usr/bin/WebKitWebDriver}"
webdriver_base_url="${VP001_WEBDRIVER_BASE_URL:-http://127.0.0.1:4444}"
ready_polls="${VP001_DRIVER_READY_POLLS:-10}"
status_url="${webdriver_base_url}/status"
# Bounds each poll: a driver that accepts the connection but never answers
# would otherwise block one `curl` for good, and the loop with it.
STATUS_POLL_MAX_TIME=2

"${tauri_driver}" --native-driver "${native_driver}" &
driver_pid=$!
trap 'kill "${driver_pid}" 2>/dev/null || true' EXIT

# A driver that already exited is named as such, never spent down to "not
# ready" one poll at a time: there is nothing left to answer.
exit_if_driver_gone() {
  if ! kill -0 "${driver_pid}" 2>/dev/null; then
    local driver_status=0
    wait "${driver_pid}" || driver_status=$?
    echo "gate=driver-exited status=${driver_status}"
    exit 1
  fi
}

driver_ready=""
for _ in $(seq 1 "${ready_polls}"); do
  exit_if_driver_gone
  if curl -sf --max-time "${STATUS_POLL_MAX_TIME}" "${status_url}" >/dev/null 2>&1; then
    driver_ready=1
    break
  fi
  sleep 1
done

# Checked once more after the loop: a driver that died during the last poll
# is named driver-exited, and a /status answered by another listener on the
# same port never passes for this driver being ready.
exit_if_driver_gone
# A driver that never answers /status must fail here, loudly, never let the
# command run against a driver that is not actually listening.
if [[ -z "${driver_ready}" ]]; then
  echo "gate=driver-not-ready"
  exit 1
fi

# `|| status=$?`, never `if ! "$@"; then`: there `$?` would be the
# negation's own 0, turning a wrapped command's rejection into exit 0
# (vp-s1-linux.sh's own header makes the same point about its own two legs).
status=0
"$@" || status=$?
exit "${status}"
