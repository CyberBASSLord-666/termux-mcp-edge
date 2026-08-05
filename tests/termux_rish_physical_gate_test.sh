#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
GATE="$REPO_ROOT/scripts/termux_rish_physical_gate.sh"
OPERATOR_GATE="$REPO_ROOT/scripts/operator_local_s25_rish_gate.sh"
CONTROLLER="$REPO_ROOT/scripts/shizuku_rish_physical_controller.sh"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
SHA_A=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
SHA_B=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
SHA_C=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc

fail_test() {
  printf 'Termux rish physical gate test failed: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  local reason="$1"
  shift
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded: $reason"
  fi
  grep -Fxq "TERMUX_RISH_PHYSICAL_GATE_RESULT=FAIL reason=$reason" \
    "$ROOT/last.stderr" \
    || fail_test "expected failure reason was absent: $reason"
  [[ ! -s "$ROOT/last.stdout" ]] \
    || fail_test "failed gate wrote standard output: $reason"
}

bash -n "$GATE"
bash -n "$OPERATOR_GATE"
bash -n "${BASH_SOURCE[0]}"
bash "$GATE" --help | grep -Fq 'Usage:' \
  || fail_test help_contract_missing

assert_fails expected_commit_invalid bash "$GATE"
assert_fails expected_version_invalid \
  bash "$GATE" --expected-commit "$COMMIT" --expected-version 'not valid'
assert_fails artifact_digest_invalid \
  bash "$GATE" --expected-commit "$COMMIT" --expected-version 0.7.0

grep -Fq '"PATH=$PREFIX/bin:/system/bin"' "$GATE" \
  || fail_test sanitized_runtime_path_missing
grep -Fq 'set +m' "$GATE" \
  || fail_test physical_gate_monitor_mode_not_disabled
grep -Fq '"$PREFIX/bin/setsid" --wait -- "$ARTIFACT"' "$GATE" \
  || fail_test physical_gate_setsid_wait_contract_missing
grep -Fq 'MCP__COMMAND__ENABLED=false' "$GATE" \
  || fail_test general_command_lane_not_disabled
grep -Fq 'MCP__ANDROID__RISH_ENABLED=$rish_enabled' "$GATE" \
  || fail_test fixed_rish_gate_missing
grep -Fq 'and (has("required") | not)' "$GATE" \
  || fail_test empty_schema_required_field_absence_not_asserted
if grep -Fq '.required == []' "$GATE"; then
  fail_test empty_schema_invented_required_array
fi
for art_key in \
  ANDROID_ART_ROOT ANDROID_ASSETS ANDROID_DATA ANDROID_I18N_ROOT \
  ANDROID_ROOT ANDROID_RUNTIME_ROOT ANDROID_STORAGE ANDROID_TZDATA_ROOT \
  ANDROID__BUILD_VERSION_SDK BOOTCLASSPATH DEX2OATBOOTCLASSPATH \
  SYSTEMSERVERCLASSPATH
do
  grep -Fxq "  $art_key" "$GATE" \
    || fail_test "ART runtime environment allowlist omits $art_key"
done
grep -Fq 'androidRishArbitraryShell == false' "$GATE" \
  || fail_test arbitrary_shell_boundary_not_asserted
grep -Fq 'androidRishMutations == false' "$GATE" \
  || fail_test mutation_boundary_not_asserted
grep -Fq 'reasonCode == "rish_dex_identity_changed"' "$GATE" \
  || fail_test dex_identity_replacement_not_tested
grep -Fq 'rikka.shizuku.shell.ShizukuShellLoader' "$GATE" \
  || fail_test trusted_direct_loader_missing
grep -Fq "'exec /system/bin/id -u'" "$GATE" \
  || fail_test trusted_direct_command_missing
grep -Fq 'RISH_APPLICATION_ID=com.termux' "$GATE" \
  || fail_test trusted_direct_application_id_missing
grep -Fq 'shizukuStartModeObserved: false' "$GATE" \
  || fail_test unobserved_shizuku_start_mode_boundary_missing
if grep -Fq 'shizukuStartMode: "adb"' "$GATE"; then
  fail_test unobserved_shizuku_start_mode_claimed
fi
grep -Fq 'trusted_direct_rish_probe pre_candidate' "$GATE" \
  || fail_test pre_candidate_trusted_probe_missing
grep -Fq 'trusted_direct_rish_probe post_candidate' "$GATE" \
  || fail_test post_candidate_trusted_probe_missing
if grep -Fq '"$ARTIFACT" --version' "$GATE"; then
  fail_test candidate_executed_during_artifact_validation
fi
if grep -Eq \
  '(^|[[:space:]])(pkg|apt|apt-get|dpkg)[[:space:]]+(install|update|upgrade)|adb[[:space:]]+root' \
  "$GATE"
then
  fail_test package_or_root_escalation_present
fi
if grep -Eq -- '--(command|argv|stdin|environment)([[:space:]]|$)' "$GATE"; then
  fail_test caller_selected_command_surface_present
fi
grep -Fq 'uid_predicate_observed_in_one_bounded_capture' "$OPERATOR_GATE" \
  || fail_test operator_local_exact_token_helper_missing
grep -Fq '((RISH_LOCAL_STATUS == 0)) || fail rish_launcher_failed' "$OPERATOR_GATE" \
  || fail_test operator_local_launcher_status_not_checked
grep -Fq 'remote exit/stream/Binder lifecycle unqualified' "$OPERATOR_GATE" \
  || fail_test operator_local_diagnostic_boundary_missing
grep -Fq 'ulimit -f 8' "$OPERATOR_GATE" \
  || fail_test operator_local_pre_capture_file_limit_missing
grep -Fq 'RISH_APPLICATION_ID=com.termux' "$OPERATOR_GATE" \
  || fail_test operator_local_fixed_rish_application_id_missing
grep -Fq 'exec env -i "${server_environment[@]}"' "$OPERATOR_GATE" \
  || fail_test operator_local_sanitized_environment_missing
grep -Fq 'MCP__FILE__WRITE_MUTATION_ENABLED=false' "$OPERATOR_GATE" \
  || fail_test operator_local_mutation_posture_not_closed
grep -Fq 'MCP__COMMAND__ENABLED=false' "$OPERATOR_GATE" \
  || fail_test operator_local_command_posture_not_closed
grep -Fq 'TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none' "$OPERATOR_GATE" \
  || fail_test operator_local_random_token_generation_missing
if grep -Fq 'operator-local-s25-token' "$OPERATOR_GATE"; then
  fail_test operator_local_predictable_token_retained
fi
grep -Fq 'set +m' "$OPERATOR_GATE" \
  || fail_test operator_local_monitor_mode_not_disabled
grep -Fq '"$PREFIX/bin/setsid" --wait -- "$ARTIFACT"' "$OPERATOR_GATE" \
  || fail_test operator_local_setsid_wait_contract_missing
grep -Fq 'kill -TERM -- "-$pid"' "$OPERATOR_GATE" \
  || fail_test operator_local_bounded_TERM_cleanup_missing
grep -Fq 'kill -KILL -- "-$pid"' "$OPERATOR_GATE" \
  || fail_test operator_local_bounded_KILL_cleanup_missing
grep -Fq -- '--argjson expected "$ENABLED_TOOLS"' "$OPERATOR_GATE" \
  || fail_test operator_local_exact_tool_inventory_missing
grep -Fq '.result.structuredContent.availableTools == $expected' "$OPERATOR_GATE" \
  || fail_test operator_local_runtime_inventory_parity_missing
if grep -Fq 'RISH_OUT="$(' "$OPERATOR_GATE"; then
  fail_test operator_local_stdout_only_command_substitution_retained
fi
stable_copy_line="$(grep -nF 'cp -f -- "$EVIDENCE" "$STABLE"' "$OPERATOR_GATE" | cut -d: -f1)"
cleanup_line="$(grep -nF 'if ! cleanup_runtime; then' "$OPERATOR_GATE" | cut -d: -f1)"
pass_line="$(grep -nF 'log "PASS android_rish_status exact UID-2000' "$OPERATOR_GATE" | cut -d: -f1)"
[[ "$stable_copy_line" =~ ^[0-9]+$ && "$cleanup_line" =~ ^[0-9]+$ \
  && "$pass_line" =~ ^[0-9]+$ \
  && "$stable_copy_line" -lt "$cleanup_line" && "$cleanup_line" -lt "$pass_line" ]] \
  || fail_test operator_local_PASS_precedes_stable_copy_or_cleanup

(
  # shellcheck source=/dev/null
  source "$OPERATOR_GATE"
  printf '2000\n' >"$ROOT/operator.expected"
  : >"$ROOT/operator.stdout"
  : >"$ROOT/operator.stderr"

  cp "$ROOT/operator.expected" "$ROOT/operator.stdout"
  uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr" \
    || fail_test operator_local_stdout_token_rejected

  : >"$ROOT/operator.stdout"
  cp "$ROOT/operator.expected" "$ROOT/operator.stderr"
  uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr" \
    || fail_test operator_local_stderr_token_rejected

  cp "$ROOT/operator.expected" "$ROOT/operator.stdout"
  if uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr"
  then
    fail_test operator_local_duplicate_token_accepted
  fi

  printf '20' >"$ROOT/operator.stdout"
  printf '00\n' >"$ROOT/operator.stderr"
  if uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr"
  then
    fail_test operator_local_split_token_accepted
  fi

  cp "$ROOT/operator.expected" "$ROOT/operator.stdout"
  printf 'warning\n' >"$ROOT/operator.stderr"
  if uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr"
  then
    fail_test operator_local_injected_token_accepted
  fi

  dd if=/dev/zero of="$ROOT/operator.stdout" bs=1025 count=1 status=none
  : >"$ROOT/operator.stderr"
  if uid_predicate_observed_in_one_bounded_capture \
    "$ROOT/operator.expected" "$ROOT/operator.stdout" "$ROOT/operator.stderr"
  then
    fail_test operator_local_oversize_capture_accepted
  fi

  for key in "${ART_RUNTIME_ENV_KEYS[@]}"; do
    unset "$key"
  done
  export ANDROID_ROOT=/system
  export BOOTCLASSPATH=/apex/com.android.art/javalib/core-oj.jar
  export UNRELATED_RUNTIME_VALUE=must-not-pass
  capture_art_runtime_environment
  [[ "${#ART_RUNTIME_ENV[@]}" == 2 \
    && "${ART_RUNTIME_ENV[0]}" == ANDROID_ROOT=/system \
    && "${ART_RUNTIME_ENV[1]}" == BOOTCLASSPATH=/apex/com.android.art/javalib/core-oj.jar ]] \
    || fail_test operator_local_ART_runtime_environment_allowlist_invalid
)

(
  # shellcheck source=/dev/null
  source "$GATE"

  is_sha256 "$SHA_A" || fail_test valid_digest_rejected
  if is_sha256 "${SHA_A}g"; then
    fail_test invalid_digest_accepted
  fi

  for key in "${ART_RUNTIME_ENV_KEYS[@]}"; do
    unset "$key"
  done
  export ANDROID_ROOT=/system
  export BOOTCLASSPATH=/apex/com.android.art/javalib/core-oj.jar
  export UNRELATED_RUNTIME_VALUE=must-not-pass
  capture_art_runtime_environment
  [[ "${#ART_RUNTIME_ENV[@]}" == 2 \
    && "${ART_RUNTIME_ENV[0]}" == ANDROID_ROOT=/system \
    && "${ART_RUNTIME_ENV[1]}" == BOOTCLASSPATH=/apex/com.android.art/javalib/core-oj.jar ]] \
    || fail_test ART_runtime_environment_allowlist_invalid

  printf '2000\n' >"$ROOT/uid.expected"
  : >"$ROOT/uid.stdout"
  : >"$ROOT/uid.stderr"
  cp "$ROOT/uid.expected" "$ROOT/uid.stdout"
  exact_uid_token_in_one_capture \
    "$ROOT/uid.expected" "$ROOT/uid.stdout" "$ROOT/uid.stderr" \
    || fail_test stdout_UID_token_rejected
  : >"$ROOT/uid.stdout"
  cp "$ROOT/uid.expected" "$ROOT/uid.stderr"
  exact_uid_token_in_one_capture \
    "$ROOT/uid.expected" "$ROOT/uid.stdout" "$ROOT/uid.stderr" \
    || fail_test stderr_UID_token_rejected
  cp "$ROOT/uid.expected" "$ROOT/uid.stdout"
  if exact_uid_token_in_one_capture \
    "$ROOT/uid.expected" "$ROOT/uid.stdout" "$ROOT/uid.stderr"
  then
    fail_test duplicated_UID_token_accepted
  fi
  printf '20' >"$ROOT/uid.stdout"
  printf '00\n' >"$ROOT/uid.stderr"
  if exact_uid_token_in_one_capture \
    "$ROOT/uid.expected" "$ROOT/uid.stdout" "$ROOT/uid.stderr"
  then
    fail_test split_UID_token_accepted
  fi

  mkdir -m 700 "$ROOT/private"
  printf 'dex fixture\n' >"$ROOT/private/rish.dex"
  chmod 400 "$ROOT/private/rish.dex"
  is_canonical_private_regular_file "$ROOT/private/rish.dex" 400 1024 \
    || fail_test private_file_rejected
  validate_private_parent "$ROOT/private/rish.dex" \
    || fail_test private_parent_rejected

  mkdir -m 700 "$ROOT/fixed-prefix" "$ROOT/fixed-prefix/bin"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' \
    >"$ROOT/fixed-prefix/bin/setsid"
  chmod 700 "$ROOT/fixed-prefix/bin/setsid"
  export PREFIX="$ROOT/fixed-prefix"
  validate_fixed_setsid_program || fail_test fixed_setsid_program_rejected
  mv "$ROOT/fixed-prefix/bin/setsid" "$ROOT/fixed-prefix/bin/setsid.real"
  ln -s "$ROOT/fixed-prefix/bin/setsid.real" "$ROOT/fixed-prefix/bin/setsid"
  if validate_fixed_setsid_program; then
    fail_test symlinked_setsid_program_accepted
  fi

  ln -s "$ROOT/private/rish.dex" "$ROOT/private/rish-link.dex"
  if is_canonical_private_regular_file "$ROOT/private/rish-link.dex" 400 1024; then
    fail_test dex_symlink_accepted
  fi
  rm -f "$ROOT/private/rish-link.dex"

  ln "$ROOT/private/rish.dex" "$ROOT/private/rish-hardlink.dex"
  if is_canonical_private_regular_file "$ROOT/private/rish.dex" 400 1024; then
    fail_test hardlinked_dex_accepted
  fi
  rm -f "$ROOT/private/rish-hardlink.dex"

  chmod 600 "$ROOT/private/rish.dex"
  if is_canonical_private_regular_file "$ROOT/private/rish.dex" 400 1024; then
    fail_test writable_dex_accepted
  fi
  chmod 400 "$ROOT/private/rish.dex"
  chmod 755 "$ROOT/private"
  if validate_private_parent "$ROOT/private/rish.dex"; then
    fail_test nonprivate_dex_parent_accepted
  fi
)

(
  # Verify the group/session accounting used to bound every candidate server.
  # shellcheck source=/dev/null
  source "$GATE"
  set +m
  setsid --wait -- bash -c 'printf "%s\n" "$$" >"$1"; exec sleep 30' \
    physical-gate-setsid "$ROOT/setsid.pid" &
  launcher_pid=$!
  for _ in $(seq 1 20); do
    [[ -s "$ROOT/setsid.pid" ]] && break
    sleep 0.05
  done
  if [[ ! -s "$ROOT/setsid.pid" ]]; then
    kill "$launcher_pid" >/dev/null 2>&1 || true
    wait "$launcher_pid" >/dev/null 2>&1 || true
    fail_test isolated_candidate_process_pid_not_observed
  fi
  observed_inner_pid="$(<"$ROOT/setsid.pid")"
  if [[ "$observed_inner_pid" != "$launcher_pid" ]]; then
    kill -- "-$launcher_pid" >/dev/null 2>&1 || kill "$launcher_pid" >/dev/null 2>&1 || true
    wait "$launcher_pid" >/dev/null 2>&1 || true
    fail_test isolated_candidate_launcher_inner_pid_mismatch
  fi
  SERVER_PID="$launcher_pid"
  SERVER_PGID=$SERVER_PID
  IFS=' ' read -r proc_self_pid _ < /proc/self/stat
  if [[ "$proc_self_pid" == "$BASHPID" && -r "/proc/$SERVER_PID/stat" ]]; then
    for _ in $(seq 1 20); do
      server_process_group_is_isolated && break
      sleep 0.05
    done
    server_process_group_is_isolated \
      || fail_test isolated_candidate_process_group_not_observed
    terminate_process_group_bounded "$SERVER_PID" "$SERVER_PGID" \
      || fail_test isolated_candidate_process_group_not_terminated
    if process_group_alive "$SERVER_PGID"; then
      fail_test isolated_candidate_process_group_survived
    fi
    wait "$launcher_pid" >/dev/null 2>&1 || true
  else
    # Some containerized CI shells expose namespace-local PIDs while /proc is
    # mounted from the parent namespace. BASHPID is the actual subshell PID;
    # comparing it with /proc/self avoids mistaking a colliding host PID for
    # this fixture. The physical gate itself still fails closed unless its
    # PID/group/session can be proved.
    kill -- "-$launcher_pid" >/dev/null 2>&1 || kill "$launcher_pid" >/dev/null 2>&1 || true
    wait "$launcher_pid" >/dev/null 2>&1 || true
  fi
)

(
  # Exercise the private raw-report and intermediate-evidence writers without
  # weakening the physical preconditions in main().
  # shellcheck source=/dev/null
  source "$GATE"
  mkdir -m 700 "$ROOT/evidence"
  ARTIFACT="$ROOT/evidence/termux-mcp-server"
  RISH_DEX="$ROOT/evidence/rish.dex"
  RAW_REPORT="$ROOT/evidence/raw.txt"
  OUTPUT="$ROOT/evidence/intermediate.json"
  printf 'candidate\n' >"$ARTIFACT"
  printf 'private dex\n' >"$RISH_DEX"
  chmod 700 "$ARTIFACT"
  chmod 400 "$RISH_DEX"
  ARTIFACT_SHA256="$(sha256sum "$ARTIFACT" | awk '{print $1}')"
  export ARTIFACT_SHA256
  RISH_DEX_SHA256="$(sha256sum "$RISH_DEX" | awk '{print $1}')"
  EXPECTED_COMMIT="$COMMIT"
  EXPECTED_VERSION=0.7.0
  POLICY_SHA256="$SHA_A"
  CARGO_LOCK_SHA256="$SHA_B"
  WORKFLOW_DEFINITION_SHA256="$SHA_C"
  WORKFLOW_RUN_ID=1001
  WORKFLOW_RUN_ATTEMPT=1
  CI_RUN_ID=1002
  SECURITY_RUN_ID=1003
  ANDROID_RUN_ID=1004
  CONTROLLER_CHALLENGE_SHA256="$SHA_A"
  API_LEVEL=35
  SECURITY_PATCH=2026-07-01
  DEVICE_PROFILE_COMMITMENT="$SHA_B"
  BUILD_FINGERPRINT_SHA256="$SHA_C"
  TERMUX_VERSION=0.118.3
  TERMUX_SIGNER_SHA256="$SHA_A"
  SHIZUKU_VERSION=13.5.4
  SHIZUKU_SIGNER_SHA256="$SHA_B"
  export ADB_SHELL_UID=2000
  export STARTED_AT=2026-07-31T12:00:00Z
  export COMPLETED_AT=2026-07-31T12:10:00Z

  write_raw_report
  write_evidence

  [[ "$(stat -c '%a' "$RAW_REPORT")" == 600 ]] \
    || fail_test raw_report_mode_invalid
  [[ "$(stat -c '%a' "$OUTPUT")" == 600 ]] \
    || fail_test evidence_mode_invalid
  [[ "$(sha256sum "$RAW_REPORT" | awk '{print $1}')" \
    == "$(jq -r '.rawReportSha256' "$OUTPUT")" ]] \
    || fail_test raw_report_digest_binding_invalid
  jq -e '
    .status == "pass"
    and .releaseEligible == false
    and .productionControlQualified == false
    and .qualificationClass == "physical_shizuku_rish_identity_development_v1"
    and .scope == "s2_5_uid_probe_only"
    and .backend == {
      name:"shizuku_rish",
      dexSha256:.backend.dexSha256,
      dexBytes:.backend.dexBytes,
      dexMode:"0400",
      dexParentMode:"0700",
      dexLinkCount:1,
      dexOwnerMatchesTermuxUid:true,
      dexCanonicalPrivatePath:true,
      principal:"android_shell",
      uid:2000,
      state:"verified_shell_uid",
      rootAccepted:false,
      arbitraryShell:false,
      mutationReady:false
    }
    and [.validation.scenarioResults[].id] == [
      "trusted_direct_rish_probe_pre_candidate",
      "runtime_disabled_tool_absent",
      "candidate_mcp_status_uid_2000",
      "extra_arguments_rejected",
      "unknown_shell_rejected",
      "dex_tamper_rejected",
      "dex_mode_rejected",
      "dex_symlink_rejected",
      "all_mutation_gates_disabled",
      "trusted_direct_rish_probe_post_candidate",
      "bounded_test_fixture_cleanup"
    ]
    and .validation.trustedDirectRishProbePreCandidate == true
    and .validation.trustedDirectRishProbePostCandidate == true
    and all(.validation.scenarioResults[0:10][];
      .execution == "physical" and .outcome == "pass")
    and .validation.scenarioResults[10] == {
      id:"bounded_test_fixture_cleanup",
      execution:"physical",
      outcome:"pending"
    }
    and .claimBoundary == {
      s3Attestation:false,
      typedReads:false,
      grantV2:false,
      deviceMutation:false,
      productionControl:false,
      sameUidPersistenceExcluded:false,
      continuousNetworkIsolation:false,
      adversarialNetworkIsolation:false
    }
    and .cleanup == {
      candidateProcessGroupStopped:true,
      portReleased:true,
      deviceFixtureStateRemoved:false,
      controllerTransportRemoved:false
    }
  ' "$OUTPUT" >/dev/null || fail_test intermediate_evidence_contract_invalid
  grep -Fxq 'bounded_test_fixture_cleanup=pending' "$RAW_REPORT" \
    || fail_test raw_report_cleanup_not_pending
  grep -Fxq 'trusted_direct_rish_probe_pre_candidate=pass' "$RAW_REPORT" \
    || fail_test raw_report_pre_candidate_probe_missing
  grep -Fxq 'trusted_direct_rish_probe_post_candidate=pass' "$RAW_REPORT" \
    || fail_test raw_report_post_candidate_probe_missing
  grep -Fxq 'candidate_mcp_status_uid_2000=pass' "$RAW_REPORT" \
    || fail_test raw_report_candidate_probe_missing
  if grep -Eq '/data/|Bearer[[:space:]]|MCP__|localhost|127\.0\.0\.1' "$OUTPUT"; then
    fail_test evidence_contains_private_runtime_material
  fi
)

(
  # The controller must independently bind every identity field instead of
  # trusting candidate-adjacent evidence or workflow outputs derived from it.
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  mkdir -m 700 "$ROOT/reconcile"
  export WORK_ROOT="$ROOT/reconcile"
  export EXPECTED_COMMIT="$COMMIT"
  export EXPECTED_VERSION=0.7.0
  export POLICY_SHA256="$SHA_A"
  export CARGO_LOCK_SHA256="$SHA_B"
  export WORKFLOW_DEFINITION_SHA256="$SHA_C"
  export WORKFLOW_RUN_ID=1001
  export WORKFLOW_RUN_ATTEMPT=1
  export CI_RUN_ID=1002
  export SECURITY_RUN_ID=1003
  export ANDROID_RUN_ID=1004
  export CONTROLLER_CHALLENGE_SHA256="$SHA_A"
  export API_LEVEL=35
  export SECURITY_PATCH=2026-07-01
  export DEVICE_PROFILE_COMMITMENT="$SHA_B"
  export BUILD_FINGERPRINT_SHA256="$SHA_C"
  export TERMUX_VERSION=0.118.3
  export TERMUX_SIGNER_SHA256="$SHA_A"
  export SHIZUKU_VERSION=13.5.4
  export SHIZUKU_SIGNER_SHA256="$SHA_B"
  RISH_DEX_SHA256="$(sha256sum "$ROOT/evidence/rish.dex" | awk '{print $1}')"
  RISH_DEX_BYTES="$(stat -c '%s' "$ROOT/evidence/rish.dex")"
  export RISH_DEX_SHA256 RISH_DEX_BYTES
  artifact_sha="$(sha256sum "$ROOT/evidence/termux-mcp-server" | awk '{print $1}')"
  artifact_bytes="$(stat -c '%s' "$ROOT/evidence/termux-mcp-server")"
  validate_intermediate_evidence \
    "$ROOT/evidence/intermediate.json" \
    "$ROOT/evidence/raw.txt" \
    "$artifact_sha" \
    "$artifact_bytes" \
    || fail_test trusted_controller_reconciliation_rejected_valid_evidence

  jq '.environment.termuxSignerSha256 =
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"' \
    "$ROOT/evidence/intermediate.json" >"$ROOT/reconcile/tampered.json"
  if validate_intermediate_evidence \
    "$ROOT/reconcile/tampered.json" \
    "$ROOT/evidence/raw.txt" \
    "$artifact_sha" \
    "$artifact_bytes"
  then
    fail_test trusted_controller_reconciliation_accepted_tampered_identity
  fi
)

printf 'Termux rish physical gate tests passed\n'
