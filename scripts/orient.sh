#!/usr/bin/env bash
# Print the bounded workbench orientation packet in one call (AGENTS.md "Mandatory orientation").
# Index + active tasks + the last two log entries of the latest dated session.
set -euo pipefail
cd "$(dirname "$0")/.."
W=akasha
BUDGET=30000
ENTRIES=${ORIENT_SESSION_ENTRIES:-2}

latest=$(find "$W/Sessions" -maxdepth 1 -name '20[0-9][0-9]-[0-9][0-9]-[0-9][0-9].md' -printf '%f\n' | sort | tail -n 1)
[ -n "$latest" ] || { echo "orient: no dated session note in $W/Sessions" >&2; exit 1; }

session_tail() {
  awk -v n="$ENTRIES" '/^## [0-9][0-9]:[0-9][0-9] /{starts[++c]=NR} {lines[NR]=$0}
    END{s=(c>n)?starts[c-n+1]:(c?starts[1]:1); for(i=s;i<=NR;i++) print lines[i]}' "$W/Sessions/$latest"
}

packet=$(
  for f in "$W/00_INDEX.md" "$W/Tasks/active.md"; do printf '===== %s =====\n' "$f"; cat "$f"; echo; done
  printf '===== %s (last %s log entries; open the full note only when continuing that thread) =====\n' \
    "$W/Sessions/$latest" "$ENTRIES"
  session_tail
)
printf '%s\n' "$packet"
size=$(printf '%s' "$packet" | wc -m)
printf '===== orientation packet: %s characters (budget %s) =====\n' "$size" "$BUDGET"
if [ "$size" -gt "$BUDGET" ]; then
  echo "orient: packet exceeds ${BUDGET} characters; compact 00_INDEX.md/Tasks/active.md (see AGENTS.md budgets)" >&2
fi
