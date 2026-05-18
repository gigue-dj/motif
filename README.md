# morceau-db

[![crates.io](https://img.shields.io/crates/v/morceau-core.svg)](https://crates.io/crates/morceau-core)
[![docs.rs](https://docs.rs/morceau-core/badge.svg)](https://docs.rs/morceau-core)
[![CI](https://github.com/morceau-db/morceau-db/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/morceau-db/morceau-db/actions/workflows/rust.yml)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.78-orange.svg)](./Cargo.toml)

**morceau-db** is a tiny, embedded follower graph store. It targets Swift and Rust application codebases on mobile and edge, with integrity outsourced to an upstream **controller** database via a generic trait — concrete controller bridges (SurrealDB, Supabase, ClickHouse, Nebula, ...) ship as optional separate crates.

> **Status:** **v0.0.4 shipping — first crates.io publish.** Through v0.0.4 morceau-db ships the engine + hand-rolled Cypher subset (edge MATCH, multi-hop, multi-pattern, ORDER BY, count / collect, DETACH DELETE), persisted mutation log, abstract `Controller` trait + thread-per-controller worker with reconnect, schema push with property-type validation, ID-namespace split + edge label / adjacency indexes (gigue B2B target: 1k–10k nodes, 100k–1M+ edges; `morceau bench --scale` p50 = 2.67 µs), `[capability]` auto-discovery (native + wasm), wasm host-storage shim, native cdylib targets (iOS / Android via `morceau-ffi`), tracing-instrumented hot paths, and cold-start measurement.
>
> **Pre-v0.1: fluid API; expect breakage.** The crates publish at v0.0.x with explicit "don't pin until v0.1.0" semantics. v0.1.0 freezes the public API and adds semver promises (see [`MORCEAU.md`](./MORCEAU.md) long-run strategy).
>
> **Project name.** The project shipped through v0.0.4 under the name "motif". v0.0.4-alpha.6 sed-renamed every workspace token, identifier, on-disk magic, Cypher metadata namespace, and doc reference to "morceau" / "morceau-db" before the first crates.io publish — too many "motif"-named databases and tools in the market (especially in biostatistics) created discoverability collision risk that compounds the closer the project gets to v0.1.0's API freeze.
>
> See [`MORCEAU.md`](./MORCEAU.md) for the design rationale, locked-in decisions, and milestone plan, and [`LIMITATIONS.md`](./LIMITATIONS.md) for the running ledger of known caveats.
>
> - **`cpp-reference/`** — alpha.1 C++ tree, scoped down from upstream Kuzu. Frozen reference; not the shipping artifact. Retains its original [Kuzu MIT license](./cpp-reference/LICENSE-MIT-KUZU). The frozen tree predates the morceau-db rename and is left under its original "motif" names; archived (separate tag / repo) on v0.1.0.
> - **Top level** — Rust workspace (`crates/morceau-core`, `crates/morceau-wasm`, `crates/morceau-cli`, `crates/morceau-ffi`).
> - **`bridges/` + `hubs/`** — planned partitions for optional controller-transport crates and host-side event/MCP layers; first arrivals post-v0.0.4.

## Architecture in one paragraph

A host application (Swift or Rust, on mobile or edge) embeds morceau-db as a WASM module via its language's wasm runtime. morceau-db holds a small local property graph for query speed and offline operation. Every committed mutation is teed to a `Controller` worker (one thread per controller; native uses `std::thread`, wasm uses `wasm-bindgen-futures`); the controller is the source of truth for schema, integrity, and conflict resolution. The controller is **server-wins**: local mutations carry a `foreshadow: bool` flag until the controller confirms or corrects them. morceau-db is a **follower**, not an authority — and it is **controller-agnostic**: SurrealDB / Supabase / ClickHouse / etc. integrations are optional separate `morceau-*-bridge` crates that morceau-core never imports.

## Design constraints

- **Tiny binary.** Target <2 MB after `wasm-opt -Oz` (v0.0.4 actual: 851 KB / 40.6% of budget).
- **Tiny on-disk footprint.** Single-file storage; no bundled extensions; mutation log lives in the same file.
- **Fast I/O.** <50 ms p50 single-key read on mid-tier mobile via the host's wasm runtime. v0.0.4 numbers (native, `morceau bench --scale`, 10k nodes + 100k edges, indexed edge `MATCH` with id-pushdown): p50 = 2.67 µs.
- **Offline-first.** Reads return stale-with-flag on miss. Writes queue locally to the persisted mutation log and sync when connectivity returns.
- **Hostile-device-aware.** Per-user + per-device auth. morceau-db compares opaque tokens; host owns the auth flow. Storage layer keeps encryption-at-rest as a future option.
- **Capability-aware.** A `[capability]` config section reports deterministic facts about the host (RAM, cores, storage, arch, GPU); the controller uses this to choose between `edge-is-tiny` (morceau-db as cache) and `edge-is-free` (morceau-db executes locally) strategies. morceau-db itself has no opinion on policy.

## Install (from crates.io)

```toml
[dependencies]
morceau-core = "0.0.4"  # the engine library
# Optional, target-specific:
morceau-wasm = "0.0.4"  # wasm32 bindings (host JS shim)
morceau-ffi  = "0.0.4"  # C ABI cdylib (iOS / Android consumers)
```

Pre-v0.1 stability disclaimer: minor bumps (v0.0.x → v0.0.x+1) may break the public API. Pin exact versions if your build matters. v0.1.0 is the API-freeze gate.

## Build from source

```bash
cargo build --release --target wasm32-unknown-unknown -p morceau-wasm
cargo run -p morceau-cli -- bench --nodes 1000 --lookups 5000
cargo run -p morceau-cli -- bench --scale --nodes 10000 --edges 100000
cargo run -p morceau-cli -- print-config morceau.toml.example
```

The frozen C++ reference can be built per `cpp-reference/CMakeLists.txt` for archaeological purposes.

## Provenance

The frozen C++ reference under `cpp-reference/` is derived from [Kuzu](https://github.com/kuzudb/kuzu/) (MIT, Copyright (c) 2022-2025 Kùzu Inc.). Upstream Kuzu has been archived. The Rust crates at the top level are greenfield code (Copyright (c) 2026 Gigue Inc. and morceau-db Contributors) informed by the C++ baseline as a behavioural spec.

## License

Dual-licensed under MIT or Apache-2.0 at your option — matches Rust crates.io convention.

- [`LICENSE-MIT`](./LICENSE-MIT) — morceau-db (Copyright (c) 2026 Gigue Inc. and morceau-db Contributors).
- [`LICENSE-APACHE`](./LICENSE-APACHE) — morceau-db Apache-2.0 grant.
- [`cpp-reference/LICENSE-MIT-KUZU`](./cpp-reference/LICENSE-MIT-KUZU) — original Kuzu MIT covering the code under `cpp-reference/`.
