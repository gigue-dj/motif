# Motif — running limitations ledger

This file is a **running ledger of every caveat we are knowingly building
in**. It is an audit aid, not a spec — `MOTIF.md` is the spec. The ledger
exists so that the audit pass between releases (CodeRabbit + a manual
sweep, performed for v0.0.1 in PR #1) has a single place to attack, and so
anyone reading the codebase can tell at a glance which "TODO"-shaped
silences in code are tracked vs. forgotten.

> **Scope:** this file is documentation only and is not compiled into the
> WASM artifact or any crate. Update it whenever an alpha lands; prune it
> whenever a limitation is closed.

## Tag legend

- `[scope]` — intentional MVP cut. Will not change before v0.0.1; may or
  may not change in v0.0.2+.
- `[gap]` — known missing functionality that a later release will fill
  (within the current minor for in-flight work, or in a future minor).
- `[debt]` — known imperfection in shipped code. Flagged for the
  pre-v0.0.1 audit pass; may demand a fix or an explicit accept.
- `[perf]` — performance compromise that is acceptable at current scale
  but will need attention as the graph grows.

Each item carries the file/region where the caveat lives, so the audit
pass can grep its way to the source.

---

## Build / distribution

- `[scope]` **WASM target only** — `wasm32-unknown-unknown`. No
  `wasm32-wasip1`, no native targets (`aarch64-apple-ios`,
  `aarch64-linux-android`), no browser-direct path. Host apps embed
  `wasmtime` or similar and pay the runtime tax.
  *Source:* `MOTIF.md` decision 2; `rust-toolchain.toml`.
- `[scope]` **WASM runtime perf tax** — host-runtime execution costs
  ~2–5× vs. native Rust. Acceptable for v0.0.1; native targets are a
  v0.0.2+ option.
  *Source:* `MOTIF.md` decision 2.
- `[scope]` **Internally usable only** — all crates are
  `publish = false`. No crates.io, no semver guarantees.
  *Source:* `Cargo.toml`; `MOTIF.md` decision 14.
- `[scope]` **Dual MIT / Apache-2.0** — Rust convention; not yet
  uniform with `cpp-reference/` (which remains MIT only via the
  inherited Kuzu license).
  *Source:* `LICENSE-MIT`, `LICENSE-APACHE`; `MOTIF.md` decision 15.

## Configuration (`motif-core::config`)

- `[scope]` **All TOML fields required.** No defaults yet — host apps
  must spell out `[identity]`, `[controller]`, `[storage]`. Defaults
  arrive only when a concrete reason demands one.
  *Source:* `crates/motif-core/src/config.rs:22-26`.
- `[scope]` **Identity fields are opaque strings.** `user_id` and
  `device_id` are not validated against any auth system; a real JWT /
  per-device-key flow is v0.0.2 work.
  *Source:* `crates/motif-core/src/config.rs:31-39`;
  `MOTIF.md` decision 4.
- `[gap]` **Only `ControllerKind::InMemory` parses.** TOML rejects
  `kind = "surreal"` or anything else. The enum gains variants when
  v0.0.2 wires real transport.
  *Source:* `crates/motif-core/src/config.rs:53-56`.

## Storage (`motif-core::storage`, `motif-core::record`)

- `[scope]` **Greenfield format.** No on-disk compatibility with the
  C++ baseline at `cpp-reference/`. Format version is `1`; any change
  bumps the constant in `storage.rs` and forces re-bootstrap.
  *Source:* `crates/motif-core/src/storage.rs:21-23`;
  `MOTIF.md` decision 10.
- `[debt]` **No CRC on records.** Length-prefixed `bincode` only. Torn
  writes surface as decode errors during recovery; in-place corruption
  in the middle of the log is detected only when read. CRC + crash-
  safety semantics land alongside the persistent `MutationLog` in
  v0.0.2.
  *Source:* `crates/motif-core/src/record.rs:1-15`.
- `[debt]` **Recovery silently truncates a torn tail.** No diagnostic
  is surfaced beyond log replay continuing past the truncation. Should
  emit a `tracing` event when `tracing` is added in v0.0.2.
  *Source:* `crates/motif-core/src/engine.rs:113`,
  `crates/motif-core/src/storage.rs:211-222`.
- `[debt]` **`fsync` per write.** `FileStorage::append` fsyncs every
  record. Correct, but expensive on flash. Batched / opt-in fsync is a
  v0.0.2 perf knob.
  *Source:* `crates/motif-core/src/storage.rs:170-188`.
- `[scope]` **Storage trait is sync.** No async I/O in `motif-core`
  per the "sync core" decision. Async at the edges only.
  *Source:* `crates/motif-core/src/storage.rs:52-71`;
  `MOTIF.md` decision 12.
- `[gap]` **`FileStorage` fails at open on `wasm32`.** No filesystem
  on the target. v0.0.1 will use `MemoryStorage` for the wasm path; a
  host-provided storage shim (alpha.5+) is the longer-term answer.
  *Source:* `crates/motif-core/src/storage.rs:5-8`.
- `[scope]` **Single-namespace ID index.** Nodes and edges share
  `HashMap<String, IndexEntry>`. Globally unique ids; trivially split
  in v0.0.2 if it bites.
  *Source:* `crates/motif-core/src/engine.rs:55-61`.
- `[debt]` **No referential integrity on node delete.** Deleting a
  node that has incoming/outgoing edges leaves them dangling; the
  query layer treats them as unreachable. Cascade (`DETACH DELETE`)
  arrives with the controller-side conflict resolution in v0.0.2.
  *Source:* `crates/motif-core/src/engine.rs:200-211`.
- `[perf]` **`iter_nodes` / `iter_edges` are O(N) reads.** The id
  index is the only secondary index in v0.0.1; any MATCH without an
  `id()` predicate scans every record. Acceptable up to ~1k nodes; a
  label / property index is alpha.5+ work.
  *Source:* `crates/motif-core/src/engine.rs:225-264`.
- `[debt]` **Property values are limited to 5 scalar variants.**
  `Null`, `Bool`, `I64`, `F64`, `String`. No timestamps, no blobs, no
  lists, no nested structs. Expanded when the query layer needs more.
  *Source:* `crates/motif-core/src/value.rs:1-13`.
- `[debt]` **`Storage::truncate` does not enforce
  `new_len >= HEADER_LEN`.** No reachable caller violates this today
  (recovery initialises `last_good = HEADER_LEN`), but a defensive guard
  prevents future misuse from corrupting the magic. ~3-line add when
  v0.0.2 touches the storage layer.
  *Source:* `crates/motif-core/src/storage.rs` (`FileStorage::truncate`,
  `MemoryStorage::truncate`). Logged from PR #1 review (finding 6).

## Query (`motif-core::query`)

- `[scope]` **Hand-rolled lexer + parser + interpreter** (not a parser
  combinator framework). No planner, no optimizer — the AST is the
  plan. Reassessed once exit criteria are met.
  *Source:* `crates/motif-core/src/query/mod.rs:1-40`;
  `MOTIF.md` decision 11.
- `[scope]` **Single bound variable per statement.** No
  `MATCH (a)-[r]->(b)`, no multi-pattern, no `WITH`, no
  `OPTIONAL MATCH`.
  *Source:* `crates/motif-core/src/query/mod.rs:30-44`,
  `crates/motif-core/src/query/ast.rs:5-7`.
- `[gap]` **Edges aren't queryable.** Engine API exposes `insert_edge`
  / `get_edge` / `iter_edges`, but Cypher queries can only target
  nodes in v0.0.1. Edge query support is alpha.5+.
  *Source:* `crates/motif-core/src/query/mod.rs:33-35`.
- `[scope]` **`MERGE` is no-op-on-hit, not full upsert.** Existing
  nodes are not updated; only missing nodes are inserted. Real upsert
  is post-v0.0.1 once the controller decides what "update" means.
  *Source:* `crates/motif-core/src/query/mod.rs:38-40`,
  `crates/motif-core/src/query/interpreter.rs:89-112`.
- `[scope]` **`CREATE` / `MERGE` require an explicit `id` string
  property.** Engine does not assign synthetic ids.
  *Source:* `crates/motif-core/src/query/mod.rs:36-37`.
- `[scope]` **Only built-in function is `id(n)`.** No `count`,
  `collect`, `coalesce`, etc.
  *Source:* `crates/motif-core/src/query/mod.rs:41`.
- `[scope]` **No multi-statement / transactions.** Each `Engine::query`
  call is one statement, auto-committed.
  *Source:* `crates/motif-core/src/engine.rs:1-15`.
- `[debt]` **Three-valued logic is implemented but lightly tested.**
  `Null` propagation through `AND`, `OR`, comparisons, and `NOT`
  matches Cypher semantics in code but only has incidental coverage
  in `query_smoke.rs`. Add explicit tests in the audit pass.
  *Source:* `crates/motif-core/src/query/interpreter.rs:284-345`.
- `[debt]` **Lexer escapes are minimal.** Only `\\ \" \' \n \t`. No
  Unicode escapes, no `\r`, no octal/hex. Fine for English-language
  test data; revisit before any locale-bearing host ships on top.
  *Source:* `crates/motif-core/src/query/lexer.rs:5-7`.
- `[debt]` **`DELETE` orphans edges silently.** Documented at the
  engine level; the parser does not surface a warning. The audit
  pass should decide whether the interpreter rejects the query when
  edges reference the doomed node.
  *Source:* `crates/motif-core/src/query/interpreter.rs:143-160`.
- `[perf]` **`extract_id_predicate` only matches a top-level
  `id(n) = X`.** `MATCH (n) WHERE id(n) = $x AND n.foo = 1` falls back
  to a full `iter_nodes()` scan instead of a constant-time index
  lookup + secondary filter. v0.0.2 fix is to walk an `AND` chain for
  the predicate; trivial change once a Conjunction normaliser exists.
  *Source:* `crates/motif-core/src/query/interpreter.rs:216-241`.
  Logged from PR #1 review (finding 4).
- `[scope]` **No unary minus expression form.** `-10` parses as the
  literal `Integer(-10)`; `-n.balance` does not parse. Acceptable for
  v0.0.1; trivial to add when the expression layer grows arithmetic.
  *Source:* `crates/motif-core/src/query/lexer.rs` (`scan_number`
  entry path). Logged from PR #1 review (finding 5).

## Sync layer (`motif-core::sync`)

- `[gap]` **No transport.** Only `InMemoryControllerClient` exists.
  Real SurrealDB transport (probably WS or HTTP) is v0.0.2.
  *Source:* `crates/motif-core/src/sync/controller_client.rs:1-7`.
- `[gap]` **`MutationLog` is in-process only.** Crash-safe persisted
  log alongside storage is v0.0.2. Until then, queued mutations are
  lost on restart.
  *Source:* `crates/motif-core/src/sync/mutation_log.rs:5-9`.
- `[debt]` **Tee fires after the storage append but before the index
  publishes for inserts.** A panic between those two steps would leave
  a record on disk that subsequent recovery picks up, with the
  controller already informed — fine. The opposite order would let a
  reader observe a write the controller doesn't know about, so the
  current order is intentional. Document and lock in with a panic-
  safety test in the audit pass.
  *Source:* `crates/motif-core/src/engine.rs` (`append_record`,
  `tee_mutation`).
- `[debt]` **`read_label` for delete tees does an extra read.**
  We re-read the deleted record's frame just to recover the label so
  the `Mutation::table_name` field is meaningful. Cheap (one offset
  read) but redundant; a label cache on `IndexEntry` is a v0.0.2 win.
  *Source:* `crates/motif-core/src/engine.rs` (`read_label`).
- `[scope]` **`wal_payload` is opaque bytes.** v0.0.1 forwards
  serialized engine records verbatim. Structured diffs for the
  controller arrive when the SurrealQL boundary lands.
  *Source:* `crates/motif-core/src/sync/mutation.rs:1-3`.
- `[debt]` **`wal_payload` shape is asymmetric across inserts and
  deletes.** Inserts ship `&frame[LEN_PREFIX_BYTES..]` (bincode-encoded
  `Record::*Insert`); deletes ship `id.as_bytes()` (raw UTF-8, not
  valid bincode for a `String`). When the controller transport lands
  in v0.0.2 the consumer would have to branch on `kind` to decode. A
  uniform "always bincode of `Record::*`" shape is one small change.
  *Source:* `crates/motif-core/src/engine.rs` `delete_node:272`,
  `delete_edge:284`. Logged from PR #1 review (finding 2).
- `[debt]` **Naming inconsistency: `MutationKind::RelInsert` /
  `RelDelete` vs `Record::EdgeInsert` / `EdgeDelete`.** Sync layer
  uses Cypher-conventional `Rel`; storage layer uses `Edge`. Internal
  only today (the inconsistency never leaves the crate). v0.0.2
  should pick one once the controller wire format is real.
  *Source:* `crates/motif-core/src/sync/mutation.rs:18-26`,
  `crates/motif-core/src/record.rs:21-25`. Logged from PR #1 review
  (finding 1).
- `[debt]` **`MutationLog::record` uses `.expect("poisoned")` on the
  mutex.** Single-threaded today; if a future alpha adds parallel
  writers, a panic on a previous lock holder propagates. Either keep
  the panic (single-writer is the documented model) or switch to
  `lock().unwrap_or_else(|e| e.into_inner())` for poison recovery.
  *Source:* `crates/motif-core/src/sync/mutation_log.rs`. Logged from
  PR #1 review (finding 7).
- `[gap]` **No provisional / CRDT shadow layer.** Server-wins is the
  decision; the local-temp-override mechanism is a v0.0.2 design
  item.
  *Source:* `MOTIF.md` decision 5; open question 1.

## Bindings (`motif-wasm`, `motif-cli`)

- `[scope]` **`Motif::open` on wasm uses `MemoryStorage`.** No
  filesystem on `wasm32-unknown-unknown`, so the `storage.path` field
  of the TOML config is ignored on the wasm path. A host-provided
  storage shim (OPFS / app sandbox / wasm-bindgen-driven I/O) is
  post-v0.0.1.
  *Source:* `crates/motif-wasm/src/lib.rs:1-15`.
- `[scope]` **Wasm params + result marshalled as JSON strings.** No
  `serde-wasm-bindgen` dependency; host wraps in `JSON.stringify` /
  `JSON.parse`. Acceptable for v0.0.1 (one allocation per call); a
  binary marshalling path is v0.0.2 if profiling shows it matters.
  *Source:* `crates/motif-wasm/src/lib.rs` (`query`, `parse_params`).
- `[debt]` **Wasm `params_json` rejects nested objects / arrays.**
  Only scalar params (`null`, `bool`, integers, floats, strings).
  Lists / structs of values land when the engine grows them.
  *Source:* `crates/motif-wasm/src/lib.rs:107-112`.
- `[debt]` **Wasm `mutation_count` is a buffered-only counter.** It
  reports `MutationLog::buffered_len` — fine while no client is wired
  on the wasm path, but if a real client is connected later the
  counter will under-report. Add an explicit per-instance counter in
  the audit pass.
  *Source:* `crates/motif-wasm/src/lib.rs` (`mutation_count`).
- `[scope]` **`motif-cli bench` measures in-memory storage on
  native.** Does not exercise `FileStorage` (and therefore not the
  `fsync`-per-write cost). A separate file-backed bench is post-v0.0.1
  — useful for the storage perf knob discussion in v0.0.2.
  *Source:* `crates/motif-cli/src/main.rs` (`run_bench`).

## Operations / observability

- `[scope]` **No metrics, no tracing.** No `tracing` crate, no
  counters, no spans. Logs go to nowhere. Adding `tracing` (with a
  no-op default subscriber) is a v0.0.2 chore — keeps `motif-core`
  observable without forcing a runtime choice on the host.
- `[scope]` **No connection pooling / multi-database.** One `Engine`
  per file, single writer, single reader (the engine takes
  `&mut self` for both). Multi-tenant is v0.0.2+.
  *Source:* `crates/motif-core/src/engine.rs:1-15`.
- `[scope]` **No backup / restore / migration.** The 4-byte
  format-version field lives in the file header but the engine
  rejects any version other than `1`. No migration script yet.
  *Source:* `crates/motif-core/src/storage.rs:21-23,156-163`.

## Security

- `[scope]` **No encryption-at-rest.** Records are plaintext
  `bincode`. Storage layer must not bake in plaintext-only
  assumptions; revisit when the per-device-key flow lands.
  *Source:* `MOTIF.md` decision 13.
- `[scope]` **No auth signing on mutations.** `ActorId` is opaque
  strings; the controller is expected to validate, but there is no
  controller. JWT / per-device-key signing is v0.0.2.
  *Source:* `crates/motif-core/src/sync/mutation.rs:7-15`;
  `MOTIF.md` decision 4.
- `[scope]` **`unsafe_code = "forbid"`** at the workspace level. This
  is a *feature*, not a limitation; noted here so the audit pass can
  confirm the lint is still in place.
  *Source:* `Cargo.toml:21-22`.

## C++ baseline (`cpp-reference/`)

- `[scope]` **Frozen.** Not built, not tested, not in CI. Acts as a
  behavioural reference for the Rust port. Will be archived (tag +
  optional separate repo) once v0.0.1 ships.
  *Source:* `cpp-reference/`; `MOTIF.md` milestone table.

---

## Audit cadence

Between the final alpha of a release and the release tag we run a single
combined pass:

1. **CodeRabbit** on the full diff from the prior release to the new
   release-candidate tag.
2. **Manual sweep** against this file: every `[debt]` either gets a fix
   commit or an explicit accept (with the reason added in-line below the
   item).
3. **`[scope]` items are reviewed but not changed** unless the maintainer
   explicitly opens a release-blocking issue.

The v0.0.1 audit pass happened in PR #1; findings 1, 2, 4, 5, 6, 7 from
that review were accepted as deferrable and are tagged in this file as the
v0.0.2 backlog. The same cadence applies between v0.0.1 and v0.0.2.
