# Motif v0.0.1 — design notes and MVP plan

This document captures the architectural decisions for Motif v0.0.1 and the
scope of the alpha milestones. It is the single source of truth for the
"what and why" of the fork from Kuzu. If something here disagrees with code,
the doc is wrong — open an issue.

## What Motif is

Motif is a **follower** graph store. A small embedded engine, derived from
[Kuzu](https://github.com/kuzudb/kuzu/), packaged as a WebAssembly module and
embedded in mobile apps and edge devices. It holds a local property graph for
query speed and offline operation. **It is not the source of truth for any
data it holds.** Integrity, schema, and conflict resolution belong to an
upstream **controller**.

## What Motif is not

- Not a server.
- Not authoritative.
- Not a long-lived store. The local file is a working cache; the controller
  can sunset any local state.
- Not a portable replacement for Kuzu. Motif drops most of Kuzu's surface
  area (extensions, language bindings, vector/FTS, multi-platform precompiled
  binaries).

## Topology

```
  ┌───────────────────────────────────────────────────────┐
  │ host app  (iOS / Android / edge)                      │
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

## Locked-in decisions (v0.0.1)

| # | Topic | Decision |
|---|---|---|
| 1 | Target platform | WASM only. iOS, Android, and edge devices all run the same wasm artifact. |
| 2 | Controller | SurrealDB for v0.0.x. Custom Nebula-class controller later. |
| 3 | Auth | Per-user **and** per-device. Treat all devices as potentially compromised. JWT-style bearer for users; per-device key for device identity. (No transport in v0.0.1; identity surfaced as `userId` + `deviceId` parameters.) |
| 4 | Conflict resolution | **Server-wins.** Local can apply optimistic mutations; controller can override or sunset them. (Provisional/CRDT shadow layer is a v0.0.2 design item, see open questions.) |
| 5 | Local read miss | Return stale-with-flag. Async refresh. Never block on the controller. |
| 6 | Offline mode | First-class from day 0. Writes queue locally; reads work without connectivity. |
| 7 | Schema ownership | Controller owns schema and pushes to followers. Motif does not declare. v0.0.1 accepts a JSON descriptor injected by the host app at open time; live push lands in v0.0.2. |
| 8 | Query language | Internal: Kuzu's Cypher subset (CRUD only, gated at the binder). Boundary: SurrealQL translation lands in v0.0.2 once real transport is wired. |
| 9 | Encryption-at-rest | Out of scope for v0.0.1; storage layer must not preclude it. |
| 10 | Terminology | "Follower," not "slave/replica." |

## v0.0.1 scope

### v0.0.1-alpha.1 (this milestone — foundation)

- [x] Fork from Kuzu, prune extensions, language bindings, benchmarks,
      multi-platform CI.
- [x] Top-level rebrand (CMake project name, WASM `EXPORT_NAME`, npm package
      name, README). C++ `kuzu::` namespace retained — rename is v0.0.2.
- [x] Slim Makefile + single CI workflow (`wasm-workflow.yml`,
      `ubuntu-latest`).
- [x] `MOTIF.md` (this file) capturing decisions.
- [x] `src/sync/` skeleton: `ControllerClient` interface and `MutationLog`
      data structure. No transport. No WAL hook yet.
- [ ] `cmake -DBUILD_WASM=TRUE` configures cleanly after prune.

### v0.0.1-alpha.2 (architectural validation)

- [ ] WAL commit hook tees mutations to `MutationLog`. Verify by test spy.
- [ ] `Motif.open(path, { userId, deviceId, schema })` JS wrapper over the
      existing `Database`/`Connection` API. `userId` and `deviceId`
      required.
- [ ] e2e harness in `test/motif_mvp/` measuring p50/p95 single-node read
      latency in a headless Chrome wasm context.

### v0.0.1 exit criteria

1. `npm install motif-wasm && new Motif().query("MATCH (n) RETURN n LIMIT 10")`
   works in a browser tab.
2. p50 single-node lookup <50 ms on a mid-tier Android via WASM in Chrome.
3. Stripped wasm artifact <5 MB.
4. Mutation hook fires `ControllerClient.applyMutation` for every commit
   (verified by test spy).

### Explicitly out of scope for v0.0.1

iOS/Android native bindings, real network transport to SurrealDB,
provisional-write shadow layer / scoped CRDT, conflict resolution wire
protocol, offline replay, JWT/auth signing, encryption-at-rest, vector
search, full-text search, multi-tenant, schema migrations.

## Open questions for v0.0.2

1. **Provisional layer shape.** Server-wins says the controller can override.
   Do we model "provisional" as a per-mutation flag in `MutationLog`, or as
   a parallel shadow store that the WAL replayer reconciles? Trade-off is
   memory/space vs. read-path complexity.
2. **SurrealQL boundary.** Translate at the JS API layer (motif-wasm
   accepts SurrealQL strings, transpiles to Cypher before hitting the
   binder), or swap the parser entirely (replace
   `third_party/antlr4_cypher` with a SurrealQL grammar)? Translation is
   cheaper for v0.0.x.
3. **Schema push channel.** Same transport as mutations (WS), or a
   distinct control channel? Affects how schema versions interact with
   pending mutations.
4. **Device key provisioning.** Per-device keys assume a first-run pairing
   flow. Owned by host app, or by Motif?
5. **Sunset semantics.** Concretely: when the controller "overrides" a
   provisional change, does Motif emit a notification to the host app's
   object graph, and what does that look like ergonomically?

## Provenance

Motif is derived from Kuzu (MIT). The C++ engine — storage, WAL, transactions,
binder, planner, vectorized executor — is upstream Kuzu, retained verbatim
in v0.0.1. The Motif-specific code is everything under `src/sync/`, the
top-level rebrand, and the Cypher-subset gating that lands in v0.0.2.
