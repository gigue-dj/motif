# Motif

**Motif** is a tiny, mobile-first follower graph store. It is a fork of [Kuzu](https://github.com/kuzudb/kuzu/), stripped down for embedded use on phones and small edge devices, with integrity outsourced to an upstream controller database.

> **Status:** v0.0.1-alpha — pre-functional. The codebase is the upstream Kuzu engine with extensions, language bindings, and benchmarks pruned. Sync layer is a stub. Not usable yet. See [`MOTIF.md`](./MOTIF.md) for the v0.0.1 plan and architectural decisions.

## Architecture in one paragraph

A host application (mobile app or edge agent) embeds Motif as a WASM module. Motif holds a small local property graph for query speed and offline operation. Every committed mutation is teed to a `ControllerClient` queue; the controller (SurrealDB today, a custom Nebula-class controller later) is the source of truth for schema, integrity, and conflict resolution. The controller is **server-wins**: it can override or sunset any local change. Motif is a **follower**, not an authority.

## Design constraints

- **Tiny binary.** Target <5 MB stripped WASM.
- **Tiny on-disk footprint.** Single-file storage, no extensions.
- **Fast I/O.** <50 ms p50 single-key read on mid-tier mobile.
- **Offline-first.** Reads return stale-with-flag on miss. Writes queue locally and sync when connectivity returns.
- **Hostile-device-aware.** Per-user + per-device auth. Storage layer keeps encryption-at-rest as a future option.

## Build (development)

WASM is the only target. iOS, Android, and edge devices all run the WASM artifact.

```bash
# Requires emsdk
make wasm
cd tools/wasm && npm install && npm run build
```

## Provenance

Motif is derived from Kuzu (MIT). Upstream Kuzu has been archived. Original `kuzu::` C++ namespace and many internal artifact names are retained in v0.0.1 — namespace rename is scheduled for v0.0.2.

## License

MIT — see [LICENSE](./LICENSE).
