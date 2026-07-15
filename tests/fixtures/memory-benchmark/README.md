# Memory-system behavioral benchmark fixture

Every system receives the facts in `evidence.md`, translated only as required by its public
ingestion interface. It then answers each prompt in `questions.tsv` under the same output budget.
Answer files are named `<case_id>.txt` and scored with:

```sh
scripts/score-memory-benchmark.sh PATH_TO_ANSWER_DIRECTORY
```

The deterministic marker score is only the first gate. A benchmark run must also record the exact
system version, ingestion commands, retrieval calls, output character count, client-reported input
and output tokens when available, tool calls, wall-clock latency, and any manual correction needed.
Raw retrieval is not scored as a final answer because a correct audit trail may legitimately
contain both current and superseded facts.
