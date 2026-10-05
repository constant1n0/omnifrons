#!/usr/bin/env bash
#
# Shared named-test selection helper for the VP-001 static evidence steps in
# .github/workflows/tauri-build.yml (VP-S13, VP-S14, ...). Library only --
# source it, never execute it -- so every static step's selection logic
# cannot drift on how a named-test run is gated and reported.
#
# Usage: source this file, then call:
#   pinned <gate-prefix> <cargo args...> -- <test names...>
# runs exactly the named tests (--exact) and requires every one of them to
# pass, so a renamed test -- which a name filter would silently match zero
# times -- fails the selection. stderr is kept (merged via `2>&1` before the
# pass/fail decision), so a build failure leaves its cause in the artifact,
# not only in the job log.
#
# Exit: 2 on a usage error (no `--` among the arguments, or no test names
# after it) -- cargo never runs. 1 and
# `gate=<gate-prefix>-selection-failed expected=<N> passed=<M> status=<S>`
# if cargo exited nonzero or fewer than N of the N named tests passed
# (N is the count of names given, M the count of "test result: ok. ...
# passed" lines summed across cargo's output, S cargo's own exit status).
# 0 otherwise, with cargo's output already echoed.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "pinned-tests.sh: library only, source it -- do not execute it directly" >&2
  exit 1
fi

pinned() {
  if [[ $# -lt 1 ]]; then
    echo "usage: pinned <gate-prefix> <cargo args...> -- <test names...>" >&2
    return 2
  fi
  local gate_prefix="$1"
  shift

  local args=()
  while [[ $# -gt 0 && "$1" != "--" ]]; do
    args+=("$1")
    shift
  done

  if [[ "${1-}" != "--" ]]; then
    echo "usage: pinned <gate-prefix> <cargo args...> -- <test names...>" >&2
    return 2
  fi
  shift

  if [[ $# -eq 0 ]]; then
    echo "usage: pinned <gate-prefix> <cargo args...> -- <test names...>" >&2
    return 2
  fi
  local names=("$@")

  local out status=0 passed
  out="$(cargo test "${args[@]}" -- --exact "${names[@]}" 2>&1)" || status=$?
  printf '%s\n' "${out}"
  passed="$(printf '%s\n' "${out}" | sed -n 's/^test result: ok\. \([0-9][0-9]*\) passed.*/\1/p' | awk '{n += $1} END {print n + 0}')"
  if [[ "${status}" -ne 0 || "${passed}" -ne "${#names[@]}" ]]; then
    echo "gate=${gate_prefix}-selection-failed expected=${#names[@]} passed=${passed} status=${status}"
    return 1
  fi
}
