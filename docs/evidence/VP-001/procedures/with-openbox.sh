#!/usr/bin/env bash
#
# Shared openbox bootstrap for the VP-001 Linux scenario steps in
# .github/workflows/tauri-build.yml (VP-S6 and VP-S13) that drive a GTK
# file chooser: starts openbox, waits for it to take the window-manager
# selection, runs the given command, and always stops openbox. One
# readiness loop, so the two steps cannot drift on how a window manager
# that never becomes ready, or exits at once, is reported. Readiness is
# read from xprop's own output (a window id for _NET_SUPPORTING_WM_CHECK),
# never its exit status: xprop exits 0 whether or not a window manager owns
# that property.
#
# Usage: with-openbox.sh <command> [args...]
# Must run inside an X display that already exists (the caller wraps this
# in xvfb-run; see the VP-S6 and VP-S13 scenario steps).
# Env: VP001_WM_READY_POLLS (default: 10, overridable for tests),
# VP001_WM_POLL_INTERVAL (default: 1, seconds between polls, overridable
# for tests)
# Exit: 2 on a usage error -- no command given, or an invalid
# VP001_WM_READY_POLLS/VP001_WM_POLL_INTERVAL override -- before openbox
# ever starts. 1 and gate=wm-exited status=<n> if openbox itself exits
# before taking the window-manager selection, <n> being its own exit status
# (from `wait`). 1 and gate=wm-not-ready if openbox stays alive but never
# takes the selection within VP001_WM_READY_POLLS polls. <command> never
# runs in either case. Once openbox is ready, gate=wm-ready is printed,
# <command> runs in the foreground, and this script exits with its exact
# exit code, so a wrapped command's own exit contract passes through
# unchanged.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: with-openbox.sh <command> [args...]" >&2
  exit 2
fi

ready_polls="${VP001_WM_READY_POLLS:-10}"
poll_interval="${VP001_WM_POLL_INTERVAL:-1}"

# Both validated before openbox ever starts: a bad override must fail
# closed as a usage error, never spend a poll budget or leave openbox
# running behind a value the loop below cannot make sense of.
if [[ ! "${ready_polls}" =~ ^[1-9][0-9]*$ ]]; then
  echo "usage: VP001_WM_READY_POLLS must be a positive integer, got '${ready_polls}'" >&2
  exit 2
fi
# Fractional is allowed, since `sleep` itself accepts it; "0", "0.0", and
# ".0" are rejected by checking that something other than a zero digit or a
# decimal point survives -- a plain >0 numeric comparison cannot do this in
# POSIX/bash arithmetic, which is integer-only.
if [[ ! "${poll_interval}" =~ ^([0-9]+\.?[0-9]*|\.[0-9]+)$ ]] || [[ -z "${poll_interval//[0.]/}" ]]; then
  echo "usage: VP001_WM_POLL_INTERVAL must be a positive number, got '${poll_interval}'" >&2
  exit 2
fi

openbox &
wm_pid=$!
trap 'kill "${wm_pid}" 2>/dev/null || true' EXIT

# openbox that already exited is named as such, never spent down to
# "not ready" one poll at a time: there is nothing left to become ready.
exit_if_wm_gone() {
  if ! kill -0 "${wm_pid}" 2>/dev/null; then
    local wm_status=0
    wait "${wm_pid}" || wm_status=$?
    echo "gate=wm-exited status=${wm_status}"
    exit 1
  fi
}

wm_ready=""
# A counted arithmetic loop, never `seq`: GNU and BSD `seq` differ in some
# corner cases, and `ready_polls` (validated above as a plain positive
# integer) is safe to use directly in an arithmetic context.
for ((attempt = 0; attempt < ready_polls; attempt++)); do
  exit_if_wm_gone
  # xprop exits 0 either way, whether or not a window manager owns the
  # property -- "no such atom on any window." is itself a successful,
  # well-formed answer -- so readiness has to come from the output, never
  # the exit status. Captured rather than piped into `grep -q`: under
  # `pipefail`, a `grep` that matches and exits early can still SIGPIPE
  # xprop, which would otherwise turn a genuine match into a false negative.
  wm_check="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)"
  if [[ "${wm_check}" == *"window id #"* ]]; then
    wm_ready=1
    break
  fi
  sleep "${poll_interval}"
done

# Checked once more after the loop: openbox that died during the last poll
# is named wm-exited, never wm-not-ready.
exit_if_wm_gone
# openbox that never takes the window-manager selection must fail here,
# loudly, never let the command run against a display with no window
# manager on it.
if [[ -z "${wm_ready}" ]]; then
  echo "gate=wm-not-ready"
  exit 1
fi
echo "gate=wm-ready"

# `|| status=$?`, never `if ! "$@"; then`: see with-webdriver.sh's own note
# on the same point -- `$?` there would be the negation's own 0, turning a
# wrapped command's rejection into exit 0.
status=0
"$@" || status=$?
exit "${status}"
