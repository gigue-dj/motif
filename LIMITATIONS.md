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

> **v0.0.2-alpha.1 retired:** PR #1 review findings 1 (`MutationKind::Rel*`
> vs `Record::Edge*` naming) and 2 (`wal_payload` asymmetric across
> inserts and deletes) — both subsumed by the redesign that collapsed
> the old `Record` enum and `MutationKind` enum into the unified
> `Mutation` / `MutationOp` shape. PR #1 finding 3 (`read_label`
> empty-string fallback for delete tees) is also retired — deletes now
> carry the original op directly.
>
> **v0.0.2-alpha.2 retired:** the `ControllerKind::InMemory`-only `[gap]`
> entry (the enum is gone; `kind` is now an opaque string). The
> in-memory MutationLog `[gap]` is also retired — the worker
> consumes the log on its own thread (native) or microtask (wasm).
>
> **v0.0.2-alpha.3 added:** schema push (controller-pushed `Schema` via
> `MutationOp::SchemaApply`); engine validates labels against the
> current schema; `_motif.schema.version` resolves via the metadata-as-
> data Cypher namespace. Format version bumped to `3`. New scope items
> for std::thread default + future Spawner trait, feature-flagged
> InMemoryController, and unvalidated `controller.kind` (slated for
> with_named_controller in alpha.5).
>
> **v0.0.2-alpha.4 added:** controller worker is now a state machine
> with `connect → loop {apply with exp-backoff on Transient}` and a
> richer `Controller` trait. `CapabilityConfig` + `EdgeConfig` parsed;
> retry knobs live, other knobs penciled. `default` + `potato` test
> profiles stood up (`ThrottledStorage` wrapper). New gaps logged for
> replay-from-disk (alpha.5), wasm sleep (v0.0.3+),
> foreshadow_eager=false (alpha.5), retention compaction (alpha.5),
> schema_cache=fetch (post-v0.0.2), hoverphone profile (v0.0.3),
> capability auto-discovery (v0.0.3).
>
> **v0.0.2-alpha.5 retired (audit pass — closed by code fix):**
> PR #1 finding 4 (`extract_id_predicate` only matched a top-level
> `id(n) = X`) — interpreter now walks `AND` chains via a recursive
> helper. PR #1 finding 6 (`Storage::truncate` lacked a header guard) —
> both `FileStorage` and `MemoryStorage` now reject
> `new_len < HEADER_LEN` with `StorageError::TruncateBelowHeader`.
> PR #1 finding 7 (`MutationLog::record` mutex poison) — all locks now
> use `lock_recover()` (`PoisonError::into_inner`). Controller-kind
> validation: `Engine::with_named_controller(c, kind)` lands and
> returns `EngineError::ControllerKindMismatch` on mismatch. Replay-
> from-disk gap: `Engine::replay_unconfirmed()` walks the persisted log
> and re-feeds foreshadow=true mutations to a freshly wired controller.
> Bench harness gained `--backend memory|file` and `--with-controller`
> flags (closes the `motif-cli bench` file-backed scope item).
>
> **v0.0.2-alpha.5 knowingly accepted (audit pass — explicit accept,
> deferred):** PR #1 finding 5 (unary minus) — trivial to add when the
> expression layer needs arithmetic; no caller demands it yet. WASM
> size jump from alpha.2 (~310 KB / 35% of 2 MiB budget) — staying well
> under budget, microtask-trampoline investigation deferred to v0.0.3+
> when a real bridge is wired. wasm sleep no-op — proper backoff costs
> additional bundle weight; defer until a real wasm bridge needs it.
> `EdgeConfig.foreshadow_eager = false` — buffer-mode is a v0.0.3+
> design item; current `true`-only behaviour matches every existing
> caller. `EdgeConfig.retention_confirmed_secs` — log compaction is
> v0.0.3+ work. `EdgeConfig.schema_cache = "fetch"` — only `"push"` is
> implemented; lazy-fetch is post-v0.0.2. `hoverphone` test profile —
> needs an interleaved-test runner, lands in v0.0.3. `CapabilityConfig`
> auto-discovery — hosts populate manually for v0.0.2; v0.0.3+ picks a
> hardware-probe crate.

---

## Build / distribution

- `[scope]` **WASM target only** — `wasm32-unknown-unknown`. No
  `wasm32-wasip1`, no native targets (`aarch64-apple-ios`,
  `aarch64-linux-android`), no browser-direct path. Host apps embed
  `wasmtime` or similar and pay the runtime tax.
  *Source:* `MOTIF.md` decision 2; `rust-toolchain.toml`.
- `[scope]` **WASM runtime perf tax** — host-runtime execution costs
  ~2–5× vs. native Rust. Acceptable through v0.0.2; native `cdylib`
  evaluation (`aarch64-apple-ios` + `aarch64-linux-android` via
  `uniffi` or direct cdylib) lands in v0.0.3 ("Run on a real device").
  *Source:* `MOTIF.md` decision 2; v0.0.3 milestone.
- `[scope]` **Internally usable only through v0.0.3** — all crates
  are `publish = false`. No crates.io, no semver guarantees.
  **First crates.io publish is v0.0.4** with a pre-v0.1 "fluid API;
  expect breakage" README note. v0.1.0 freezes the public surface
  and adds semver promises.
  *Source:* `Cargo.toml`; `MOTIF.md` decision 14; v0.0.4 + v0.1.0
  milestones.
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
- `[scope]` **`[capability]` reports deterministic facts only.** No
  qualitative labels ("medium", "sufficient") — just numbers and well-
  defined enums. Motif reports facts; controller (or host) decides
  what counts as enough.
  *Source:* `motif.toml.example`; `MOTIF.md` decision 20.
- `[scope]` **`[capability]` probe is primary; TOML is per-field
  override.** v0.0.3-alpha.1 made the probe authoritative — at open
  time motif probes the resources it can verify access to (cores via
  `std::thread::available_parallelism`, RAM via `sysinfo`, disk via
  `Storage::free_space`, arch via `cfg!`) and merges with the host's
  declaration: declared fields win, probe fills the rest. Reason: the
  host's view of the device's resources isn't always motif's view —
  cgroup quotas, sandboxes, or shared multi-tenant hosts can put the
  two out of sync, and decisions made on inflated numbers fail
  downstream. Per-field override lets the host lie when it knows
  better (constraint testing, deliberate budgets) without forcing
  full declaration.
  *Source:* `crates/motif-core/src/capability.rs`; `MOTIF.md` v0.0.3
  milestone.
- `[scope]` **`[edge]` knobs are first-class for both strategies.**
  `edge-is-tiny` (cache + foreshadow) and `edge-is-free` (local
  execution) are both supported via configuration. Motif itself picks
  no strategy.
  *Source:* `motif.toml.example`; `MOTIF.md` decisions 21.
- `[scope]` **`controller.kind` is an opaque string.** v0.0.2-alpha.2
  dropped the `ControllerKind` enum so motif-core never enumerates
  concrete bridges (per the OSS posture, MOTIF.md decision 18). The
  bridge crate (or the host) interprets the string; conventional
  values are documented in `motif.toml.example`.
  *Source:* `crates/motif-core/src/config.rs` (`ControllerConfig`).
- `[scope]` **`controller.kind` validation is opt-in via
  `Engine::with_named_controller`.** v0.0.2-alpha.5 closed the debt:
  hosts that want kind-checking call
  `Engine::with_named_controller(controller, kind)` instead of
  `with_controller(controller)`, and motif returns
  `EngineError::ControllerKindMismatch { declared, wired }` if the
  config's `controller.kind` and the host's wired-kind string disagree.
  Plain `with_controller` remains available for hosts that don't want
  the check (or are testing against multiple controller stand-ins). The
  `kind` string itself is still opaque — motif-core never enumerates
  concrete bridges per MOTIF.md decision 18.
  *Source:* `crates/motif-core/src/engine.rs`
  (`with_named_controller`, `ControllerKindMismatch`).

## Storage (`motif-core::storage`, `motif-core::record`)

- `[scope]` **Greenfield format.** No on-disk compatibility with the
  C++ baseline at `cpp-reference/`. Format version is `1`; any change
  bumps the constant in `storage.rs` and forces re-bootstrap.
  *Source:* `crates/motif-core/src/storage.rs:21-23`;
  `MOTIF.md` decision 10.
- `[debt]` **No CRC on records.** Length-prefixed `bincode` only. Torn
  writes surface as decode errors during recovery; in-place corruption
  in the middle of the log is detected only when read. CRC + improved
  crash-safety semantics land in v0.0.5 ("Hostile-device-aware")
  alongside encryption-at-rest — both are durability concerns and
  share the storage-layer touch.
  *Source:* `crates/motif-core/src/record.rs:1-15`;
  `MOTIF.md` v0.0.5 milestone.
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
  on the target. `MemoryStorage` covers the wasm path through v0.0.2;
  a host-provided storage shim (OPFS / app sandbox / wasm-bindgen-
  driven I/O) is the v0.0.3 "Run on a real device" North Star.
  *Source:* `crates/motif-core/src/storage.rs:5-8`;
  `MOTIF.md` v0.0.3 milestone.
- `[scope]` **Single-namespace ID index.** Nodes and edges share
  `HashMap<String, IndexEntry>`. Globally unique ids today.
  Namespace split (separate node and edge maps) graduates to v0.0.4
  alongside the edge-index work — gigue's B2B target (1M+ edges)
  makes the shared map a real cost, not a "trivial split if it
  bites".
  *Source:* `crates/motif-core/src/engine.rs:55-61`;
  `MOTIF.md` v0.0.4 milestone.
- `[debt]` **No referential integrity on node delete.** Deleting a
  node that has incoming/outgoing edges leaves them dangling; the
  query layer treats them as unreachable. `DETACH DELETE` /
  cascade lands in v0.0.4 alongside the rest of the Cypher surface
  growth.
  *Source:* `crates/motif-core/src/engine.rs:200-211`;
  `MOTIF.md` v0.0.4 milestone.
- `[perf]` **`iter_nodes` / `iter_edges` are O(N) reads.** The id
  index is the only secondary index in v0.0.2; any MATCH without an
  `id()` predicate scans every record. Acceptable through v0.0.3
  ("real device" North Star doesn't grow the graph); **graduates to
  v0.0.4 North-Star tier** because the gigue B2B target (100k–1M+
  edges) makes O(N) edge scans non-negotiable. Edge label + property
  indexes ship with the Cypher surface growth.
  *Source:* `crates/motif-core/src/engine.rs:225-264`;
  `MOTIF.md` v0.0.4 milestone.
- `[debt]` **Property values are limited to 5 scalar variants.**
  `Null`, `Bool`, `I64`, `F64`, `String`. No timestamps, no blobs, no
  lists, no nested structs. Expanded when the query layer needs more.
  *Source:* `crates/motif-core/src/value.rs:1-13`.
- ~~`[debt]` `Storage::truncate` does not enforce
  `new_len >= HEADER_LEN`~~ — **closed in v0.0.2-alpha.5.** Both
  `FileStorage::truncate` and `MemoryStorage::truncate` now return
  `StorageError::TruncateBelowHeader { new_len, header_len }` when a
  caller would otherwise wipe the magic. Two new unit tests cover the
  guard.

## Query (`motif-core::query`)

- `[scope]` **Hand-rolled lexer + parser + interpreter** (not a parser
  combinator framework). No planner, no optimizer — the AST is the
  plan. Reassessed once exit criteria are met.
  *Source:* `crates/motif-core/src/query/mod.rs:1-40`;
  `MOTIF.md` decision 11.
- `[scope]` **Cypher only.** No SurrealQL or other-dialect translators
  inside Motif. Translators (if anyone wants them) live in optional
  bridge / hub crates.
  *Source:* `MOTIF.md` decision 11.
- `[scope]` **Metadata is queryable as data.** v0.0.2 exposes
  foreshadow flags, override history, and the capability profile via
  Cypher tables (`MATCH (n) WHERE n._motif.foreshadow = true RETURN n`)
  rather than a separate state-inspection API. No parallel vocabulary.
  *Source:* `MOTIF.md` decision 19.
- `[scope]` **Single bound variable per statement.** No
  `MATCH (a)-[r]->(b)`, no multi-pattern, no `WITH`, no
  `OPTIONAL MATCH`.
  *Source:* `crates/motif-core/src/query/mod.rs:30-44`,
  `crates/motif-core/src/query/ast.rs:5-7`.
- `[gap]` **Edges aren't queryable from Cypher.** Engine API exposes
  `insert_edge` / `get_edge` / `iter_edges`, but Cypher queries can
  only target nodes through v0.0.2. Edge queries (`MATCH (a)-[r]->(b)`)
  + multi-pattern `MATCH` ship in v0.0.4 ("Real Cypher queries at
  scale") alongside the edge-index work.
  *Source:* `crates/motif-core/src/query/mod.rs:33-35`;
  `MOTIF.md` v0.0.4 milestone.
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
- ~~`[perf]` `extract_id_predicate` only matches a top-level
  `id(n) = X`~~ — **closed in v0.0.2-alpha.5.** A recursive
  `extract_id_in_expr` helper walks `AND` chains so
  `MATCH (n) WHERE id(n) = $x AND n.foo = 1` now hits the index fast
  path with a secondary filter. Bench: file-backed +
  with-controller p50 = 17.67 µs (well within budget).
- `[scope]` **No unary minus expression form. *Knowingly accepted in
  v0.0.2-alpha.5.*** `-10` parses as the literal `Integer(-10)`;
  `-n.balance` does not parse. No caller needs arithmetic on properties
  yet; trivial to add when the expression layer grows in v0.0.3+.
  *Source:* `crates/motif-core/src/query/lexer.rs` (`scan_number`
  entry path). Logged from PR #1 review (finding 5).

## Sync layer (`motif-core::sync`)

- `[scope]` **Bridges architecture: motif-core never bundles a
  controller transport.** The `Controller` trait (lands in v0.0.2) is
  the only seam. Concrete transports — `motif-surreal-bridge`,
  `motif-supabase-bridge`, etc. — ship as optional separate crates
  under `bridges/` (or separate repos). `cargo tree -p motif-core`
  must show no network deps. Likewise host-side event/MCP layers live
  in `hubs/` (`motif-hub` etc.).
  *Source:* `MOTIF.md` decisions 3, 18.
- `[scope]` **Foreshadow semantics.** v0.0.2-alpha.1 added a
  `foreshadow: bool` flag to `Mutation` (default `true` on every fresh
  commit). Server-wins resolution will flip the flag or evict the
  record once the controller flow lands in alpha.2.
  *Source:* `crates/motif-core/src/sync/mutation.rs`;
  `crates/motif-core/src/engine.rs`; `MOTIF.md` decision 5.
- `[gap]` **No concrete bridge crates.** v0.0.2-alpha.2 lands the
  abstract `Controller` trait + thread-per-controller worker (native
  `std::thread` + `mpsc`, wasm `wasm-bindgen-futures::spawn_local` +
  `futures-channel::mpsc::unbounded`); only `InMemoryController` ships
  in motif-core. Concrete transports (e.g. `motif-surreal-bridge`)
  live outside motif-core per MOTIF.md decision 18.
  *Source:* `crates/motif-core/src/sync/controller.rs`;
  `crates/motif-core/src/sync/worker.rs`.
- `[debt]` **WASM size jumped 308 KB in alpha.2. *Knowingly accepted in
  v0.0.2-alpha.5.*** wasm-bindgen-futures + futures-channel +
  futures-util + js-sys add ~310 KB after `wasm-opt -Oz` (713 KB total
  vs alpha.1's 405 KB; 35% of the 2 MiB budget). Plenty of headroom; a
  hand-rolled microtask trampoline (drops futures-util) is v0.0.3+
  work, motivated when a real wasm bridge actually pushes the budget.
  *Source:* `crates/motif-core/src/sync/worker.rs` (wasm impl);
  workspace deps.
- `[scope]` **In-process reconnect: retry-with-backoff on every apply.**
  v0.0.2-alpha.4 grew the worker into a state machine: `connect`
  once, then `apply` with exponential backoff (100ms doubling, capped
  at `EdgeConfig.controller_retry_max_backoff_ms`) on
  `ControllerError::Transient`. Permanent errors short-circuit
  (mutation stays foreshadow=true on disk). Mutations queued during
  the retry window stack up in the channel and drain after recovery —
  ordering preserved.
  *Source:* `crates/motif-core/src/sync/worker.rs`; `MOTIF.md`
  decision 12.
- ~~`[gap]` No replay-from-disk after worker crash~~ — **closed in
  v0.0.2-alpha.5.** `Engine::replay_unconfirmed()` walks the persisted
  log in offset order and re-feeds every foreshadow=true mutation
  through the wired `MutationLog`. No-op when no controller is wired.
  Tests cover the basic case, ordered insert+delete replay, and the
  no-controller no-op. *Source:*
  `crates/motif-core/src/engine.rs` (`replay_unconfirmed`);
  `crates/motif-core/tests/audit_pass.rs`.
- ~~`[gap]` wasm worker doesn't actually sleep on backoff~~
  — **closed in v0.0.3-alpha.2.** `wasm_sleep` now awaits
  `gloo_timers::future::TimeoutFuture::new(ms)` (a `setTimeout`-
  backed Future) instead of `future::ready`. Wasm retry backoff is
  real now; bundle hit was modest (well within the 2 MiB budget).
  *Source:* `crates/motif-core/src/sync/worker.rs` (`wasm_sleep`).
- `[gap]` **`EdgeConfig.foreshadow_eager = false` not yet enforced.
  *Knowingly accepted in v0.0.2-alpha.5.*** Parses the field but always
  behaves as if `true` (apply locally, mark foreshadow=true). No caller
  needs buffer-mode yet; lands when one does. Field stays on
  `EdgeConfig` for forward-compatibility so existing `motif.toml`
  files don't have to change.
  *Source:* `crates/motif-core/src/config.rs` (`EdgeConfig`).
- `[gap]` **`EdgeConfig.retention_confirmed_secs` not yet enforced.
  *Knowingly accepted in v0.0.2-alpha.5.*** Log compaction of
  confirmed mutations is v0.0.3+ work — needs a confirm-acknowledgement
  protocol with the controller that we haven't designed yet. Field is
  parsed and stored for forward-compatibility.
  *Source:* `crates/motif-core/src/config.rs` (`EdgeConfig`).
- `[gap]` **`EdgeConfig.schema_cache = "fetch"` not implemented.
  *Knowingly accepted in v0.0.2-alpha.5.*** Only `"push"` (controller
  pushes; motif caches the latest) works. Lazy-fetch is post-v0.0.2;
  no caller needs it yet.
  *Source:* `crates/motif-core/src/config.rs` (`EdgeConfig`).
- `[scope]` **`hoverphone` test profile deferred to v0.0.3.
  *Knowingly accepted in v0.0.2-alpha.5.*** v0.0.2-alpha.4 stood up
  `default` and `potato` (the latter via a `ThrottledStorage` wrapper
  that sleeps before every storage op). `hoverphone` (artificially
  over-fast / unusual scheduling) needs an interleaved-test runner —
  more invasive than a sleep wrapper — and lands in v0.0.3+.
  v0.0.2 exit criterion 11 carries forward into v0.0.3.
  *Source:* `crates/motif-core/tests/profiles.rs`; `MOTIF.md`
  decision 22.
- ~~`[gap]` `CapabilityConfig` auto-discovery deferred to v0.0.3+~~
  — **closed in v0.0.3-alpha.1.** Native probe via `sysinfo` (RAM)
  + `std::thread::available_parallelism` (cores) + `fs2` via
  `Storage::free_space` (disk) + `cfg!` (arch). Resolved at open
  time; declared TOML fields override probe per-field. wasm32
  probe defers to alpha.3 — alongside the storage shim, the
  `navigator.*` probes land then. *Source:*
  `crates/motif-core/src/capability.rs`.
- `[scope]` **`Spawner` trait is native-only; wasm has no equivalent
  seam.** v0.0.3-alpha.2 added [`crate::sync::Spawner`] +
  [`crate::sync::StdThreadSpawner`] (default) +
  `Engine::with_controller_spawned_by(controller, spawner)` for hosts
  that want to route the controller worker through GCD (iOS) or
  coroutines (Android). Wasm builds skip the trait entirely — the
  host's wasm runtime *is* the spawner, and overriding `spawn_local`
  would mean swapping the microtask queue for something else, which
  isn't a real use case. **Do not introduce target-specific runtime
  assumptions in motif-core** — contributors adding a tokio reactor
  or similar would break this composability.
  *Source:* `crates/motif-core/src/sync/spawner.rs`; `MOTIF.md`
  decision 12.
- `[scope]` **`InMemoryController` is feature-flagged (default-on).**
  Gated behind the `in-memory-controller` Cargo feature. Production
  builds wiring a real `motif-*-bridge` controller can drop it via
  `default-features = false`. Tests, the dev CLI, and hosts that don't
  need controller integration keep the default.
  *Source:* `crates/motif-core/Cargo.toml` (`[features]`);
  `crates/motif-core/src/sync/controller.rs`.
- `[debt]` **Tee fires after the storage append but before the index
  publishes for inserts.** A panic between those two steps would leave
  a record on disk that subsequent recovery picks up, with the
  controller already informed — fine. The opposite order would let a
  reader observe a write the controller doesn't know about, so the
  current order is intentional. Document and lock in with a panic-
  safety test in the next audit pass.
  *Source:* `crates/motif-core/src/engine.rs` (`commit`).
- ~~`[debt]` `MutationLog::record` uses `.expect("poisoned")` on the
  mutex~~ — **closed in v0.0.2-alpha.5.** All locks now go through a
  shared `lock_recover()` helper that calls
  `PoisonError::into_inner` on poison, so a panic in a previous lock
  holder no longer propagates to subsequent callers. Test
  `record_recovers_from_poisoned_mutex` covers the path.
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
  `JSON.parse`. Acceptable through v0.0.2 (one allocation per call);
  binary marshalling lands in v0.0.3 ("Run on a real device") if the
  cold-start measurement harness shows it matters — otherwise pushed
  later.
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
- ~~`[scope]` `motif-cli bench` measures in-memory storage on
  native~~ — **closed in v0.0.2-alpha.5.** `motif bench` now accepts
  `--backend memory|file` and `--with-controller` flags; the
  file-backed path exercises `FileStorage` (fsync per write) and the
  controller-wired path measures the tee + worker channel. Smoke run:
  file-backed + with-controller, 1k nodes, 1k lookups → p50 17.67 µs
  (well within the 50 ms exit criterion).
  *Source:* `crates/motif-cli/src/main.rs` (`run_bench`).
- `[scope]` **Cold-start measurement is `motif bench --cold-start`.**
  v0.0.3-alpha.1 added the harness. Per-iteration: fresh tempdir,
  optional untimed seed, drop-and-reopen, time `Engine::open`. Reports
  p50/p95/p99/mean + the resolved capability profile from the last
  open. Smoke run (file backend, 1k seed, 30 iters): p50 = 1.26 ms.
  No formal cold-start budget yet — set in v0.0.3-alpha.5 audit pass
  once we have iOS / Android numbers to anchor against.
  *Source:* `crates/motif-cli/src/main.rs` (`run_cold_start`);
  `MOTIF.md` v0.0.3 milestone.

## Operations / observability

- `[scope]` **Test profiles: `default` / `potato` / `hoverphone`.**
  Timing-sensitive integration tests (schema race, reconnect replay,
  contention) must pass on all three. `potato` injects artificial
  slowness (low-throughput storage I/O, tight RAM, capped threads);
  `hoverphone` injects artificial speed / unusual timing. Keeps the
  system honest against both edge-is-tiny hardware and edge-is-free
  acceleration.
  *Source:* `MOTIF.md` decision 22; harness lands in v0.0.2-alpha.4.
- `[scope]` **`tracing` only (no `tracing-subscriber` in motif-core).**
  v0.0.3-alpha.1 added the `tracing` crate with instrumentation on
  `Engine::open_with`, the controller worker's connect / apply / retry
  paths, and the resolved capability log. **No subscriber is
  initialized in motif-core** — events are no-ops until the host
  wires its own (`tracing-subscriber` on native, `tracing-wasm` on
  wasm). Counters / metrics still aren't shipped.
  *Source:* `crates/motif-core/src/engine.rs`,
  `crates/motif-core/src/sync/worker.rs`; `MOTIF.md` v0.0.3 milestone.
- `[scope]` **No connection pooling / multi-database.** One `Engine`
  per file, single writer, single reader (the engine takes
  `&mut self` for both). Multi-tenant evaluation lands in v0.0.6
  ("Scale and operate"); the architectural answer may be a host-side
  multiplexer rather than growing `Engine` itself.
  *Source:* `crates/motif-core/src/engine.rs:1-15`;
  `MOTIF.md` v0.0.6 milestone.
- `[scope]` **No backup / restore / migration.** The 4-byte
  format-version field lives in the file header but the engine
  rejects any version other than the current one. Migration design
  lands in v0.0.6 alongside the rest of the operability work.
  *Source:* `crates/motif-core/src/storage.rs:21-23,156-163`;
  `MOTIF.md` v0.0.6 milestone.

## Security

- `[scope]` **No encryption-at-rest through v0.0.4.** Records are
  plaintext `bincode`. Storage layer doesn't bake in plaintext-only
  assumptions. Encryption-at-rest design + impl lands in v0.0.5
  ("Hostile-device-aware") alongside CRC and the controller crypto
  handshake.
  *Source:* `MOTIF.md` decision 13; v0.0.5 milestone.
- `[scope]` **Motif compares opaque tokens, doesn't validate them.**
  Per MOTIF.md decision 4 (refined for v0.0.2), Motif takes opaque
  tokens from the host and opaque keys from the controller and checks
  they match — no JWT validation chain inside motif-core. Real auth
  flow ownership is host (token issuance) + controller (key issuance);
  motif is the comparison point. v0.0.5 documents and tests both
  flows end-to-end with a spec controller.
  *Source:* `crates/motif-core/src/sync/mutation.rs:7-15`;
  `MOTIF.md` decision 4; v0.0.5 milestone.
- `[gap]` **No controller crypto-suite handshake.** Bridges currently
  deliver `Mutation`s with no declaration of how they were
  transmitted. Motif imposes nothing through v0.0.4. v0.0.5 adds a
  `[security]` TOML section (`require_authenticated_channel`,
  `min_aead`, `pq_required`, suite allow-list); bridges advertise
  their suite at `Controller::connect`; motif validates declared-vs-
  policy and surfaces `ControllerSecurityError` on mismatch. PQ-
  forward (Signal-PQXDH-style fail-visible scanner) is plumbing in
  v0.0.5; PQ implementation itself is stretch. Opt-out is named
  `Engine::dangerously_*` — no quiet escape hatches.
  *Source:* `MOTIF.md` v0.0.5 milestone.
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
that review were accepted as deferrable and tagged in this file as the
v0.0.2 backlog. The v0.0.2 audit pass is v0.0.2-alpha.5 (this PR):
findings 4, 6, 7 closed by code fix; finding 5 explicitly accepted.
Plus v0.0.2's own debts: controller-kind validation closed
(`with_named_controller`); replay-from-disk gap closed
(`replay_unconfirmed`); WASM size, wasm sleep, foreshadow_eager,
retention, schema_cache, hoverphone, capability auto-discovery all
explicitly accepted with the reason inline. Same cadence applies
between v0.0.2 and v0.0.3.
