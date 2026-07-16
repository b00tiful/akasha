#!/usr/bin/env bash

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${PROJECT_ROOT}/.env"

if [[ ! -r "${ENV_FILE}" ]]; then
    echo "Missing readable ${ENV_FILE}; copy .env.example to .env and set OBSIDIAN_API_KEY." >&2
    exit 1
fi

set -a
# shellcheck source=/dev/null
source "${ENV_FILE}"
set +a

: "${OBSIDIAN_API_KEY:?OBSIDIAN_API_KEY must be set in ${ENV_FILE}}"

export OBSIDIAN_BASE_URL="${OBSIDIAN_BASE_URL:-http://127.0.0.1:${OBSIDIAN_PORT:-27123}}"
export MCP_TRANSPORT_TYPE="stdio"
export OBSIDIAN_READ_ONLY="true"
export OBSIDIAN_ENABLE_COMMANDS="false"

exec obsidian-mcp-server
