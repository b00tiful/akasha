# Archive reads

Each library projection validates every registered project and the Global entity collection.
Project validation checks canonical schemas, links, immutable events and maintained projection
fingerprints. Search builds this fresh projection before matching canonical source text.

The shared wikilink scanner retains its current line-end byte offset until the cursor reaches
or crosses it, including after multiline comments and inline-code jumps. Long paragraphs avoid
repeated searches through the remaining line. Targets, aliases, fragments, embeds, ignored
regions, byte ranges and malformed-link errors retain their existing semantics.

Inline-code matching also shares one forward lookahead cursor. Runs of other backtick lengths
are queued until a later request needs them; obsolete positions are discarded. Matching still
selects the nearest raw equal-length run, including inside comments, fences or link labels.
Ordinary same-length spans without intervening different-length runs allocate no queue.

For N body bytes, R examined raw backtick runs and D encountered lengths, this lookahead takes
O(N + R log(D + 1)) work and O(R + D) auxiliary storage. Queued positions live only for one parse.
This bounds this lookahead path; it does not establish a whole-parser or archive resource bound.

All original filesystem reads, fresh project/global validation, source integrity checks and
journal policies remain. No metadata observer, additional source buffer, persisted cache,
public schema or dependency was added. Concurrent reads still do not form an atomic snapshot.
Whole-note/archive resource limits remain separate.

To measure the release build with disposable 100, 1,000 and 10,000-note archives:

```sh
cargo test --offline --release -p akasha-core --test library synthetic_archive_runtime \
  -- --ignored --nocapture
```

The fixture splits added notes equally between a project and Global. Each JSON row reports
full projection, search and background-core microseconds, exact counts and source preservation.
Additional raw-read, frontmatter-parse and SHA-256 timers each process every source once after
the operation timers. Those phases overlap work inside the complete operations; do not add
them to operation times. Setup warms the filesystem. Measurements describe the fixture,
machine and build, with no universal latency guarantee.

To compare the current scanner with frozen former control flow on long Unicode paragraphs:

```sh
cargo test --offline --release -p akasha-core --lib synthetic_wikilink_runtime \
  -- --ignored --nocapture
```

Each row reports source bytes, former/current nanoseconds per parse and exact occurrence parity.
The current value averages twenty parses; the former value is one parse. Neither benchmark is
a normal test timing threshold. See also [YAML limits](resource-limits.md).

To compare inline-code scanning with the frozen parser after the line-boundary fix:

```sh
cargo test --offline --release -p akasha-core --lib synthetic_backtick_runtime \
  -- --ignored --nocapture
```

This generates increasing distinct unmatched backtick lengths and compares every occurrence
and byte range. JSON rows report run counts, bytes and former/current nanoseconds. The former
value is one parse; the current value averages twenty checked parses. This is an adversarial
case, not a representative Markdown workload or a timing threshold.
