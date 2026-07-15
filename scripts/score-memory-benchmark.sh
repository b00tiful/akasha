#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 PATH_TO_ANSWER_DIRECTORY" >&2
  exit 2
fi

answers_dir=$1
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ground_truth="$repo_root/tests/fixtures/memory-benchmark/ground-truth.tsv"

if [[ ! -d "$answers_dir" ]]; then
  echo "benchmark answers directory does not exist: $answers_dir" >&2
  exit 2
fi

failures=0
printf 'case_id\tresult\tcharacters\n'
while IFS=$'\t' read -r case_id required_marker forbidden_marker; do
  if [[ "$case_id" == "case_id" ]]; then
    continue
  fi

  answer="$answers_dir/$case_id.txt"
  result=pass
  if [[ ! -f "$answer" ]] || ! grep -Fq -- "$required_marker" "$answer"; then
    result=fail
  elif [[ -n "$forbidden_marker" ]] && grep -Fq -- "$forbidden_marker" "$answer"; then
    result=fail
  fi

  if [[ -f "$answer" ]]; then
    characters=$(wc -m < "$answer" | tr -d '[:space:]')
  else
    characters=0
  fi
  printf '%s\t%s\t%s\n' "$case_id" "$result" "$characters"
  if [[ "$result" == "fail" ]]; then
    failures=$((failures + 1))
  fi
done < "$ground_truth"

if (( failures > 0 )); then
  echo "$failures benchmark case(s) failed" >&2
  exit 1
fi
