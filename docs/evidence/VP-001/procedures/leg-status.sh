#!/usr/bin/env bash
#
# Shared exit-status mapping for vp-s1-linux.sh and vp-s3-linux.sh's AV1/AV2
# and scenario legs. Library only -- sourced via `procedures_dir`, never
# executed -- so the four legs (two scripts x two legs) cannot drift on how
# a `timeout`-wrapped child's exit status becomes a `gate=<leg>-<reason>`
# line and this script's own 0/1/2 contract.
#
# Usage: after `timeout --kill-after=10s ... || status=$?` captures a
# nonzero status, call `map_leg_status <leg> <status> || exit $?`. `leg` is
# the token used in the gate line, e.g. `av-feasibility-check` or
# `scenario`, matching each driver's existing gate names exactly.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "leg-status.sh: library only, source it -- do not execute it directly" >&2
  exit 1
fi

map_leg_status() {
  local leg="$1"
  local status="$2"

  case "${status}" in
    0)
      return 0
      ;;
    1 | 2)
      # The child's own contract, not this bound's: for the AV leg it comes
      # with vp-s6-linux.sh's own gate line, for the scenario leg with
      # whatever it prints for its own blocker.
      return "${status}"
      ;;
    124)
      # `timeout`'s own bound fired -- a wedge, named as one.
      echo "gate=${leg}-timeout"
      return 1
      ;;
    137)
      # 128+SIGKILL: either --kill-after forcing the issue or a SIGKILL from
      # outside (the OOM killer, say) -- the status cannot tell which, so it
      # is named a kill, never a timeout.
      echo "gate=${leg}-killed"
      return 1
      ;;
    143)
      # 128+SIGTERM from outside this bound (an external kill or job
      # cancellation, say): `timeout` itself reports 124 when its own bound
      # fires, so a raw SIGTERM exit here did not come from that bound.
      echo "gate=${leg}-terminated"
      return 1
      ;;
    *)
      # 125 (timeout itself failed to run the command), 126 (found but not
      # executable), 127 (not found), or anything else outside this
      # script's documented 0/1/2 contract -- named with the raw status
      # rather than silently passed through.
      echo "gate=${leg}-unexpected-exit status=${status}"
      return 1
      ;;
  esac
}
