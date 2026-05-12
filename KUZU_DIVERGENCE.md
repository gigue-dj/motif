# Motif → Kuzu divergence

This document catalogues the meaningful deltas between Motif (the
greenfield Rust port at the top of this repo) and upstream Kuzu (the
frozen C++ reference at `cpp-reference/`). Contributors arriving with
Kuzu expectations can use this as a quick orientation: most of what
Kuzu does, Motif intentionally does not.

> **Cadence.** Refresh once per minor (between v0.0.x → v0.0.x+1) as
> part of the audit pass already in `LIMITATIONS.md`. Per-alpha is too
> noisy.
>
> **Scope of comparison.** Kuzu at the alpha.1 freeze point
> (`cpp-reference/`, derived from the Kuzu MIT release at the time of
> the Motif fork). We don't track ongoing upstream changes — Kuzu was
> archived after our fork landed.

## Why we diverged at all

Motif is a **mobile-first follower**. Kuzu is an **embedded
analytical graph engine**. The two end up in very different places
on the design dial because the constraints are different:

- Mobile / edge devices: tens of MB of RAM, flash storage, ARM,
  intermittent connectivity, hostile-device-aware.
- Single-writer client-side, integrity outsourced to a controller —
  not a server engine that owns truth.
- WASM as the universal compile target so iOS / Android / edge devices
  embed the same artifact via a wasm runtime.

Where Kuzu optimised for "lots of data, complex queries on a
laptop / server," Motif optimises for "small working set, simple
queries on a phone, controller-corrected on the wire."

### Scale target (gigue-driven)

Polypartite graph in B2B collaboration contexts (gigue's specific
implementation will be a user- or edge-scoped subgraph — hence the
name). Per-device working set:

- **1k–10k nodes** (the original Kuzu-blueprint target).
- **100k–1M+ edges** at scale. Edge count significantly outpaces
  node count.

That asymmetry has design consequences. Edge label + property
indexes graduate to North-Star tier in the v0.0.4 milestone (see
`MOTIF.md`); O(N) edge scans don't survive the upper bound. The
indexes row below reflects current state vs target state.

## Subsystem-by-subsystem

### Storage

| Kuzu | Motif | Why |
|---|---|---|
| Columnar disk-based storage with buffer manager, segments, vectorised page layout. Multiple files (catalog + data + WAL + shadow). | Single-file append-only Mutation log; 16-byte header, length-prefixed bincode records. In-memory `id → offset` index rebuilt on open. | Mobile budget: <2 MB binary, <100 KB working set is realistic. Buffer manager + segments are overkill. The single file is also a friendly artifact for sandboxed app-data directories on iOS / Android. |
| Bespoke on-disk format with index hashing for `UINT128`, dictionary compression, etc. | bincode (serde) with a pinned config. No compression. | We ship far less data and far less variety. |
| Crash safety via WAL + shadow files + checkpoint. | No WAL beyond the append log itself. Torn-write recovery via bincode decode-error truncation. | Single-writer + per-write fsync gives mobile-grade durability without the complexity. CRC + improved crash-safety semantics land in v0.0.5 alongside encryption-at-rest — both are storage-layer touches. |

### Query engine

| Kuzu | Motif | Why |
|---|---|---|
| Full openCypher: planner → optimiser → vectorised executor with morsel-driven parallelism and factorised intermediate state. | Hand-rolled lexer + recursive-descent parser + AST-walking interpreter. No planner, no optimiser. The AST is the plan. | Most local-cache reads are by-id lookups. Constant-time fast path (`WHERE id(n) = $x`) covers the hot path through v0.0.3. v0.0.4 grows label + property indexes (nodes AND edges) — see the indexes row — so non-id MATCH stays sub-linear at the gigue B2B target (100k–1M+ edges). |
| Multi-statement queries, `WITH` clause, `OPTIONAL MATCH`, list comprehensions, subqueries, `CALL`, aggregation, `ORDER BY`, grouping. | Single-statement subset: `CREATE`, `MATCH` + `WHERE` + `RETURN` + `LIMIT`, `MERGE` (no-op-on-hit), `MATCH ... DELETE`. | Targeted at "bind a parameter, fetch the row." Anything else is the controller's problem. |
| Multi-pattern `MATCH (a)-[r]->(b)`, edges queryable, properties on relationships. | Single bound variable per statement. Edges not queryable from Cypher (only via the engine API). | One thing well; multi-pattern join planning belongs in the controller. |
| Rich expression layer: arithmetic, list / map / struct comprehensions, regex, full-text MATCH. | `=` `!=` `<` `>` `<=` `>=` `AND` `OR` `NOT`, the `id(n)` builtin, integer / float / string / bool / null literals. Three-valued logic for `Null`. | Cypher's full surface is huge; we ship the slice queries actually need. |

### Concurrency

| Kuzu | Motif | Why |
|---|---|---|
| ACID transactions with MVCC; multiple concurrent readers + writers. | Single-writer engine (`&mut self` for both reads and writes). No transactions. | Mobile clients don't have multiple writers. Removing MVCC removes a lot of code. |
| Multi-core query parallelism. | Sync core; one thread per controller worker; that's it. | Sync core is intentional (MOTIF.md decision 12). Parallel query execution is a server-side concern. |

### Indexes

| Kuzu | Motif | Why |
|---|---|---|
| Hash index, vector index, full-text search, sparse-row CSR adjacency for joins. | **Through v0.0.3:** one id index (`HashMap<String, IndexEntry>`) shared across nodes and edges. **v0.0.4-alpha.1:** node and edge namespaces split into independent maps; `edge_by_label` index + `iter_edges_by_label` API land. **Edge property index** is alpha.2 work (alongside the Cypher edge surface that drives it). **Node label / property indexes** stay deferred (10k node ceiling per the gigue B2B target keeps O(N_nodes) cheap). Vector / FTS / specialised joins remain explicit `[scope]` cuts. | The shared map was fine for the v0.0.2 by-id hot path; the gigue B2B target (100k–1M+ edges) makes O(N) edge scans non-negotiable, so the v0.0.4 milestone graduates real edge indexes alongside the Cypher surface growth. Vector / FTS stay bridge concerns — controller bridges that need them route the relevant queries upstream. |

### Type system

| Kuzu | Motif | Why |
|---|---|---|
| `BOOL`, `INT8…INT64`, `UINT8…UINT128`, `SERIAL`, `FLOAT`, `DOUBLE`, `STRING`, `BLOB`, `DATE`, `TIMESTAMP`, `INTERVAL`, `UUID`, `STRUCT`, `MAP`, `LIST`, `UNION`, `NODE`, `REL`, `RECURSIVE_REL`. | `Null`, `Bool`, `I64`, `F64`, `String`, `Timestamp` (epoch ms; alpha.3), `List` (heterogeneous; alpha.3). Seven variants shipped; `Blob` / `Map` / `Struct` discriminants reserved for v0.0.5+. | Property types follow query needs. Codec discriminant layout pencilled in `value.rs` so v0.0.5+ additions don't re-encode shipped data. We don't preempt typed lists, required-property markers, or the larger int / unsigned families until a caller asks. |

### Catalog / schema

| Kuzu | Motif | Why |
|---|---|---|
| Engine owns the catalog: `CREATE NODE TABLE`, `CREATE REL TABLE`, schema migrations, type validation at insert. | Schema is **controller-owned**, pushed via `MutationOp::SchemaApply` over the same channel as mutations. Engine validates labels (since v0.0.2-alpha.3) and property types (since v0.0.4-alpha.3); permissive on undeclared properties; `Value::Null` accepted for any declared type. No incremental migration — newer schema versions wholly replace older ones. | Motif is a follower; the upstream is the source of truth for shape. Required-property markers and typed lists wait for a caller need. |

### Bindings / distribution

| Kuzu | Motif | Why |
|---|---|---|
| Java, Python, Node.js, Rust, C, C++, WebAssembly bindings. Pre-compiled binaries for Linux / macOS / Windows × x86_64 / arm64. | Rust core; wasm-bindgen for `wasm32-unknown-unknown`. Native targets (iOS / Android) penciled for v0.0.3+. | Single shipping artifact per MOTIF.md decision 2. |
| Extension framework with 16+ bundled extensions (`fts`, `vector`, `azure`, `delta`, `duckdb`, `httpfs`, `iceberg`, `json`, `llm`, `neo4j`, `postgres`, `sqlite`, `unity_catalog`). | No extensions. Controller bridges (`motif-*-bridge`) are optional separate crates. | Controller-agnostic posture (decision 18). Motif-core never imports a network DB client. |

### Controller / sync

This is the section without a Kuzu counterpart. Kuzu is authoritative
where Motif is a follower:

- `Controller` trait + worker-per-controller scaffolding (native
  `std::thread`, wasm `wasm-bindgen-futures`).
- `Mutation { local_seq, actor, foreshadow, op }` is the on-disk
  record AND the wire-shaped message.
- Server-wins resolution with `foreshadow: bool` for in-flight tracking.
- `metadata-as-data` Cypher namespace (`n._motif.foreshadow`,
  `n._motif.schema.version`).
- `replay_unconfirmed` for catching a fresh worker up after a crash.

These live in `crates/motif-core/src/sync/` and the engine's commit
path; the upstream Kuzu had nothing to port here.

## Performance comparison

Not yet measured. The v0.0.4 milestone ("Real Cypher queries at
scale") includes a benchmark vs upstream Kuzu — by-id lookup path
plus a representative non-id `MATCH` once edge indexes are in place.
Sanity check: are we within 10× the right ballpark at the gigue B2B
target (1k–10k nodes, 100k–1M+ edges)?

We expect the direct comparison to be unflattering on Kuzu's side
at small N (Motif has no buffer manager / no MVCC / no planner
overhead) and unflattering on ours at large N (no vectorised
executor, no morsel parallelism). The crossover point is the
actual interesting datum.

`motif bench` is the harness today (`--backend memory|file`,
`--with-controller`); cross-engine comparison lands in v0.0.4.

## What we kept

- The `cpp-reference/` tree itself, frozen, MIT-licensed via
  `cpp-reference/LICENSE-MIT-KUZU`. Useful as a behavioural spec when
  designing new query semantics.
- Cypher (subset). The query language is the user-facing seam, and
  Cypher is well-understood.
- "Property graph" data model: nodes + edges, both with arbitrary
  property bags.
- "Embedded, no server process" design philosophy — even though Motif
  takes that further (no buffer manager, no parallelism) it's the
  same posture.

## What we explicitly will not bring back

These are decisions, not omissions:

- Multi-writer / MVCC / ACID transactions — single-writer is the
  right shape for the follower role.
- A bundled extension framework — bridges are separate crates,
  always (MOTIF.md decision 18).
- Vector / FTS indexes inside motif-core — those are bridge concerns.
- The full Cypher surface — we ship the subset queries actually need
  on the edge.

If a future contributor wants any of the above, the answer is: build
it as a separate `motif-*-bridge` or `motif-*-hub` crate, or fork the
engine. Motif's core stays small.
