# Motif — design notes and roadmap

This document captures the architectural decisions for Motif and the scope
of in-flight and planned milestones. It is the single source of truth for
the "what and why" of the project. If something here disagrees with code,
the doc is wrong — open an issue.

> **Audience.** Motif is the OSS edge graph DB. Gigue Inc. has a private
> downstream — `phrase-db` — that wires Motif to its own controller stack;
> the OSS Motif must be usable by people who aren't Gigue. Decisions in
> this file are made through that lens: when a choice would lock Motif to
> Gigue's stack, the choice goes the other way and the Gigue-specific
> piece moves out of this repo.

## What Motif is

Motif is a **follower** graph store. A small embedded engine, packaged as
a WebAssembly module (`wasm32-unknown-unknown`) and embedded in mobile
apps and edge devices via their host language's wasm runtime — primarily
Swift and Rust. It holds a local property graph for query speed and
offline operation. **It is not the source of truth for any data it
holds.** Integrity, schema, and conflict resolution belong to an upstream
**controller** that motif reaches via a generic trait; concrete
controller transports live as optional separate bridge crates.

## What Motif is not

- Not a server.
- Not authoritative.
- Not a long-lived store. The local file is a working cache; the
  controller can sunset any local state.
- Not bundled with any specific controller transport. SurrealDB / Supabase
  / ClickHouse / Nebula / TigerGraph / etc. integrations are optional
  separate crates (or separate repos).
- Not a browser-facing database. Host apps are Swift / Rust / similar
  embedding wasm runtimes; browsers, fetch-based hosts, and WASI runtimes
  are out of scope.
- Not a UI / event / MCP framework. Host integrations for those concerns
  live in optional `motif-*-hub` crates above motif-core's surface.

## Topology

```
  ┌─────────────────────────────────────────────────────────────┐
  │ host app  (Swift on iOS / Rust on edge / Android / ...)     │
  │                                                             │
  │   ┌───────────────────────────┐    ┌──────────────────┐     │
  │   │ Motif (wasm)              │    │ object graph     │     │
  │   │   Cypher query surface    │◄──►│ (host-owned)     │     │
  │   │   local single-file db    │    └──────────────────┘     │
  │   │   persisted mutation log  │                             │
  │   │   foreshadow tracking     │                             │
  │   │   capability profile      │                             │
  │   └─────────────┬─────────────┘                             │
  └─────────────────┼───────────────────────────────────────────┘
                    │ Controller trait (abstract; no built-in transports)
                    ▼
            ┌──────────────────────────┐
            │ controller bridge        │   optional separate crate
            │   (motif-*-bridge)       │   e.g. motif-surreal-bridge
            └─────────────┬────────────┘
                          │
                          ▼
                ┌────────────────────┐
                │ controller DB      │   any controller (SurrealDB,
                │   schema owner     │   Supabase, ClickHouse, Nebula,
                │   integrity        │   TigerGraph, custom, ...)
                │   conflict res     │
                └────────────────────┘
```

Optional `motif-*-hub` crates can sit between Motif and the host app to
provide event-driven / MCP / RAG-routing behaviour. Motif itself stays
agnostic about how its data is consumed — the host queries metadata via
ordinary Cypher just like data.

## Locked-in decisions

| # | Topic | Decision |
|---|---|---|
| 1 | Implementation language | **Rust.** No team C++ expertise; the C++ Kuzu fork at `cpp-reference/` is a behavioural reference only. |
| 2 | WASM target | **`wasm32-unknown-unknown`.** No WASI, no browser host. Swift/Rust host apps embed a wasm runtime (e.g. `wasmtime`) and accept the perf tax. Native targets (e.g. `aarch64-apple-ios`) are deferred to v0.0.3+. |
| 3 | Controller transport | **Generic trait in motif-core. No bundled controller transports.** Concrete bridges (e.g. `motif-surreal-bridge`) are optional separate crates published independently. SurrealDB is Gigue's first concrete bridge target via the private `phrase-db` downstream; not bundled with Motif. |
| 4 | Auth | Per-user **and** per-device. Motif takes **opaque tokens** from the host and **opaque keys** from the controller and verifies they match — no JWT validation chain inside motif. Treat all devices as potentially compromised. Identity surfaced as `user_id` + `device_id` in `motif.toml`. |
| 5 | Conflict resolution | **Server-wins with foreshadow marking.** Local mutations carry a `foreshadow: bool` flag indicating "applied locally, not yet confirmed by controller." When the controller corrects or confirms, motif flips the flag or evicts the record. CRDT-style merge is out of scope. |
| 6 | Local read miss | Return stale-with-flag. Async refresh. Never block on the controller. |
| 7 | Offline mode | First-class from day 0. Writes queue locally to the persisted mutation log; reads work without connectivity. Foreshadow keeps the local store usable while disconnected. |
| 8 | Schema ownership | Controller owns schema and pushes to followers over the same channel as mutations (single-channel ordering avoids the schema-late race). Motif does not declare. |
| 9 | Configuration | **TOML.** `motif.toml` lives next to the host app, parsed via `serde + toml`. See `motif.toml.example`. |
| 10 | Storage format | Greenfield Rust. **No on-disk compatibility** with the C++ baseline; bleeding-edge until outside contributors arrive (v0.0.2 bumps the format version and rejects v0.0.1 stores rather than migrating). |
| 11 | Query language | **Cypher only.** Hand-rolled recursive-descent parser + interpreter. No SurrealQL or other-dialect translators inside Motif — those live in optional bridge / hub crates if anywhere. |
| 12 | Async runtime | **Sync `motif-core`.** No `tokio` in core. Native uses `std::thread` for the controller worker; wasm uses `wasm-bindgen-futures` with channels. Don't bottleneck on the microtask queue — gigue's mobile model implies high inbound transaction throughput. |
| 13 | Encryption-at-rest | Out of scope through v0.0.2; storage layer must not preclude it. |
| 14 | Distribution | `motif-core`, `motif-wasm`, `motif-cli` ship `publish = false` (internally usable only) through v0.0.2. Bridges and hubs may publish independently on crates.io with namespaced names (`motif-surreal-bridge`, `motif-supabase-bridge`, etc.). |
| 15 | License | **Dual MIT / Apache-2.0** at the top level. `cpp-reference/` retains its inherited Kuzu MIT in `cpp-reference/LICENSE-MIT-KUZU`. Motif copyright: Gigue Inc. and Motif Contributors. |
| 16 | Safety | `unsafe_code = "forbid"` at workspace level. Easier to defend on hostile devices. |
| 17 | Terminology | "Follower," not "slave/replica." |
| 18 | OSS posture / bridges architecture | Motif-core stays controller-agnostic. **Concrete controller transports never live inside `crates/motif-*`** — they ship as optional separate `motif-*-bridge` crates. Likely partition: `bridges/` (controller transports, e.g. `motif-surreal-bridge`) and `hubs/` (host-side event / MCP / RAG layers, e.g. `motif-hub`). Crates.io publishing is namespaced (`motif-...`). |
| 19 | Metadata as data | Motif's internal state — foreshadow flags, override history, capability profile, mutation log — is exposed as queryable Cypher tables, not a separate API. `MATCH (n) WHERE n._motif.foreshadow = true RETURN n` works. Keeps the surface uniform; no parallel state-inspection vocabulary. |
| 20 | Capability profile | Top-level `[capability]` section in `motif.toml` reports **deterministic facts** about the host — RAM MB, CPU cores, storage MB, arch, etc. Motif reports facts only. **No qualitative labels** ("medium", "sufficient") — host and controller decide what counts as enough. Auto-discovery is v0.0.3+. |
| 21 | Edge strategies | Both **edge-is-tiny** (Motif as cache + foreshadow buffer; controller does real compute) and **edge-is-free** (Motif executes locally based on capability profile; controller hands off opportunistically) are first-class. Configurable via `[capability]` and `[edge]`; no compile-time strategy lock-in. |
| 22 | Test profiles | Integration tests run against `default`, `potato` (artificially constrained — slow I/O, low RAM, capped throughput), and `hoverphone` (artificially over-fast, unusual timing). Schema-race and other timing-sensitive tests **must pass on all three.** "Going unexpectedly slow" and "going unexpectedly fast" are both real edge cases. |

## Milestone structure

| milestone | content | status |
|---|---|---|
| **alpha.1** | Fork upstream Kuzu, prune extensions/bindings/benchmarks, top-level rebrand, C++ sync skeleton in `src/sync/`. | ✅ shipped |
| **alpha.1.5** | Mechanical move of the C++ tree into `cpp-reference/` so the Rust crates can land at the top level. | ✅ shipped |
| **alpha.2** | Rust workspace: `crates/{motif-core, motif-wasm, motif-cli}`. TOML config (`MotifConfig`) via `serde`. Sync skeleton ported 1:1 from C++. Rust CI. Dual licensing. | ✅ shipped |
| **alpha.3** | Single-file append-log storage, in-memory `id → offset` index, replay-on-open recovery. 1,014 LOC, 23 tests. | ✅ shipped |
| **alpha.4** | Hand-rolled Cypher subset (`CREATE` / `MATCH`/`WHERE`/`RETURN`/`LIMIT` / `MERGE` / `MATCH ... DELETE`) — lexer + parser + interpreter. 41 tests. | ✅ shipped |
| **alpha.5** | WAL → MutationLog tee. `motif-wasm` real bindings (`Motif::open` + `query`). `motif-cli bench` latency harness. CI runs `wasm-opt -Oz` and enforces 2 MiB budget. 45 tests. | ✅ shipped |
| **v0.0.1** | Tag. Hits exit criteria below. Audit pass via PR #1 review; six findings deferred to v0.0.2 backlog (logged in `LIMITATIONS.md`). | ✅ shipped |
| **0.0.2-alpha.1** | **Foreshadow + persisted MutationLog.** Add `foreshadow: bool` to `Mutation`. New `Record::Mutation` variant + replay on open. Format-version bump to `2`; reject v0.0.1 stores with a clear error. State queries via metadata-as-data Cypher tables. | pending |
| **0.0.2-alpha.2** | **`Controller` trait + thread-per-controller.** Define the trait in motif-core; rename `InMemoryControllerClient` → `InMemoryController`; drop the `ControllerKind::InMemory` enum variant in favour of trait dispatch. Worker-thread-per-controller scaffolding: native uses `std::thread`, wasm uses `wasm-bindgen-futures` with channels. No real network yet — the in-memory controller runs on its own thread to validate the threading model. | pending |
| **0.0.2-alpha.3** | **Schema push + ordering invariants.** Schema-record kind on the same channel as mutations; controller pushes schema before motif processes mutations against new tables. Schema-arrives-late surfaces a clean `SchemaUnknown` error rather than a silent rejection cascade. | pending |
| **0.0.2-alpha.4** | **Reconnect + offline state machine.** Connection lifecycle for the controller worker: connect / disconnect / backoff. Replay persisted MutationLog after reconnect. `[capability]` and `[edge]` config knobs (foreshadow eagerness, retention, schema-cache policy). Test profiles (`default` / `potato` / `hoverphone`) stand up. | pending |
| **0.0.2-alpha.5** | **Audit pass.** Six PR #1 `[debt]` items + v0.0.2 debt closed or knowingly accepted. Bench harness extended for file-backed and thread-contention cases. | pending |
| **v0.0.2** | Tag. | pending |

## Exit criteria for v0.0.1

All satisfied at tag (see PR #1):

1. `cargo build --release --target wasm32-unknown-unknown -p motif-wasm` produces a loadable module; `cargo run -p motif-cli -- print-config motif.toml.example` round-trips.
2. `Motif::open` API works native + wasm.
3. TOML schema documented; `serde` rejects malformed configs.
4. `motif.query("MATCH (n) RETURN n LIMIT 10")` returns rows.
5. p50 single-node lookup <50 ms — actual: 1.22 µs native (40,000× margin).
6. WASM artifact <2 MB after `wasm-opt -Oz` — actual: 414 KB (19% of budget).
7. Mutation tee verified for every commit (4 tests in `tests/sync_hook.rs`).
8. No `unsafe` in `motif-core`.
9. TOML round-trip.

## Exit criteria for v0.0.2

1. **Foreshadow round-trip via in-memory controller.** Engine inserts → Mutation stamped `foreshadow=true` → in-memory controller "approves" → engine flips foreshadow to false. Verified by test spy.
2. **Persisted MutationLog survives crash.** Kill mid-write, restart, replay reconciles state — no lost writes, foreshadow flags preserved.
3. **Controller trait fully abstract.** No SurrealDB / network-DB / Surreal-anything in any `crates/motif-*`. `cargo tree -p motif-core` shows no network deps.
4. **Threading model works on wasm.** Wasm controller worker integrates via `wasm-bindgen-futures`; bench shows non-trivial throughput (target: ≥10× the v0.0.1 single-threaded baseline for mutation submission).
5. **Schema race surfaces a clean error**, not silent corruption.
6. **All 6 PR #1 `[debt]` findings closed** (fixed or knowingly accepted with explicit comment on the LIMITATIONS entry).
7. **WASM artifact still <2 MB** after `wasm-opt -Oz`.
8. **No `unsafe` in `motif-core`** preserved.
9. **`[capability]` documented** with `default` / `potato` / `hoverphone` example configs in `motif.toml.example`. All values are deterministic facts (numbers / enums) — no qualitative labels.
10. **Metadata-as-data demonstrated**: `MATCH (n) WHERE n._motif.foreshadow = true RETURN n` returns the right set; same for override history.
11. **Test profiles in CI**: schema-race and reconnect-replay tests pass on all three of `default`, `potato`, `hoverphone`.

## Out of scope for v0.0.2

`motif-surreal-bridge` (Gigue's job; lands separately). `motif-hub` / `motif-mcp` host-event layers (optional, separate). Any non-Cypher dialect support. Encryption-at-rest. Multi-tenant. Vector / FTS. Native targets (`aarch64-apple-ios`, `aarch64-linux-android`). Real JWT validation in motif (host's job). Cypher surface beyond v0.0.1 (no `ORDER BY`, no aggregates, no multi-pattern). Conflict-resolution beyond server-wins (no CRDTs, no OT). Public crates.io publish for `motif-core` / `motif-wasm` / `motif-cli`. Auto-discovery of `[capability]`. iOS/Android-specific bench profiles.

## Open questions for v0.0.3+

1. **`[capability]` auto-discovery.** Manual config in v0.0.2; v0.0.3 should pick a Rust-ecosystem story for runtime hardware probing. Likely solved-problem in a small crate; just need to pick one.
2. **Crates.io publication for `motif-core`.** When and under what semver promises? Ties to outside-contributor onboarding.
3. **Multi-tenant.** Multiple databases per process? How does `[capability]` work when multiple Motif instances share a host?
4. **Encryption-at-rest design.** Motif holds the key (insecure but simple) vs. host provides the key on every read (slow). Need a third option that works on hostile devices.
5. **Native targets.** `aarch64-apple-ios` / `aarch64-linux-android` build pipelines. `uniffi`? Direct `cdylib`? Affects whether the wasm runtime tax is ever optional.
6. **Bridge test parameterisation.** How do `default` / `potato` / `hoverphone` profiles get applied across multiple bridge crates without duplicating CI matrices?
7. **`bridges/` vs `hubs/` partition.** First bridge and first hub will tell us whether the directory split is worth keeping or whether everything wants to be `motif-*-ext/` or similar.

## Repository layout

```
motif/
  Cargo.toml                       # workspace root
  rust-toolchain.toml              # pins stable + wasm32-unknown-unknown
  motif.toml.example               # canonical config schema (with [capability])
  crates/
    motif-core/                    # engine library, sync core, no_unsafe
    motif-wasm/                    # wasm-bindgen surface (cdylib)
    motif-cli/                     # dev/smoke CLI + bench
  bridges/                         # optional controller-transport crates
                                   # (planned; first arrives post-v0.0.2)
  hubs/                            # optional host-side event/MCP/RAG layers
                                   # (planned; first arrives post-v0.0.2)
  cpp-reference/                   # frozen alpha.1 C++ tree, behavioural spec
  .github/workflows/rust.yml       # fmt + clippy + test + wasm32 build + size budget
  LICENSE-MIT, LICENSE-APACHE
  MOTIF.md, README.md, LIMITATIONS.md
```

## Provenance

The frozen C++ reference under `cpp-reference/` is derived from
[Kuzu](https://github.com/kuzudb/kuzu/) (MIT, Copyright (c) 2022-2025
Kùzu Inc.); see `cpp-reference/LICENSE-MIT-KUZU`. Upstream Kuzu has been
archived. The Rust crates are greenfield code (Copyright (c) 2026 Gigue
Inc. and Motif Contributors) informed by the C++ baseline as a behavioural
spec; the storage and query engines are designed for mobile constraints
and do not preserve any on-disk or namespace compatibility.
