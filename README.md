# Motif

**Motif** is a tiny, embedded follower graph store. It targets Swift and Rust application codebases on mobile and edge, with integrity outsourced to an upstream controller database (SurrealDB today, a custom Nebula-class controller later).

> **Status:** **v0.0.1 shipped.** Engine, query layer, sync skeleton, and wasm bindings all green. SurrealDB controller transport, host-provided wasm storage, and provisional-write semantics arrive in v0.0.2. See [`MOTIF.md`](./MOTIF.md) for the design rationale and milestone plan, and [`LIMITATIONS.md`](./LIMITATIONS.md) for the running ledger of known caveats.
>
> - **`cpp-reference/`** — alpha.1 C++ tree, scoped down from upstream Kuzu. Frozen reference; not the shipping artifact. Retains its original [Kuzu MIT license](./cpp-reference/LICENSE-MIT-KUZU). Will be archived (separate tag / repo) post-v0.0.1.
> - **Top level** — Rust workspace (`crates/motif-core`, `crates/motif-wasm`, `crates/motif-cli`).

## Architecture in one paragraph

A host application (Swift or Rust, on mobile or edge) embeds Motif as a WASM module via its language's wasm runtime. Motif holds a small local property graph for query speed and offline operation. Every committed mutation is teed to a `ControllerClient` queue; the controller is the source of truth for schema, integrity, and conflict resolution. The controller is **server-wins**: it can override or sunset any local change. Motif is a **follower**, not an authority.

## Design constraints

- **Tiny binary.** Target <2 MB after `wasm-opt -Oz`.
- **Tiny on-disk footprint.** Single-file storage, no extensions.
- **Fast I/O.** <50 ms p50 single-key read on mid-tier mobile (via the host's wasm runtime).
- **Offline-first.** Reads return stale-with-flag on miss. Writes queue locally and sync when connectivity returns.
- **Hostile-device-aware.** Per-user + per-device auth. Storage layer keeps encryption-at-rest as a future option.

## Build

Rust workspace and `wasm32-unknown-unknown` build instructions land in alpha.2. The frozen C++ reference can be built per `cpp-reference/CMakeLists.txt` for archaeological purposes.

## Provenance

The frozen C++ reference under `cpp-reference/` is derived from [Kuzu](https://github.com/kuzudb/kuzu/) (MIT, Copyright (c) 2022-2025 Kùzu Inc.). Upstream Kuzu has been archived. The Rust crates at the top level are greenfield code informed by the C++ baseline as a behavioural spec.

## License

Dual-licensed under MIT or Apache-2.0 at your option — matches Rust crates.io convention.

- [`LICENSE-MIT`](./LICENSE-MIT) — Motif (Copyright (c) 2026 gigue-dj and Motif Contributors).
- [`LICENSE-APACHE`](./LICENSE-APACHE) — Motif Apache-2.0 grant.
- [`cpp-reference/LICENSE-MIT-KUZU`](./cpp-reference/LICENSE-MIT-KUZU) — original Kuzu MIT covering the code under `cpp-reference/`.
