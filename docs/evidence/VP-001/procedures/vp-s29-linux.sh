#!/usr/bin/env bash
#
# VP-S29 operator procedure helper (desktop-stack-verification-plan.md, VP-S29).
# Prepares the fixture, takes and compares definition digests, derives each
# leg's observation from a retained transcript, and assembles the scenario
# record. It never launches a harness or the app: the operator does that
# (vp-s29-operator.md).
#
# Usage: vp-s29-linux.sh --help
#        vp-s29-linux.sh fixture
#        vp-s29-linux.sh digests <label>
#        vp-s29-linux.sh compare <digests-a> <digests-b>
#        vp-s29-linux.sh observe <adapter> <transcript> <version-file>
#        vp-s29-linux.sh record --run-date <YYYY-MM-DD> [--record-id <id>]
#          [--baseline-id <id>] [--build-channel <channel>]
#          [--observed-state <token>] [--operator-role <role>]
# Env: VP029_RUN_ID   run identifier (required for `record`; names the artifact folder)
#      VP029_RUN_DIR  artifact folder (default: docs/evidence/VP-001/artifacts/vp-s29/<run id>)
#      VP029_ROOT     the definition root (fixture, digests)
#      VP029_HOME     the HOME holding the user-level skill (fixture; redacted from retained text)
# Exit: 0 ok; 1 a failed check (digests differ, a helper failed); 2 a usage error.

set -euo pipefail

procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${procedures_dir}/../../../.." && pwd)"

usage() {
  sed -n '3,/^# Exit:/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

die_usage() {
  echo "vp-s29-linux.sh: $1" >&2
  exit 2
}

run_dir() {
  if [[ -n "${VP029_RUN_DIR:-}" ]]; then
    printf '%s\n' "${VP029_RUN_DIR}"
  elif [[ -n "${VP029_RUN_ID:-}" ]]; then
    printf '%s\n' "${repo_root}/docs/evidence/VP-001/artifacts/vp-s29/${VP029_RUN_ID}"
  else
    die_usage "set VP029_RUN_ID or VP029_RUN_DIR"
  fi
}

# The three identity files (named whether or not they exist) and every
# skills/**/SKILL.md, as `<sha256>  <relative path>` lines; a missing identity
# file is `absent  <path>`, so an added or removed file shows in a comparison.
digest_tree() {
  local root="$1" name
  (
    cd "${root}"
    for name in AGENTS.md CLAUDE.md GEMINI.md; do
      if [[ -f "${name}" ]]; then
        sha256sum -- "${name}"
      else
        printf 'absent  %s\n' "${name}"
      fi
    done
    if [[ -d skills ]]; then
      find skills \( -type f -o -type l \) -name SKILL.md -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum --
    fi
  )
}

mode_fixture() {
  [[ -n "${VP029_ROOT:-}" && -n "${VP029_HOME:-}" ]] || die_usage "fixture needs VP029_ROOT and VP029_HOME"
  local dir
  dir="$(run_dir)"
  mkdir -p "${dir}"
  node "${procedures_dir}/vp-s29-fixture.mjs" --root "${VP029_ROOT}" --home "${VP029_HOME}" --manifest "${dir}/fixture.json"
  echo "fixture prepared; manifest: fixture.json"
}

mode_digests() {
  [[ $# -eq 1 ]] || die_usage "digests needs a label"
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || die_usage "the label is a plain word: '$1'"
  [[ -n "${VP029_ROOT:-}" && -d "${VP029_ROOT}" ]] || die_usage "digests needs VP029_ROOT to name an existing directory"
  local dir
  dir="$(run_dir)"
  mkdir -p "${dir}"
  digest_tree "${VP029_ROOT}" > "${dir}/digests-$1.txt"
  echo "wrote digests-$1.txt"
}

mode_compare() {
  [[ $# -eq 2 ]] || die_usage "compare needs two digest files"
  [[ -f "$1" && -f "$2" ]] || die_usage "digest file not found"
  if diff -u --label "$(basename "$1")" --label "$(basename "$2")" "$1" "$2"; then
    echo "digests unchanged"
  else
    echo "digests CHANGED" >&2
    return 1
  fi
}

mode_observe() {
  [[ $# -eq 3 ]] || die_usage "observe needs <adapter> <transcript> <version-file>"
  [[ -f "$2" && -f "$3" ]] || die_usage "transcript or version file not found"
  local dir manifest=() home=()
  dir="$(run_dir)"
  mkdir -p "${dir}"
  [[ -f "${dir}/fixture.json" ]] && manifest=(--manifest "${dir}/fixture.json")
  [[ -n "${VP029_HOME:-}" ]] && home=(--home "${VP029_HOME}")
  node "${procedures_dir}/vp-s29-observations.mjs" "$1" "$2" "$3" ${manifest[@]+"${manifest[@]}"} ${home[@]+"${home[@]}"} \
    --redacted-out "${dir}/transcript-$1.txt" > "${dir}/observation-$1.json" || {
      rm -f "${dir}/observation-$1.json"
      return 1
    }
  cat "${dir}/observation-$1.json"
}

mode_record() {
  local record_id="" baseline_id="" build_channel="" observed_state="" operator_role="" run_date=""
  while [[ $# -gt 0 ]]; do
    [[ $# -ge 2 ]] || die_usage "$1 needs a value"
    case "$1" in
      --record-id) record_id="$2" ;;
      --baseline-id) baseline_id="$2" ;;
      --build-channel) build_channel="$2" ;;
      --observed-state) observed_state="$2" ;;
      --operator-role) operator_role="$2" ;;
      --run-date) run_date="$2" ;;
      *) die_usage "unknown option: $1" ;;
    esac
    shift 2
  done
  local dir
  dir="$(run_dir)"
  [[ -d "${dir}" ]] || die_usage "run directory not found"
  [[ -n "${VP029_RUN_ID:-}" ]] || die_usage "record needs VP029_RUN_ID"
  [[ "${run_date}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die_usage "record needs --run-date YYYY-MM-DD"

  local unchanged=unknown
  if [[ -f "${dir}/digests-before.txt" && -f "${dir}/digests-after.txt" ]]; then
    if cmp -s "${dir}/digests-before.txt" "${dir}/digests-after.txt"; then unchanged=true; else unchanged=false; fi
  fi

  # The retained, redacted evidence in one file whose digest the record cites.
  local summary="${dir}/evidence-summary.txt" file
  : > "${summary}"
  for file in "${dir}"/observation-*.json "${dir}/digests-before.txt" "${dir}/digests-after.txt"; do
    [[ -f "${file}" ]] || continue
    printf '== %s ==\n' "$(basename "${file}")" >> "${summary}"
    cat "${file}" >> "${summary}"
  done
  for file in "${dir}"/transcript-*.txt; do
    [[ -f "${file}" ]] || continue
    printf '== sha256 of %s ==\n%s\n' "$(basename "${file}")" "$(sha256sum < "${file}" | cut -d' ' -f1)" >> "${summary}"
  done
  local digest
  digest="$(sha256sum < "${summary}" | cut -d' ' -f1)"

  local optional=()
  [[ -n "${record_id}" ]] && optional+=(--record-id "${record_id}")
  [[ -n "${baseline_id}" ]] && optional+=(--baseline-id "${baseline_id}")
  [[ -n "${build_channel}" ]] && optional+=(--build-channel "${build_channel}")
  [[ -n "${observed_state}" ]] && optional+=(--observed-state "${observed_state}")
  [[ -n "${operator_role}" ]] && optional+=(--operator-role "${operator_role}")
  node "${procedures_dir}/vp-s29-observations.mjs" record --run-dir "${dir}" --run-id "${VP029_RUN_ID}" \
    --run-date "${run_date}" --digests-unchanged "${unchanged}" \
    --evidence-path "docs/evidence/VP-001/artifacts/vp-s29/${VP029_RUN_ID}/evidence-summary.txt" \
    --evidence-digest "${digest}" ${optional[@]+"${optional[@]}"} > "${dir}/record.md"
  echo "wrote record.md and evidence-summary.txt; fill every <operator: ...> placeholder before appending to records.md"
}

if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi
case "$1" in
  --help|-h) usage ;;
  fixture) shift; mode_fixture "$@" ;;
  digests) shift; mode_digests "$@" ;;
  compare) shift; mode_compare "$@" ;;
  observe) shift; mode_observe "$@" ;;
  record) shift; mode_record "$@" ;;
  *) die_usage "unknown mode: $1" ;;
esac
