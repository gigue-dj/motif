# Motif v0.0.1 — design notes and MVP plan

This document captures the architectural decisions for Motif v0.0.1 and the
scope of the alpha milestones. It is the single source of truth for the
"what and why" of the project. If something here disagrees with code,
the doc is wrong — open an issue.

## What Motif is

Motif is a **follower** graph store. A small embedded engine, packaged as a
WebAssembly module (`wasm32-unknown-unknown`) and embedded in mobile apps
and edge devices via their host language's wasm runtime — primarily Swift
and Rust. It holds a local property graph for query speed and offline
operation. **It is not the source of truth for any data it holds.**
Integrity, schema, and conflict resolution belong to an upstream
**controller** (SurrealDB today, a custom Nebula-class controller later).

## What Motif is not

- Not a server.
- Not authoritative.
- Not a long-lived store. The local file is a working cache; the controller
  can sunset any local state.
- Not a browser-facing database. v0.0.1 explicitly assumes a Swift or Rust
  host. Browsers, fetch-based hosts, and WASI runtimes are out of scope.

## Topology

```
  ┌───────────────────────────────────────────────────────┐
  │ host app  (Swift on iOS / Rust on edge / Android)     │
  │                                                       │
  │   ┌─────────────────────────┐    ┌──────────────────┐ │
  │   │ Motif (wasm)            │    │ object graph     │ │
  │   │   query / write surface │◄──►│ (host-owned)     │ │
  │   │   local single-file db  │    └──────────────────┘ │
  │   │   WAL                   │                         │
  │   │   ControllerClient      │                         │
  │   └────────────┬────────────┘                         │
  └────────────────┼──────────────────────────────────────┘
                   │  mutation log
                   ▼
            ┌────────────────────┐
            │ controller         │   v0.0.1: SurrealDB
            │   schema owner     │   later:  custom Nebula
            │   integrity        │
            │   conflict res     │
            └────────────────────┘
```

## Locked-in decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Implementation language | **Rust.** No team C++ expertise; the C++ Kuzu fork at `cpp-reference/` is a behavioural reference only. |
| 2 | WASM target | **`wasm32-unknown-unknown`.** No WASI, no browser host. Swift/Rust host apps embed a wasm runtime (e.g. `wasmtime`) and accept the perf tax. Native targets (e.g. `aarch64-apple-ios`) are deferred to v0.0.2+. |
| 3 | Controller | SurrealDB for v0.0.x. Custom Nebula-class controller later. |
| 4 | Auth | Per-user **and** per-device. Treat all devices as potentially compromised. JWT-style bearer for users; per-device key for device identity. (No transport in v0.0.1; identity surfaced as `user_id` + `device_id` in `motif.toml`.) |
| 5 | Conflict resolution | **Server-wins.** Local can apply optimistic mutations; controller can override or sunset them. Provisional/CRDT shadow layer is a v0.0.2 design item. |
| 6 | Local read miss | Return stale-with-flag. Async refresh. Never block on the controller. |
| 7 | Offline mode | First-class from day 0. Writes queue locally; reads work without connectivity. |
| 8 | Schema ownership | Controller owns schema and pushes to followers. Motif does not declare. v0.0.1 treats schema as opaque. Live push lands in v0.0.2. |
| 9 | Configuration | **TOML.** `motif.toml` lives next to the host app, parsed via `serde + toml`. See `motif.toml.example`. |
| 10 | Storage format | Greenfield Rust. **No on-disk compatibility** with the C++ baseline. |
| 11 | Query parser | **Hand-rolled recursive descent** for the Cypher subset in v0.0.1 (~500 LOC target). No `nom`/`pest`/`chumsky`. Reassess once exit criteria are met. |
| 12 | Async runtime | **Sync `motif-core`**, async at the edges if needed. No `tokio` in core. A `smol` or `async-std` reactor may land later if the controller transport demands it. |
| 13 | Encryption-at-rest | Out of scope for v0.0.1; storage layer must not preclude it. |
| 14 | Distribution | Internally usable only. Crates exist (`motif-core`, `motif-wasm`, `motif-cli`) with `publish = false`. Public crates.io publishing deferred. |
| 15 | License | **Dual MIT / Apache-2.0** — Rust crates.io convention. The cpp-reference subdirectory remains MIT (inherited from Kuzu). |
| 16 | Safety | `unsafe_code = "forbid"` at workspace level. Easier to defend on hostile devices. |
| 17 | Terminology | "Follower," not "slave/replica." |

## Milestone structure

| milestone | content | status |
|---|---|---|
| **alpha.1** | Fork upstream Kuzu, prune extensions/bindings/benchmarks, top-level rebrand, C++ sync skeleton in `src/sync/`. | ✅ shipped |
| **alpha.1.5** | Mechanical move of the C++ tree into `cpp-reference/` so the Rust crates can land at the top level. | ✅ shipped |
| **alpha.2** | Rust workspace: `crates/{motif-core, motif-wasm, motif-cli}`. TOML config (`MotifConfig`) via `serde`. Sync skeleton ported 1:1 from C++ (`ControllerClient` trait, `MutationLog`, `InMemoryControllerClient`). Rust CI (`fmt`, `clippy`, `test`, `wasm32-unknown-unknown` build). Dual licensing in place. **No engine yet.** | ✅ shipped |
| **alpha.3** | Minimal Rust storage: single-file append log + in-memory id→offset index. Node/edge insert + get-by-id only. ~1k LOC. | pending |
| **alpha.4** | Hand-rolled recursive-descent parser + direct interpreter for a tiny Cypher subset (`CREATE`, `MATCH (n) WHERE id(n)=$x RETURN n`, `MERGE`, `DELETE`). ~800 LOC. | pending |
| **alpha.5** | `wasm-bindgen` `Motif::open(&MotifConfig)` API, latency harness in `wasmtime`, WAL→`MutationLog` hook wired, `wasm-opt -Oz`. | pending |
| **v0.0.1** | Hits exit criteria below. Tag `cpp-reference/` for archival. | pending |

## Exit criteria for v0.0.1

1. `cargo build --release --target wasm32-unknown-unknown -p motif-wasm` produces a module loadable from a `wasmtime` host. **And** `cargo run -p motif-cli -- print-config motif.toml.example` round-trips cleanly.
2. Library API: `Motif::open(&MotifConfig)` returns a handle. `MotifConfig` is loaded via `serde + toml` from `motif.toml`.
3. TOML schema documented in `motif.toml.example`; `serde` rejects malformed configs with a useful error.
4. Query API: `motif.query("MATCH (n) RETURN n LIMIT 10")` returns rows.
5. p50 single-node lookup <50 ms in `wasmtime` on a representative mid-tier dev box (proxy for mobile).
6. WASM artifact <2 MB after `wasm-opt -Oz`. (Current alpha.2 stub: 358 KB unstripped — plenty of headroom.)
7. Mutation tee verified: every commit produces a `Mutation` in the registered `ControllerClient` (test spy).
8. No `unsafe` in `motif-core`. (Enforced by `#![forbid(unsafe_code)]` in alpha.2.)
9. TOML round-trip: `MotifConfig::from_toml_str(&cfg.to_toml_string())` yields the same value.

## Explicitly out of scope for v0.0.1

iOS/Android native targets (`aarch64-apple-ios`, `aarch64-linux-android`),
real network transport to SurrealDB, provisional-write shadow layer /
scoped CRDT, conflict resolution wire protocol, offline replay, JWT/auth
signing, encryption-at-rest, vector search, full-text search, multi-tenant,
schema migrations, browser/WASI hosts, public crates.io publication.

## Open questions for v0.0.2

1. **Provisional layer shape.** Server-wins says the controller can override.
   Do we model "provisional" as a per-mutation flag in `MutationLog`, or as
   a parallel shadow store that the WAL replayer reconciles? Trade-off is
   memory/space vs. read-path complexity.
2. **SurrealQL boundary.** Translate at the API layer (motif-wasm
   accepts SurrealQL strings, transpiles to Cypher before hitting the
   interpreter), or swap parsers entirely? Translation is cheaper for v0.0.x.
3. **Schema push channel.** Same transport as mutations, or a distinct
   control channel? Affects how schema versions interact with pending
   mutations.
4. **Device key provisioning.** Per-device keys assume a first-run pairing
   flow. Owned by host app, or by Motif?
5. **Sunset semantics.** Concretely: when the controller "overrides" a
   provisional change, does Motif emit a notification to the host app's
   object graph, and what does that look like ergonomically?
6. **Persisted MutationLog.** v0.0.1 keeps it in-process. Where does the
   persisted version live — alongside the WAL, or as a separate journal?
7. **Async transport.** Will the SurrealDB transport demand a runtime in
   `motif-core`, or can we keep it pure-sync with a thread per controller?

## Repository layout

```
motif/
  Cargo.toml                  # workspace root
  rust-toolchain.toml         # pins stable + wasm32-unknown-unknown
  motif.toml.example          # canonical config schema
  crates/
    motif-core/               # engine library, sync core, no_unsafe
    motif-wasm/               # wasm-bindgen surface (cdylib)
    motif-cli/                # dev/smoke CLI
  cpp-reference/              # frozen alpha.1 C++ tree, behavioural spec
  .github/workflows/rust.yml  # fmt + clippy + test + wasm32 build
  LICENSE-MIT, LICENSE-APACHE
  MOTIF.md, README.md
```

## Provenance

The frozen C++ reference under `cpp-reference/` is derived from
[Kuzu](https://github.com/kuzudb/kuzu/) (MIT). Upstream Kuzu has been
archived. The Rust crates are greenfield code informed by the C++ baseline
as a behavioural spec; the storage and query engines are designed for
mobile constraints and do not preserve any on-disk or namespace
compatibility.
