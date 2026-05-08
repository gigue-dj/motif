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
| **0.0.2-alpha.1** | **Foreshadow + persisted MutationLog.** `Mutation` gains `foreshadow: bool` and an inner `MutationOp` enum that subsumes the old separate `Record` and `MutationKind` types — closing PR #1 audit findings 1 (Rel/Edge naming) and 2 (`wal_payload` asymmetry) in the redesign. The on-disk log IS the persisted MutationLog: every record is a bincoded `Mutation`. Format-version bump to `2` rejects v0.0.1 stores. `Engine::is_foreshadow(id)` + Cypher `n._motif.X` metadata namespace dispatch (multi-dot AST path). 56 tests (38 unit + 6 storage + 6 query + 6 foreshadow + 4 sync hook). | ✅ shipped |
| **0.0.2-alpha.2** | **`Controller` trait + thread-per-controller.** `Controller: Send + 'static` with `apply(&mut self, m: Mutation)`; `InMemoryController` + `InMemoryHandle` (Clone + Send + Sync) for tests. `MutationLog` switched to a forwarder-closure model. Worker scaffolding: native uses `std::thread::spawn` + `std::sync::mpsc`, wasm uses `wasm_bindgen_futures::spawn_local` + `futures_channel::mpsc::unbounded`. `Engine::with_controller<C>(c)` does the wiring. `ControllerKind::InMemory` enum dropped in favour of an opaque `kind: String` (motif-core never enumerates concrete bridges). 60 tests; native-only worker test asserts in-flight ordering survives a 50-mutation burst. WASM artifact +310 KB after wasm-opt (713 KB total / 35% of 2 MiB budget) due to wasm-bindgen-futures + futures-channel + js-sys. | ✅ shipped |
| **0.0.2-alpha.3** | **Schema push + chores.** New `Schema` / `TableSchema` / `TableKind` / `PropertyType` types in `motif-core::schema`. `MutationOp::SchemaApply(Schema)` variant persists schemas on the same on-disk Mutation log (format version → 3). `Engine::apply_schema()`, `current_schema()`; inserts validate labels and surface `EngineError::SchemaUnknown { label, schema_version }` on misses. Permissive when no schema is set. Cypher metadata extends to `_motif.schema.version` (multi-segment paths inside the `_motif` namespace). Folded chores: `in-memory-controller` Cargo feature (default-on) so prod builds can drop it; `[scope]` for std::thread default + future Spawner trait; `[debt]` for unvalidated `controller.kind` slated for `with_named_controller` in alpha.5. 69 tests; v0.0.2-alpha.2 stores rejected on alpha.3 binary open. | ✅ shipped |
| **0.0.2-alpha.4** | **Worker state machine + config knobs + test profiles.** `Controller` trait grows `connect(&CapabilityConfig)` lifecycle hook + Result-returning `apply`; new `ControllerError { Transient, Permanent }`. Worker drives `connect → loop {apply with exp-backoff on Transient}`, capped by `EdgeConfig.controller_retry_max_backoff_ms`. `MotifConfig.capability` + `MotifConfig.edge` real now (parsed; retry knobs live, other knobs penciled). `default` + `potato` test profiles via `ThrottledStorage` wrapper; `hoverphone` deferred to v0.0.3. 77 tests; replay-from-disk after worker crash deferred to alpha.5. | ✅ shipped |
| **0.0.2-alpha.5** | **Audit pass.** PR #1 findings 4 (`extract_id_predicate` walks `AND` chains), 6 (`Storage::truncate` header guard), 7 (`MutationLog` poison recovery via `lock_recover()`) closed by code fix; finding 5 (unary minus) explicitly accepted. `Engine::with_named_controller(c, kind)` adds opt-in `controller.kind` validation (returns `ControllerKindMismatch` on a typo). `Engine::replay_unconfirmed()` walks the persisted log and re-feeds foreshadow=true mutations to a freshly wired controller after a crash. `motif bench` gained `--backend memory\|file` + `--with-controller` flags (file+controller smoke: p50 = 17.67 µs). New top-level `KUZU_DIVERGENCE.md` catalogues what we cut from upstream Kuzu by subsystem (storage, query, concurrency, indexes, types, catalog). LIMITATIONS.md sweep: fixed items retired, deferred items explicitly accepted with reasons inline. 85 tests (48 unit + 37 integration; +5 audit_pass over alpha.4). | ✅ shipped |
| **v0.0.2** | **Tag.** Foreshadow + persisted MutationLog; `Controller` trait + thread-per-controller worker; schema push + cached schema; worker state machine with reconnect-with-backoff; `[capability]` + `[edge]` config sections; audit pass closing PR #1 backlog. v0.0.2-alpha.5 audit pass: PR #1 findings 4 / 6 / 7 fixed in code, finding 5 explicitly accepted; `Engine::with_named_controller` (opt-in `kind` validation); `Engine::replay_unconfirmed` (replay foreshadow=true mutations after worker crash); `KUZU_DIVERGENCE.md` written. 85 tests at tag; 768 KB / 37.5% of WASM budget. | ✅ shipped |
| **v0.0.3** | **"Run on a real device" — North Star: a host on iOS / Android / edge ships an app that actually persists, with measurable cold-start.** Wasm storage shim (OPFS / app sandbox / wasm-bindgen-driven I/O); `Spawner` trait so hosts opt into platform runtimes (iOS GCD, Android coroutines); native `cdylib` evaluation for `aarch64-apple-ios` + `aarch64-linux-android` (via `uniffi` or direct cdylib — `cdylib` chosen in alpha.1 per the long-run strategy answer); wasm sleep via `gloo-timers` / `web-sys::setTimeout`; cold-start measurement harness; `tracing` crate with no-op default subscriber; `[capability]` auto-discovery (RAM / cores / arch probe). | in flight |
| **v0.0.3-alpha.1** | **Instrumentation foundation.** `tracing` crate added (no-op until host wires a subscriber), instrumented on `Engine::open_with` + the controller worker's connect / apply / retry paths. `[capability]` auto-discovery: probe is now primary (cores via `std::thread::available_parallelism`, RAM via `sysinfo`, disk via new `Storage::free_space`, arch via `cfg!`), per-field overridable from `motif.toml`'s `[capability]` section. `motif bench --cold-start [--seed N] [--iterations N] [--backend memory|file]` measures `Engine::open` timing. Native target decision: `cdylib` (no uniffi yet — defer until Swift FFI ergonomics force it). 93 tests (48 → 56 unit; +8 capability tests). | ✅ shipped |
| **v0.0.3-alpha.2** | **`Spawner` trait + real wasm sleep.** Native-only `Spawner` trait + default `StdThreadSpawner`; `Engine::with_controller_spawned_by(controller, spawner)` lets hosts on iOS / Android route the worker through GCD / coroutines instead of `std::thread::spawn` (post-alpha.4 cdylib targets). Wasm path keeps `spawn_local` directly (the host's wasm runtime *is* the spawner). `wasm_sleep` migrates from `future::ready` no-op to `gloo_timers::future::TimeoutFuture` — wasm retry backoff is real now. 97 tests (+2 spawner unit + 2 spawner integration). | ✅ shipped |
| **v0.0.4** | **"Real Cypher queries at scale" — North Star: apps express queries without falling back to engine API, on graphs sized the way gigue actually uses them.** Edge queries `MATCH (a)-[r]->(b)`; multi-pattern `MATCH`; `DETACH DELETE` / cascade; property type expansion (timestamps + lists at minimum); property-type validation against schema; `ORDER BY` + simple aggregates (`count`, `collect`); edge label + property indexes (graduated from "alpha.5+ work" — gigue B2B target makes O(N) edge scans non-negotiable); ID-namespace split (nodes / edges separate); `foreshadow_eager = false` buffer mode; `schema_cache = "fetch"` lazy fetch; performance benchmark vs upstream Kuzu. **First crates.io publish** for `motif-core` / `motif-wasm` / `motif-cli` with a pre-v0.1 "fluid API; expect breakage" README note — public-but-not-frozen API surface. | planned |
| **v0.0.5** | **"Hostile-device-aware" — North Star: auth + encryption are real, host policy is honoured, no quiet escape hatches.** New `[security]` TOML section: `require_authenticated_channel`, `min_aead`, `pq_required`, suite allow-list. Bridges advertise their crypto suite at handshake; motif validates declared-vs-policy and surfaces `ControllerSecurityError` on mismatch. Opt-out is explicitly named `Engine::dangerously_*` (Rust convention). SOTA-classical allow-list (X25519, ChaCha20-Poly1305, AES-GCM) is B2B table stakes. **PQ forward-compat plumbing** — Signal-PQXDH-style scanner, fail-visible when host requires PQ but bridge doesn't advertise; PQ implementation itself is stretch. Host token-issuance and controller key-rotation flows documented + tested. Encryption-at-rest design + impl. CRC on records. Tee panic-safety test. | planned |
| **v0.0.6** | **"Scale and operate" — North Star: confidence at the gigue B2B target (1M+ edges) and observability that survives a long-lived edge service.** Scale benchmarks at 1M edges + 10k nodes (p50 / p95 / p99 across query, insert, replay); `retention_confirmed_secs` log compaction; disk-size optimization (compression of bincode payloads, dictionary if it pays); `hoverphone` test profile via interleaved-test runner (closes v0.0.2 exit criterion 11 carry-over); multi-tenant evaluation (multiple `Engine` instances per host); backup / restore / migration design; WASM-size microtask trampoline if the futures-util budget bites. | planned |
| **v0.1.0** | **"OSS-ready" — North Star: external contributors `cargo add motif-core` with semver promises; first concrete bridge ships independently.** Public API freeze + audit; semver policy documented; first concrete bridge crate (the experience answers the `bridges/` vs `hubs/` partition question); bridge-author + hub-author guides; documentation pass (API docs, getting-started, deployment); `cpp-reference/` archived to a separate tag / repo. | planned |

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

## Long-run strategy: v0.0.3 → v0.1.0

**Target:** an OSS-ready micro-graph-db capable of supporting
enterprise-edge apps. Each minor has **one North Star**; debt that
touches that theme hitchhikes; everything else waits. Decisions in
this section supersede the loose "open questions for v0.0.3+" framing
that lived here through v0.0.2 — items below are now committed to a
release or explicitly parked.

### Scale target (gigue-driven)

Motif is designed for a **polypartite graph in B2B collaboration
contexts** — gigue's specific implementation will be a user- and/or
edge-scoped subgraph, hence the name. Per-device working set:

- **1k–10k nodes** (the original Kuzu-blueprint target).
- **100k–1M+ edges** at scale. Edge count significantly outpaces node
  count.

That asymmetry has design consequences. Edge label + property indexes
are co-equal deliverables with the v0.0.4 Cypher work, not
hitchhikers. Any `iter_edges()` O(N) scan path is debt to pay down,
not a "fine at small N" caveat. `KUZU_DIVERGENCE.md`'s scale row
reflects this.

### v0.0.3 — "Run on a real device"

Hosts on iOS / Android / edge ship apps that persist for real and
measurable cold-start. The wasm-vs-native question gets evaluated
end-to-end with both options on the table.

- Wasm storage shim (OPFS / app sandbox / wasm-bindgen-driven I/O).
- `Spawner` trait so hosts opt into platform runtimes (iOS GCD,
  Android coroutines) instead of `std::thread` everywhere.
- Native `cdylib` evaluation: `aarch64-apple-ios` + `aarch64-linux-android`
  via `uniffi` or direct `cdylib`. Decision in alpha.1 of the release.
- Wasm sleep via `gloo-timers` / `web-sys::setTimeout` (closes the
  v0.0.2 wasm-sleep no-op).
- Cold-start measurement harness; budget set in alpha.1.
- `tracing` crate, no-op default subscriber. Required for cold-start
  measurement and downstream observability.
- `[capability]` auto-discovery (RAM / cores / arch / GPU probe).

### v0.0.4 — "Real Cypher queries at scale" + first crates.io publish

Apps express queries without falling back to engine API, on graphs
sized the way gigue actually uses them. Indexes graduate to
North-Star tier: O(N) on edges is unacceptable at the gigue B2B
target.

Cypher surface:

- Edge queries `MATCH (a)-[r]->(b)`.
- Multi-pattern `MATCH`.
- `DETACH DELETE` / cascade semantics.
- Property type expansion (timestamps + lists at minimum;
  blobs / structs only when a query needs them).
- Property-type validation against schema.
- `ORDER BY` + simple aggregates (`count`, `collect`).
- `foreshadow_eager = false` buffer mode (some queries need
  consistent reads).
- `schema_cache = "fetch"` lazy fetch.

Indexes / scale:

- **Edge label + property indexes** (not just nodes).
- ID-namespace split: nodes and edges in separate maps.
- Performance benchmark vs upstream Kuzu (penciled in
  `KUZU_DIVERGENCE.md` since v0.0.2-alpha.5).

Distribution:

- Crates.io publish for `motif-core` / `motif-wasm` / `motif-cli`.
- README "**pre-v0.1; fluid API; expect breakage**" note —
  standard for serious 0.x crates.
- Public-but-not-frozen API surface. v0.1.0 freezes; v0.0.x can
  break.

### v0.0.5 — "Hostile-device-aware"

Auth + encryption become real. Motif imposes nothing — it inherits
host and controller policy, validates that the bridge actually meets
that policy, and refuses on mismatch. No quiet escape hatches.

Security model:

- New `[security]` TOML section. Host policy: `require_authenticated_channel`,
  `min_aead`, `pq_required`, suite allow-list.
- Bridges advertise their crypto suite at `Controller::connect`.
  Motif validates declared-vs-policy.
- Mismatch surfaces `ControllerSecurityError` to the host (downgrade
  attack, weak suite, missing PQ when required).
- Opt-out is explicitly `Engine::dangerously_*` (Rust convention you
  flagged) — `dangerously_ingest_unencrypted_mutations` and friends.
  No silent escape hatches.
- SOTA-classical allow-list as B2B table stakes: X25519,
  ChaCha20-Poly1305, AES-GCM.
- **PQ forward-compat plumbing** — Signal-PQXDH-style scanner,
  fail-visible when host requires PQ but bridge advertises classical-
  only. PQ implementation itself is stretch; the *handshake shape*
  must be PQ-aware so adopting Kyber768 / ML-KEM later is a bridge
  upgrade, not a motif upgrade.

Auth integration:

- Host token-issuance flow documented + tested with a spec controller.
- Controller key-rotation flow documented + tested.
- Capability fields for TEE / secure-enclave context.

Durability:

- Encryption-at-rest design + impl.
- CRC on records (closes the v0.0.2 [debt]).
- Tee panic-safety test (LIMITATIONS [debt]).

### v0.0.6 — "Scale and operate"

Confidence at the gigue B2B target. Long-lived edge-service
operability.

Scale:

- Benchmarks at 1M edges + 10k nodes; p50 / p95 / p99 across
  query / insert / replay.
- `retention_confirmed_secs` log compaction (closes the v0.0.2
  pencil).
- Disk-size optimization: bincode payload compression, dictionary
  encoding for repeated label / property names if it pays.
- Multi-tenant evaluation: multiple `Engine` instances per host
  process.

Operability:

- `hoverphone` test profile via interleaved-test runner (closes
  v0.0.2 exit criterion 11 carry-over).
- Backup / restore / migration design.
- WASM-size microtask trampoline if the futures-util budget bites
  (currently 768 KB / 37.5% of 2 MiB).

### v0.1.0 — "OSS-ready"

External contributors `cargo add motif-core` with semver promises.
First concrete bridge ships independently. The `bridges/` vs `hubs/`
partition decision falls out of the experience.

- Public API freeze + audit.
- Semver policy documented.
- First concrete bridge crate (likely SurrealDB; gigue drives
  separately under `phrase-db` naming per decision 3).
- Bridge-author + hub-author guides.
- Documentation pass: API docs, getting-started, deployment.
- `cpp-reference/` archived (separate tag / repo).

### Parked indefinitely (post-v0.1.0)

These appeared in v0.0.2 planning but have no destination yet:

- **CRDT / provisional-shadow conflict resolution beyond server-wins.**
  Server-wins is the v0.0.2 decision; revisit only if a real
  device-conflict scenario forces it. Not enterprise-blocking.
- **Bundled extension framework** (vector / FTS / etc.). Bridges,
  always (decision 18).
- **Multi-database per `Engine`.** Multi-tenant evaluation in v0.0.6
  may recommend an architectural split instead of growing the engine.
- **Browser-direct path / WASI runtimes.** Out of scope per
  decision 2.
- **Cypher beyond "query layer".** No transactions, no procedural
  extensions, no `CALL`, no list-comprehensions. Bridges that need
  that surface translate before talking to motif.

## Open questions

These wait on the first concrete bridge crate to answer:

1. **Bridge test parameterisation.** How do `default` / `potato` /
   `hoverphone` profiles get applied across multiple bridge crates
   without duplicating CI matrices?
2. **`bridges/` vs `hubs/` partition.** First bridge and first hub
   will tell us whether the directory split is worth keeping or
   whether everything wants to be `motif-*-ext/` or similar.

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
