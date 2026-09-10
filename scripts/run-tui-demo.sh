#!/usr/bin/env bash
# Run the real terminal adapter against a disposable copy of the synthetic product vault.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
demo_dir="$(mktemp -d /tmp/akasha-tui-demo.XXXXXX)"
trap 'rm -rf -- "$demo_dir"' EXIT
cp -R -- "$project_dir/tests/fixtures/resolution/valid-root" "$demo_dir/root"
mkdir -- "$demo_dir/repository"
cargo run --quiet --manifest-path "$project_dir/Cargo.toml" -p akasha-cli -- \
  --root "$demo_dir/root" --project example tui "$@"
