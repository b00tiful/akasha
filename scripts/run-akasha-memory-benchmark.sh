#!/usr/bin/env bash

set -euo pipefail

export LC_ALL=C.UTF-8

readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly FIXTURE_ROOT="${PROJECT_ROOT}/tests/fixtures/memory-benchmark"
readonly EVIDENCE="${FIXTURE_ROOT}/evidence.md"
readonly QUESTIONS="${FIXTURE_ROOT}/questions.tsv"
readonly GROUND_TRUTH="${FIXTURE_ROOT}/ground-truth.tsv"
readonly SCORER="${PROJECT_ROOT}/scripts/score-memory-benchmark.sh"
readonly AKASHA_BIN="${PROJECT_ROOT}/target/debug/akasha"
readonly PROJECT="benchmark"
readonly ANSWER_BUDGET_CHARS=256

usage() {
    echo "Usage: $0 OUTPUT_DIRECTORY ANSWERS_DIRECTORY" >&2
    echo "ANSWERS_DIRECTORY must contain direct-read/ and bounded-cli/ answer files." >&2
    exit 2
}

if [[ $# -ne 2 ]]; then
    usage
fi

run_root=$1
answers_root=$2

if [[ -e "${run_root}" ]]; then
    echo "benchmark output already exists: ${run_root}" >&2
    exit 2
fi
if [[ ! -d "${answers_root}/direct-read" || ! -d "${answers_root}/bounded-cli" ]]; then
    echo "benchmark answers must contain direct-read/ and bounded-cli/: ${answers_root}" >&2
    exit 2
fi

mkdir -p "${run_root}"
run_root="$(cd "${run_root}" && pwd)"
answers_root="$(cd "${answers_root}" && pwd)"

readonly ANSWER_PRODUCER="${BENCHMARK_ANSWER_PRODUCER:-external-supplied}"
readonly CLIENT_NAME="${BENCHMARK_CLIENT_NAME:-unavailable}"
readonly CLIENT_VERSION="${BENCHMARK_CLIENT_VERSION:-unavailable}"
readonly CLIENT_INPUT_TOKENS="${BENCHMARK_CLIENT_INPUT_TOKENS:-unavailable}"
readonly CLIENT_OUTPUT_TOKENS="${BENCHMARK_CLIENT_OUTPUT_TOKENS:-unavailable}"
readonly CLIENT_TOOL_CALLS="${BENCHMARK_CLIENT_TOOL_CALLS:-unavailable}"
readonly ANSWER_WALL_NS="${BENCHMARK_ANSWER_WALL_NS:-unavailable}"
readonly MANUAL_CORRECTION="${BENCHMARK_MANUAL_CORRECTION:-none}"

require_tsv_value() {
    local name=$1
    local value=$2
    if [[ "${value}" == *$'\t'* || "${value}" == *$'\n'* || "${value}" == *$'\r'* ]]; then
        echo "${name} must be a single TSV-safe line" >&2
        exit 2
    fi
}

for metadata in \
    "ANSWER_PRODUCER:${ANSWER_PRODUCER}" \
    "CLIENT_NAME:${CLIENT_NAME}" \
    "CLIENT_VERSION:${CLIENT_VERSION}" \
    "CLIENT_INPUT_TOKENS:${CLIENT_INPUT_TOKENS}" \
    "CLIENT_OUTPUT_TOKENS:${CLIENT_OUTPUT_TOKENS}" \
    "CLIENT_TOOL_CALLS:${CLIENT_TOOL_CALLS}" \
    "ANSWER_WALL_NS:${ANSWER_WALL_NS}" \
    "MANUAL_CORRECTION:${MANUAL_CORRECTION}"; do
    require_tsv_value "${metadata%%:*}" "${metadata#*:}"
done

record_command() {
    local destination=$1
    shift
    printf '%q ' "$@" >> "${destination}"
    printf '\n' >> "${destination}"
}

record_command_in_directory() {
    local destination=$1
    local directory=$2
    shift 2
    printf '(cd %q && ' "${directory}" >> "${destination}"
    printf '%q ' "$@" >> "${destination}"
    printf ')\n' >> "${destination}"
}

answer_metrics() {
    local lane=$1
    local lane_dir="${run_root}/${lane}"
    local count=0
    local total=0

    mkdir -p "${lane_dir}/answers"
    cp -R "${answers_root}/${lane}/." "${lane_dir}/answers/"

    while IFS=$'\t' read -r case_id _question; do
        if [[ "${case_id}" == "case_id" ]]; then
            continue
        fi
        local answer="${lane_dir}/answers/${case_id}.txt"
        if [[ ! -f "${answer}" ]]; then
            echo "missing ${lane} answer: ${answer}" >&2
            exit 2
        fi
        local characters
        characters=$(wc -m < "${answer}" | tr -d '[:space:]')
        if (( characters > ANSWER_BUDGET_CHARS )); then
            echo "${lane} answer ${case_id} exceeds ${ANSWER_BUDGET_CHARS} characters" >&2
            exit 2
        fi
        count=$((count + 1))
        total=$((total + characters))
    done < "${QUESTIONS}"

    printf '%s\t%s\n' "${count}" "${total}"
}

write_manifest() {
    local lane=$1
    local retrieval_command=$2
    local retrieval_wall_ns=$3
    local score_status=$4
    local answer_count=$5
    local answer_chars=$6
    local ingestion_commands=$7
    local validation_commands=$8
    local lane_dir="${run_root}/${lane}"
    local payload_chars
    local payload_bytes
    local payload_sha256
    payload_chars=$(wc -m < "${lane_dir}/retrieval.md" | tr -d '[:space:]')
    payload_bytes=$(wc -c < "${lane_dir}/retrieval.md" | tr -d '[:space:]')
    payload_sha256=$(sha256sum "${lane_dir}/retrieval.md" | cut -d ' ' -f 1)

    {
        printf 'field\tvalue\n'
        printf 'schema_version\t1\n'
        printf 'system\takasha\n'
        printf 'system_version\t%s\n' "$("${AKASHA_BIN}" --version)"
        printf 'lane\t%s\n' "${lane}"
        printf 'status\t%s\n' "${score_status}"
        printf 'corpus_sha256\t%s\n' "$(sha256sum "${EVIDENCE}" | cut -d ' ' -f 1)"
        printf 'questions_sha256\t%s\n' "$(sha256sum "${QUESTIONS}" | cut -d ' ' -f 1)"
        printf 'ground_truth_sha256\t%s\n' "$(sha256sum "${GROUND_TRUTH}" | cut -d ' ' -f 1)"
        printf 'adapter_sha256\t%s\n' "$(sha256sum "${BASH_SOURCE[0]}" | cut -d ' ' -f 1)"
        printf 'scorer_sha256\t%s\n' "$(sha256sum "${SCORER}" | cut -d ' ' -f 1)"
        printf 'answer_budget_characters\t%s\n' "${ANSWER_BUDGET_CHARS}"
        printf 'ingestion_commands\t%s\n' "${ingestion_commands}"
        printf 'validation_commands\t%s\n' "${validation_commands}"
        printf 'retrieval_command\t%s\n' "${retrieval_command}"
        printf 'retrieval_payload\tretrieval.md\n'
        printf 'retrieval_payload_sha256\t%s\n' "${payload_sha256}"
        printf 'retrieval_payload_characters\t%s\n' "${payload_chars}"
        printf 'retrieval_payload_bytes\t%s\n' "${payload_bytes}"
        printf 'retrieval_wall_time_ns\t%s\n' "${retrieval_wall_ns}"
        printf 'retrieval_tool_calls\t1\n'
        printf 'answer_count\t%s\n' "${answer_count}"
        printf 'answer_characters\t%s\n' "${answer_chars}"
        printf 'answer_producer\t%s\n' "${ANSWER_PRODUCER}"
        printf 'client_name\t%s\n' "${CLIENT_NAME}"
        printf 'client_version\t%s\n' "${CLIENT_VERSION}"
        printf 'client_input_tokens\t%s\n' "${CLIENT_INPUT_TOKENS}"
        printf 'client_output_tokens\t%s\n' "${CLIENT_OUTPUT_TOKENS}"
        printf 'client_tool_calls\t%s\n' "${CLIENT_TOOL_CALLS}"
        printf 'answer_wall_time_ns\t%s\n' "${ANSWER_WALL_NS}"
        printf 'manual_correction\t%s\n' "${MANUAL_CORRECTION}"
        printf 'commands\tcommands.txt\n'
        printf 'scores\tscore.tsv\n'
    } > "${lane_dir}/manifest.tsv"
}

score_lane() {
    local lane=$1
    local lane_dir="${run_root}/${lane}"
    if "${SCORER}" "${lane_dir}/answers" > "${lane_dir}/score.tsv" 2> "${lane_dir}/score.stderr"; then
        printf 'pass'
    else
        printf 'fail'
    fi
}

cargo build --offline --quiet --package akasha-cli

mkdir -p "${run_root}/direct-read"
readonly direct_commands="${run_root}/direct-read/commands.txt"
record_command "${direct_commands}" sed -n 1,260p "${EVIDENCE}"
direct_start=$(date +%s%N)
sed -n '1,260p' "${EVIDENCE}" > "${run_root}/direct-read/retrieval.md"
direct_end=$(date +%s%N)
IFS=$'\t' read -r direct_answer_count direct_answer_chars < <(answer_metrics direct-read)
direct_status=$(score_lane direct-read)
write_manifest \
    direct-read \
    "sed -n '1,260p' evidence.md" \
    "$((direct_end - direct_start))" \
    "${direct_status}" \
    "${direct_answer_count}" \
    "${direct_answer_chars}" \
    0 \
    0

readonly bounded_dir="${run_root}/bounded-cli"
readonly data_root="${bounded_dir}/root"
readonly repository="${bounded_dir}/repository"
readonly projection_input="${bounded_dir}/roadmap-input.md"
readonly bounded_commands="${bounded_dir}/commands.txt"
mkdir -p \
    "${data_root}/Meta" \
    "${data_root}/templates" \
    "${data_root}/Global" \
    "${data_root}/Projects" \
    "${data_root}/Inbox" \
    "${repository}"
cp -R "${FIXTURE_ROOT}/akasha-adapter/root/." "${data_root}/"

printf '# Benchmark roadmap\n\n- [[Projects/%s/records/tasks/benchmark-evidence|Benchmark evidence]]\n' \
    "${PROJECT}" > "${projection_input}"

record_command_in_directory "${bounded_commands}" "${repository}" \
    "${AKASHA_BIN}" --root "${data_root}" init "${PROJECT}"
(
    cd "${repository}"
    "${AKASHA_BIN}" --root "${data_root}" init "${PROJECT}"
) > "${bounded_dir}/initialization.txt"

evidence_source=$(< "${EVIDENCE}")
record_command "${bounded_commands}" \
    "${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" \
    create-note task benchmark-evidence.md \
    --projection "${projection_input}" \
    --field status=open \
    --field created=2026-07-15 \
    --field updated=2026-07-15 \
    --field 'title=Memory benchmark evidence' \
    --field "body=${evidence_source}"
"${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" \
    create-note task benchmark-evidence.md \
    --projection "${projection_input}" \
    --field status=open \
    --field created=2026-07-15 \
    --field updated=2026-07-15 \
    --field 'title=Memory benchmark evidence' \
    --field "body=${evidence_source}" > "${bounded_dir}/ingestion.txt"

record_command "${bounded_commands}" \
    "${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" validate
"${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" validate \
    > "${bounded_dir}/validation.txt"

record_command "${bounded_commands}" \
    "${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" context
bounded_start=$(date +%s%N)
"${AKASHA_BIN}" --root "${data_root}" --project "${PROJECT}" context \
    > "${bounded_dir}/retrieval.md"
bounded_end=$(date +%s%N)
IFS=$'\t' read -r bounded_answer_count bounded_answer_chars < <(answer_metrics bounded-cli)
bounded_status=$(score_lane bounded-cli)
write_manifest \
    bounded-cli \
    'akasha --root RUN_ROOT/root --project benchmark context' \
    "$((bounded_end - bounded_start))" \
    "${bounded_status}" \
    "${bounded_answer_count}" \
    "${bounded_answer_chars}" \
    2 \
    1

printf 'lane\tstatus\tretrieval_characters\tretrieval_bytes\tretrieval_wall_time_ns\tanswer_characters\n' \
    > "${run_root}/summary.tsv"
for lane in direct-read bounded-cli; do
    manifest="${run_root}/${lane}/manifest.tsv"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${lane}" \
        "$(awk -F '\t' '$1 == "status" { print $2 }' "${manifest}")" \
        "$(awk -F '\t' '$1 == "retrieval_payload_characters" { print $2 }' "${manifest}")" \
        "$(awk -F '\t' '$1 == "retrieval_payload_bytes" { print $2 }' "${manifest}")" \
        "$(awk -F '\t' '$1 == "retrieval_wall_time_ns" { print $2 }' "${manifest}")" \
        "$(awk -F '\t' '$1 == "answer_characters" { print $2 }' "${manifest}")" \
        >> "${run_root}/summary.tsv"
done

echo "Akasha memory benchmark run: ${run_root}"
sed -n '1,3p' "${run_root}/summary.tsv"

if [[ "${direct_status}" != "pass" || "${bounded_status}" != "pass" ]]; then
    exit 1
fi
