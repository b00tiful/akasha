# Memory-system behavioral benchmark fixture

Every system receives the facts in `evidence.md`, translated only as required by its public
ingestion interface. It then answers each prompt in `questions.tsv` under the same output budget.
The fixed first-run budget is 256 Unicode characters per answer, including the final newline.
Answer files are named `<case_id>.txt` and scored with:

```sh
scripts/score-memory-benchmark.sh PATH_TO_ANSWER_DIRECTORY
```

The deterministic marker score is only the first gate. A benchmark run must also record the exact
system version, ingestion commands, retrieval calls, output character count, client-reported input
and output tokens when available, tool calls, wall-clock latency, and any manual correction needed.
Raw retrieval is not scored as a final answer because a correct audit trail may legitimately
contain both current and superseded facts.

The Akasha adapter runs the direct-file and bounded-CLI lanes without modifying the source fixture:

```sh
scripts/run-akasha-memory-benchmark.sh OUTPUT_DIRECTORY ANSWERS_DIRECTORY
```

`ANSWERS_DIRECTORY` contains parallel `direct-read/` and `bounded-cli/` directories. Each run
preserves retrieval payloads, exact commands, validation output, deterministic scores, and a TSV
manifest with explicit `unavailable` values when the active client cannot report token or answer
latency telemetry. Output directories are create-only so a later run cannot overwrite evidence.
