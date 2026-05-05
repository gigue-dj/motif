# Motif

**Motif** is a tiny, embedded follower graph store. It targets Swift and Rust application codebases on mobile and edge, with integrity outsourced to an upstream **controller** database via a generic trait — concrete controller bridges (SurrealDB, Supabase, ClickHouse, Nebula, ...) ship as optional separate crates.

> **Status:** **v0.0.1 shipped, v0.0.2 in flight.** v0.0.1 delivered the engine, hand-rolled Cypher subset, sync skeleton, and wasm bindings. v0.0.2 lands persisted mutation log + foreshadow tracking, the abstract `Controller` trait + thread-per-controller model, schema push, reconnect / offline state machine, and a `[capability]` profile that lets the controller decide between `edge-is-tiny` and `edge-is-free` strategies. See [`MOTIF.md`](./MOTIF.md) for the design rationale, locked-in decisions, and milestone plan, and [`LIMITATIONS.md`](./LIMITATIONS.md) for the running ledger of known caveats.
>
> - **`cpp-reference/`** — alpha.1 C++ tree, scoped down from upstream Kuzu. Frozen reference; not the shipping artifact. Retains its original [Kuzu MIT license](./cpp-reference/LICENSE-MIT-KUZU). Will be archived (separate tag / repo) post-v0.0.2.
> - **Top level** — Rust workspace (`crates/motif-core`, `crates/motif-wasm`, `crates/motif-cli`).
> - **`bridges/` + `hubs/`** — planned partitions for optional controller-transport crates and host-side event/MCP layers; first arrivals post-v0.0.2.

## Architecture in one paragraph

A host application (Swift or Rust, on mobile or edge) embeds Motif as a WASM module via its language's wasm runtime. Motif holds a small local property graph for query speed and offline operation. Every committed mutation is teed to a `Controller` worker (one thread per controller; native uses `std::thread`, wasm uses `wasm-bindgen-futures`); the controller is the source of truth for schema, integrity, and conflict resolution. The controller is **server-wins**: local mutations carry a `foreshadow: bool` flag until the controller confirms or corrects them. Motif is a **follower**, not an authority — and it is **controller-agnostic**: SurrealDB / Supabase / ClickHouse / etc. integrations are optional separate `motif-*-bridge` crates that motif-core never imports.

## Design constraints

- **Tiny binary.** Target <2 MB after `wasm-opt -Oz` (v0.0.1 actual: 414 KB).
- **Tiny on-disk footprint.** Single-file storage; no bundled extensions; mutation log lives in the same file from v0.0.2.
- **Fast I/O.** <50 ms p50 single-key read on mid-tier mobile via the host's wasm runtime (v0.0.1 actual: 1.22 µs native).
- **Offline-first.** Reads return stale-with-flag on miss. Writes queue locally to the persisted mutation log and sync when connectivity returns.
- **Hostile-device-aware.** Per-user + per-device auth. Motif compares opaque tokens; host owns the auth flow. Storage layer keeps encryption-at-rest as a future option.
- **Capability-aware.** A `[capability]` config section reports deterministic facts about the host (RAM, cores, storage, arch, GPU); the controller uses this to choose between `edge-is-tiny` (motif as cache) and `edge-is-free` (motif executes locally) strategies. Motif itself has no opinion on policy.

## Build

```bash
cargo build --release --target wasm32-unknown-unknown -p motif-wasm
cargo run -p motif-cli -- bench --nodes 1000 --lookups 5000
cargo run -p motif-cli -- print-config motif.toml.example
```

The frozen C++ reference can be built per `cpp-reference/CMakeLists.txt` for archaeological purposes.

## Provenance

The frozen C++ reference under `cpp-reference/` is derived from [Kuzu](https://github.com/kuzudb/kuzu/) (MIT, Copyright (c) 2022-2025 Kùzu Inc.). Upstream Kuzu has been archived. The Rust crates at the top level are greenfield code (Copyright (c) 2026 Gigue Inc. and Motif Contributors) informed by the C++ baseline as a behavioural spec.

## License

Dual-licensed under MIT or Apache-2.0 at your option — matches Rust crates.io convention.

- [`LICENSE-MIT`](./LICENSE-MIT) — Motif (Copyright (c) 2026 Gigue Inc. and Motif Contributors).
- [`LICENSE-APACHE`](./LICENSE-APACHE) — Motif Apache-2.0 grant.
- [`cpp-reference/LICENSE-MIT-KUZU`](./cpp-reference/LICENSE-MIT-KUZU) — original Kuzu MIT covering the code under `cpp-reference/`.
