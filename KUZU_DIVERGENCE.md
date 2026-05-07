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
laptop / server," Motif optimises for "tiny working set, simple queries
on a phone, controller-corrected on the wire."

## Subsystem-by-subsystem

### Storage

| Kuzu | Motif | Why |
|---|---|---|
| Columnar disk-based storage with buffer manager, segments, vectorised page layout. Multiple files (catalog + data + WAL + shadow). | Single-file append-only Mutation log; 16-byte header, length-prefixed bincode records. In-memory `id → offset` index rebuilt on open. | Mobile budget: <2 MB binary, <100 KB working set is realistic. Buffer manager + segments are overkill. The single file is also a friendly artifact for sandboxed app-data directories on iOS / Android. |
| Bespoke on-disk format with index hashing for `UINT128`, dictionary compression, etc. | bincode (serde) with a pinned config. No compression. | We ship far less data and far less variety. |
| Crash safety via WAL + shadow files + checkpoint. | No WAL beyond the append log itself. Torn-write recovery via bincode decode-error truncation. | Single-writer + per-write fsync gives mobile-grade durability without the complexity. CRC + crash-safety semantics are tracked in `LIMITATIONS.md` for v0.0.3+. |

### Query engine

| Kuzu | Motif | Why |
|---|---|---|
| Full openCypher: planner → optimiser → vectorised executor with morsel-driven parallelism and factorised intermediate state. | Hand-rolled lexer + recursive-descent parser + AST-walking interpreter. No planner, no optimiser. The AST is the plan. | Most local-cache reads are by-id lookups. Constant-time fast path (`WHERE id(n) = $x`) covers the hot path; O(N) scan for everything else is fine at expected scale (1k-10k nodes). |
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
| Hash index, vector index, full-text search, sparse-row CSR adjacency for joins. | One id index (`HashMap<String, IndexEntry>`) shared across nodes and edges. | Vector / FTS / specialised joins are explicit `[scope]` cuts, listed in `LIMITATIONS.md`. We don't ship them; controller bridges that need them route the relevant queries upstream. |

### Type system

| Kuzu | Motif | Why |
|---|---|---|
| `BOOL`, `INT8…INT64`, `UINT8…UINT128`, `SERIAL`, `FLOAT`, `DOUBLE`, `STRING`, `BLOB`, `DATE`, `TIMESTAMP`, `INTERVAL`, `UUID`, `STRUCT`, `MAP`, `LIST`, `UNION`, `NODE`, `REL`, `RECURSIVE_REL`. | `Null`, `Bool`, `I64`, `F64`, `String`. Five scalar variants. | Property types follow query needs. v0.0.3+ adds whatever the query layer asks for next; we don't preempt. |

### Catalog / schema

| Kuzu | Motif | Why |
|---|---|---|
| Engine owns the catalog: `CREATE NODE TABLE`, `CREATE REL TABLE`, schema migrations, type validation at insert. | Schema is **controller-owned**, pushed via `MutationOp::SchemaApply` over the same channel as mutations. Engine validates labels but not property types. No incremental migration — newer schema versions wholly replace older ones. | Motif is a follower; the upstream is the source of truth for shape. v0.0.3+ adds property-type validation if/when needed. |

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

Not yet measured. v0.0.3+ should benchmark the by-id lookup path
against an equivalent Kuzu query as a sanity check that we're at
least within 10× the right ballpark for our scale targets. Motif's
expected scale (1k-10k nodes) is far below where Kuzu's optimisations
start mattering, so we expect the direct comparison to be unflattering
on Kuzu's side at small N and unflattering on ours at large N — the
crossover point is the actual interesting datum.

`motif bench` is the harness today (`--backend memory|file`,
`--with-controller`); cross-engine comparison is post-v0.0.2.

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
