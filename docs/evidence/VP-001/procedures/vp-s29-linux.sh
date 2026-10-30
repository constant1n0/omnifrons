#!/usr/bin/env bash
#
# VP-S29 operator procedure helper (desktop-stack-verification-plan.md, VP-S29).
# Prepares the fixture, takes and compares definition digests, derives each
# leg's observation from a retained transcript, and assembles the scenario
# record. It never launches a harness or the app: the operator does that
# (vp-s29-operator.md).
#
# usage-begin
# Usage: vp-s29-linux.sh --help
#        vp-s29-linux.sh fixture
#        vp-s29-linux.sh digests <label>   (before; after-<adapter>-<1|2> per invocation)
#        vp-s29-linux.sh compare <digests-a> <digests-b>
#        vp-s29-linux.sh observe <adapter> <transcript> <version-file>
#        vp-s29-linux.sh record --run-date <YYYY-MM-DD> [--record-id <id>]
#          [--baseline-id <id>] [--build-channel <channel, default packaged-ci>]
#          [--build-channel-digest <sha256> --exercised-artifact-digest <sha256>]
#          [--evidence-artifact <file under the run folder, default evidence-summary.txt>]
#          [--observed-state <token, default uncertain>] [--operator-role <role>]
# Env: VP_S29_RUN_ID   run identifier (required for `record`; names the artifact folder)
#      VP_S29_RUN_DIR  artifact folder (default: docs/evidence/VP-001/artifacts/vp-s29/<run id>)
#      VP_S29_ROOT     the definition root (fixture, digests)
#      VP_S29_HOME     the HOME holding the user-level skill (fixture; redacted from retained text)
# Exit: 0 ok; 1 a failed check (digests differ, a helper failed); 2 a usage error.
# usage-end

set -euo pipefail

procedures_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${procedures_dir}/../../../.." && pwd)"

usage() {
  sed -n '/^# usage-begin$/,/^# usage-end$/p' "${BASH_SOURCE[0]}" | sed '1d;$d' | sed 's/^# \{0,1\}//'
}

die_usage() {
  echo "vp-s29-linux.sh: $1" >&2
  exit 2
}

run_dir() {
  if [[ -n "${VP_S29_RUN_DIR:-}" ]]; then
    printf '%s\n' "${VP_S29_RUN_DIR}"
  elif [[ -n "${VP_S29_RUN_ID:-}" ]]; then
    printf '%s\n' "${repo_root}/docs/evidence/VP-001/artifacts/vp-s29/${VP_S29_RUN_ID}"
  else
    die_usage "set VP_S29_RUN_ID or VP_S29_RUN_DIR"
  fi
}

# The three identity files (named whether or not they exist) and every
# skills/**/SKILL.md, as `<sha256>  <relative path>` lines; a missing identity
# file is `absent  <path>`, so an added or removed file shows in a comparison.
# The host project-skill folders are listed `present`/`absent` too: none may
# appear in the definition root during a run.
digest_tree() {
  local root="$1" list status=0
  list="$(mktemp)" || return 1
  # Every step checks its own status: this runs under `if !`, where errexit is off.
  (
    cd "${root}" || exit 1
    local name target hash
    for name in AGENTS.md CLAUDE.md GEMINI.md; do
      if [[ -f "${name}" ]]; then
        sha256sum -- "${name}" || exit 1
      else
        printf 'absent  %s\n' "${name}"
      fi
    done
    for name in .agents/skills .gemini/skills; do
      if [[ -e "${name}" ]]; then
        printf 'present  %s\n' "${name}"
      else
        printf 'absent  %s\n' "${name}"
      fi
    done
    if [[ -d skills ]]; then
      # The listing goes to a file first so a failing find (an unreadable
      # subdirectory) fails the mode instead of yielding a partial digest.
      find skills \( -type f -o -type l \) -name SKILL.md -print0 | LC_ALL=C sort -z > "${list}" || exit 1
      while IFS= read -r -d '' name; do
        if [[ -L "${name}" ]]; then
          # A symlink is listed with its target and, when it resolves to a
          # regular file, that file's content digest; the listing never follows it.
          target="$(readlink -- "${name}")" || exit 1
          if [[ -f "${name}" ]]; then
            hash="$(sha256sum -- "${name}" | cut -d' ' -f1)" || exit 1
          else
            hash=dangling
          fi
          printf 'symlink  %s -> %s  %s\n' "${name}" "${target}" "${hash}"
        else
          sha256sum -- "${name}" || exit 1
        fi
      done < "${list}"
    fi
  ) || status=$?
  rm -f "${list}"
  return "${status}"
}

mode_fixture() {
  [[ -n "${VP_S29_ROOT:-}" && -n "${VP_S29_HOME:-}" ]] || die_usage "fixture needs VP_S29_ROOT and VP_S29_HOME"
  local dir
  dir="$(run_dir)"
  mkdir -p "${dir}"
  node "${procedures_dir}/vp-s29-fixture.mjs" --root "${VP_S29_ROOT}" --home "${VP_S29_HOME}" --manifest "${dir}/private/fixture.json"
  echo "fixture prepared; manifest: private/fixture.json (local paths, ignored by git)"
}

mode_digests() {
  [[ $# -eq 1 ]] || die_usage "digests needs a label"
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || die_usage "the label is a plain word: '$1'"
  [[ -n "${VP_S29_ROOT:-}" && -d "${VP_S29_ROOT}" ]] || die_usage "digests needs VP_S29_ROOT to name an existing directory"
  local dir
  dir="$(run_dir)"
  mkdir -p "${dir}"
  # Written to a temporary file and moved into place only when complete.
  if ! digest_tree "${VP_S29_ROOT}" > "${dir}/.digests-$1.tmp"; then
    rm -f "${dir}/.digests-$1.tmp"
    echo "vp-s29-linux.sh: digests failed" >&2
    return 1
  fi
  mv "${dir}/.digests-$1.tmp" "${dir}/digests-$1.txt"
  echo "wrote digests-$1.txt"
}

mode_compare() {
  [[ $# -eq 2 ]] || die_usage "compare needs two digest files"
  [[ -f "$1" && -f "$2" ]] || die_usage "digest file not found"
  local status=0
  diff -u --label "$(basename "$1")" --label "$(basename "$2")" "$1" "$2" || status=$?
  case "${status}" in
    0) echo "digests unchanged" ;;
    1) echo "digests CHANGED" >&2; return 1 ;;
    *) echo "vp-s29-linux.sh: diff failed (status ${status})" >&2; return 2 ;;
  esac
}

mode_observe() {
  [[ $# -eq 3 ]] || die_usage "observe needs <adapter> <transcript> <version-file>"
  [[ -f "$2" && -f "$3" ]] || die_usage "transcript or version file not found"
  local dir manifest=() home=()
  dir="$(run_dir)"
  mkdir -p "${dir}"
  [[ -f "${dir}/private/fixture.json" ]] && manifest=(--manifest "${dir}/private/fixture.json")
  [[ -n "${VP_S29_HOME:-}" ]] && home=(--home "${VP_S29_HOME}")
  node "${procedures_dir}/vp-s29-observations.mjs" "$1" "$2" "$3" ${manifest[@]+"${manifest[@]}"} ${home[@]+"${home[@]}"} \
    --redacted-out "${dir}/transcript-$1.txt" > "${dir}/observation-$1.json" || {
      rm -f "${dir}/observation-$1.json" "${dir}/transcript-$1.txt"
      return 1
    }
  cat "${dir}/observation-$1.json"
}

mode_record() {
  local record_id="" baseline_id="" build_channel="" observed_state="" operator_role="" run_date=""
  local channel_digest="" artifact_digest="" evidence_artifact="evidence-summary.txt"
  while [[ $# -gt 0 ]]; do
    [[ $# -ge 2 ]] || die_usage "$1 needs a value"
    case "$1" in
      --record-id) record_id="$2" ;;
      --baseline-id) baseline_id="$2" ;;
      --build-channel) build_channel="$2" ;;
      --observed-state) observed_state="$2" ;;
      --operator-role) operator_role="$2" ;;
      --run-date) run_date="$2" ;;
      --build-channel-digest) channel_digest="$2" ;;
      --exercised-artifact-digest) artifact_digest="$2" ;;
      --evidence-artifact) evidence_artifact="$2" ;;
      *) die_usage "unknown option: $1" ;;
    esac
    shift 2
  done
  local dir
  dir="$(run_dir)"
  [[ -d "${dir}" ]] || die_usage "run directory not found"
  [[ -n "${VP_S29_RUN_ID:-}" ]] || die_usage "record needs VP_S29_RUN_ID"
  [[ "${run_date}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die_usage "record needs --run-date YYYY-MM-DD"

  if [[ -n "${channel_digest}${artifact_digest}" ]]; then
    [[ "${channel_digest}" =~ ^[0-9a-f]{64}$ && "${artifact_digest}" =~ ^[0-9a-f]{64}$ ]] \
      || die_usage "give both --build-channel-digest and --exercised-artifact-digest, each 64 hex digits"
  fi
  if [[ "${evidence_artifact}" == /* || "${evidence_artifact}" == ".." || "${evidence_artifact}" == ../* \
    || "${evidence_artifact}" == */../* || "${evidence_artifact}" == */.. ]]; then
    die_usage "--evidence-artifact is a relative path inside the run folder"
  fi
  if [[ "${evidence_artifact}" != evidence-summary.txt && ! -f "${dir}/${evidence_artifact}" ]]; then
    die_usage "evidence artifact not found in the run folder: ${evidence_artifact}"
  fi

  # One digest file per harness invocation (digests after-<adapter>-<1|2>),
  # each compared with `before`. Changed anywhere means changed overall; a
  # missing invocation keeps the overall value `unknown`, never `true`.
  local unchanged=true adapter index label expected=" " invocations=""
  if [[ ! -f "${dir}/digests-before.txt" ]]; then unchanged=unknown; fi
  record_invocation() {
    local state
    if [[ ! -f "${dir}/digests-$1.txt" ]]; then
      state=missing
      [[ "${unchanged}" == true ]] && unchanged=unknown
    elif [[ ! -f "${dir}/digests-before.txt" ]]; then
      state=unknown
    elif cmp -s "${dir}/digests-before.txt" "${dir}/digests-$1.txt"; then
      state=true
    else
      state=false
      unchanged=false
    fi
    invocations+="${invocations:+,}$1=${state}"
  }
  for adapter in claude-code stream-json-cli pty-cli; do
    for index in 1 2; do
      label="after-${adapter}-${index}"
      expected+="${label} "
      record_invocation "${label}"
    done
  done
  for file in "${dir}"/digests-after-*.txt; do
    [[ -f "${file}" ]] || continue
    label="$(basename "${file}" .txt)"
    label="${label#digests-}"
    [[ "${expected}" == *" ${label} "* ]] || record_invocation "${label}"
  done

  # The retained, redacted evidence in one file whose digest the record cites.
  local summary="${dir}/evidence-summary.txt" file
  : > "${summary}"
  for file in "${dir}"/observation-*.json "${dir}/digests-before.txt" "${dir}"/digests-after-*.txt; do
    [[ -f "${file}" ]] || continue
    printf '== %s ==\n' "$(basename "${file}")" >> "${summary}"
    cat "${file}" >> "${summary}"
  done
  for file in "${dir}"/transcript-*.txt; do
    [[ -f "${file}" ]] || continue
    printf '== sha256 of %s ==\n%s\n' "$(basename "${file}")" "$(sha256sum < "${file}" | cut -d' ' -f1)" >> "${summary}"
  done
  local digest
  digest="$(sha256sum < "${dir}/${evidence_artifact}" | cut -d' ' -f1)"

  local optional=()
  [[ -n "${record_id}" ]] && optional+=(--record-id "${record_id}")
  [[ -n "${baseline_id}" ]] && optional+=(--baseline-id "${baseline_id}")
  [[ -n "${build_channel}" ]] && optional+=(--build-channel "${build_channel}")
  [[ -n "${channel_digest}" ]] && optional+=(--build-channel-digest "${channel_digest}" --exercised-artifact-digest "${artifact_digest}")
  [[ -n "${observed_state}" ]] && optional+=(--observed-state "${observed_state}")
  [[ -n "${operator_role}" ]] && optional+=(--operator-role "${operator_role}")
  node "${procedures_dir}/vp-s29-observations.mjs" record --run-dir "${dir}" --run-id "${VP_S29_RUN_ID}" \
    --run-date "${run_date}" --digests-unchanged "${unchanged}" --invocations "${invocations}" \
    --evidence-path "docs/evidence/VP-001/artifacts/vp-s29/${VP_S29_RUN_ID}/${evidence_artifact}" \
    --evidence-digest "${digest}" ${optional[@]+"${optional[@]}"} > "${dir}/.record.md.tmp" || {
      rm -f "${dir}/.record.md.tmp"
      return 1
    }
  mv "${dir}/.record.md.tmp" "${dir}/record.md"
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
