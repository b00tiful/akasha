#!/usr/bin/env bash

set -euo pipefail

readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly FIXTURE_ROOT="${PROJECT_ROOT}/tests/fixtures/codex-onboarding"
readonly RUN_ROOT="${AKASHA_CODEX_SMOKE_ROOT:-${PROJECT_ROOT}/target/codex-onboarding-smoke}"
readonly DATA_ROOT="${RUN_ROOT}/root"
readonly REPOSITORY="${RUN_ROOT}/repository"
readonly PROJECT="codex-smoke"
readonly AKASHA_BIN="${PROJECT_ROOT}/target/debug/akasha"
readonly MCP_BIN="${PROJECT_ROOT}/target/debug/akasha-onboarding-mcp"

toml_string() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '"%s"' "${value}"
}

require_setup() {
    if [[ ! -x "${AKASHA_BIN}" || ! -x "${MCP_BIN}" || ! -f "${REPOSITORY}/.akasha.toml" ]]; then
        echo "Codex onboarding fixture is not initialized; run '$0 setup' first." >&2
        exit 2
    fi
}

codex_config() {
    local command_value
    local args_value
    command_value="$(toml_string "${MCP_BIN}")"
    args_value="[$(toml_string --root), $(toml_string "${DATA_ROOT}"), $(toml_string --project), $(toml_string "${PROJECT}"), $(toml_string --repo), $(toml_string "${REPOSITORY}")]"

    CODEX_MCP_COMMAND="mcp_servers.akasha_onboarding.command=${command_value}"
    CODEX_MCP_ARGS="mcp_servers.akasha_onboarding.args=${args_value}"
}

setup_fixture() {
    cargo build --offline --package akasha-cli --package akasha-mcp

    rm -rf "${RUN_ROOT}"
    mkdir -p "${DATA_ROOT}/Global" "${DATA_ROOT}/Projects" "${DATA_ROOT}/Inbox" "${REPOSITORY}"
    cp -R "${FIXTURE_ROOT}/root/." "${DATA_ROOT}/"
    cp -R "${FIXTURE_ROOT}/repository/." "${REPOSITORY}/"

    (
        cd "${REPOSITORY}"
        "${AKASHA_BIN}" --root "${DATA_ROOT}" init "${PROJECT}"
    )

    echo "Prepared Codex onboarding fixture:"
    echo "  repository: ${REPOSITORY}"
    echo "  Akasha root: ${DATA_ROOT}"
    echo "Run '$0 probe', then '$0 run'."
}

probe_codex() {
    require_setup
    codex_config
    codex mcp list \
        -c "${CODEX_MCP_COMMAND}" \
        -c "${CODEX_MCP_ARGS}" \
        -c 'mcp_servers.codegraph.enabled=false' \
        -c 'mcp_servers.obsidian.enabled=false' \
        -c 'mcp_servers.akasha_onboarding.required=true' \
        -c 'mcp_servers.akasha_onboarding.default_tools_approval_mode="writes"'
}

run_codex() {
    require_setup
    codex_config

    local prompt
    prompt="Onboard this synthetic repository into its already initialized Akasha project. Use the akasha_onboarding MCP workflow: call prepare, inspect the repository with read-only tools, and propose exactly one entity note at core.md that records only source-grounded facts about the library. Use the returned template and evidence contract, include links to the project index and roadmap, and propose exact index and roadmap projections. Call validate and preview. Show me the exact approval_summary, proposal_id, preview_id, and per-note actions, then call apply with those exact identifiers in the same Codex turn so the destructive-tool confirmation is the independent human approval gate. Do not apply if any identifier or preview detail changes, and do not modify this source repository."

    exec codex \
        --strict-config \
        --no-alt-screen \
        --cd "${REPOSITORY}" \
        --sandbox read-only \
        --ask-for-approval on-request \
        -c "${CODEX_MCP_COMMAND}" \
        -c "${CODEX_MCP_ARGS}" \
        -c 'mcp_servers.codegraph.enabled=false' \
        -c 'mcp_servers.obsidian.enabled=false' \
        -c 'mcp_servers.akasha_onboarding.required=true' \
        -c 'mcp_servers.akasha_onboarding.default_tools_approval_mode="writes"' \
        "${prompt}"
}

verify_result() {
    require_setup

    "${AKASHA_BIN}" --root "${DATA_ROOT}" --project "${PROJECT}" validate
    test -f "${DATA_ROOT}/Projects/${PROJECT}/entities/core.md"
    diff -ru --exclude=.akasha.toml "${FIXTURE_ROOT}/repository" "${REPOSITORY}"

    echo "Codex onboarding smoke result is valid and the source repository is unchanged."
}

case "${1:-}" in
    setup)
        setup_fixture
        ;;
    probe)
        probe_codex
        ;;
    run)
        run_codex
        ;;
    verify)
        verify_result
        ;;
    *)
        echo "Usage: $0 {setup|probe|run|verify}" >&2
        exit 2
        ;;
esac
