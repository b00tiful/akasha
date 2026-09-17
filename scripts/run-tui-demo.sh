#!/usr/bin/env bash
# Run the real terminal adapter against a disposable copy of the synthetic product vault.
set -euo pipefail
# Dedicated demo windows can return to a real interactive shell after the TUI exits.
keep_open=false
if [[ "${1:-}" == --keep-open ]]; then
  keep_open=true
  shift
fi
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
demo_dir="$(mktemp -d /tmp/akasha-tui-demo.XXXXXX)"
trap 'rm -rf -- "$demo_dir"' EXIT
cp -R -- "$project_dir/tests/fixtures/resolution/valid-root" "$demo_dir/root"
cp -- "$project_dir/tests/fixtures/tui/"*.md "$demo_dir/root/Projects/example/templates/"
mkdir -- "$demo_dir/repository"
demo_status=0
cargo run --quiet --manifest-path "$project_dir/Cargo.toml" -p akasha-cli -- \
  --root "$demo_dir/root" --project example tui "$@" || demo_status=$?
if "$keep_open" && [[ -t 0 && -t 1 ]]; then
  rm -rf -- "$demo_dir"
  trap - EXIT
  cd -- "$project_dir"
  exec "${SHELL:-/bin/bash}" -i
fi
exit "$demo_status"
