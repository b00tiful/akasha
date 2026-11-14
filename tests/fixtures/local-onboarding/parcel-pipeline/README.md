# Parcel dispatch

Parcel is a synthetic synchronous dispatch library, not a running service.
It combines bounded unique pending IDs, stop-on-failure dispatch and a compact report.
Configuration accepts an optional decimal capacity. There is no persistence or retry timer.
Run `cargo test --offline` to verify queue boundaries, configuration and delivery ordering.
Implementation lives in src/config.rs, src/queue.rs, src/delivery.rs and src/report.rs.
tests/pipeline.rs checks their interaction. No deployment or benchmark has been performed.
