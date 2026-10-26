# Terminal reader runtime

Akasha retains one reader layout for the current source, reader width, presentation mode and
palette. Scrolling reuses formatted logical lines and a wrapped-line index, then asks Ratatui
to render the visible range. Source changes, resizing, switching between literal reports and
Markdown, and palette changes rebuild that layout. No document cache is written to disk.

Source dirty checks and layout reuse compare exact editor lines without creating another full
source string. LF/CRLF, trailing newlines, undo, checked saves and stale-source refusal retain
their existing behavior.

Prompt decoration runs at up to 20 Hz on the dashboard or while the command prompt is focused.
Reader/editor focus, terminal focus loss and reduced motion pause it. The independent
[five-second external-change check](terminal-refresh.md) keeps its existing cadence and safeguards.
Actual terminal dimensions are also checked by the input loop so paused views resize and rebuild
mouse targets even when the terminal does not deliver a resize event.

The initial layout still processes the whole document. A single very long logical line still
requires reflow, and checking source equality still scans its bytes. These changes do not provide
a full-archive loading, search or background-validation latency guarantee. The shared core's
[YAML resource limits](resource-limits.md) separately bound metadata parsing and registry reads;
that guide also provides a reproducible full-archive measurement.

For a reproducible synthetic debug-build comparison of 100, 1,000 and 10,000 Unicode lines:

```sh
cargo test --offline -p akasha-cli --bin akasha \
  tui::reader::tests::synthetic_reader_runtime -- --ignored --nocapture
```

Each JSON row reports cold layout microseconds and total microseconds for 30 cached/reference
scroll renders. The reference uses whole-document Ratatui counting and rendering; its formatting
is already prepared, so it understates the previous reader's work. Timings describe the machine
and build being measured; exact buffer parity is tested separately.
