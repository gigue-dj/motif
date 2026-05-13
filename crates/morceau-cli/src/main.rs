//! Minimal CLI for development and v0.0.1 latency validation.
//!
//! Subcommands:
//!   morceau version
//!   morceau print-config <path-to-morceau.toml>
//!   morceau bench [--nodes N] [--lookups M] [--backend memory|file]
//!               [--with-controller]
//!     id-lookup latency harness; v0.0.2-alpha.5 added file-backed and
//!     with-controller modes per MORCEAU.md alpha.5 bench-extension item.
//!   morceau bench --cold-start [--seed N] [--iterations N]
//!               [--backend memory|file]
//!     v0.0.3-alpha.1: Engine::open timing harness. Per-iteration:
//!     create a fresh store, optionally seed it untimed, drop the
//!     engine, then time the reopen. Reports p50/p95/p99/mean +
//!     the resolved capability profile from one of the opens.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use morceau_core::{
    CapabilityConfig, ControllerConfig, Engine, IdentityConfig, InMemoryController, MorceauConfig,
    Node, Params, StorageConfig, Value,
};
use tempfile::TempDir;

const USAGE: &str = "\
usage:
  morceau version
  morceau print-config <path-to-morceau.toml>
  morceau bench [--nodes N] [--lookups M] [--backend memory|file]
              [--with-controller]
  morceau bench --cold-start [--seed N] [--iterations N]
              [--backend memory|file]
  morceau bench --scale [--nodes N] [--edges M] [--lookups L]
              [--backend memory|file]
    v0.0.4-alpha.4: seeds a gigue-target graph (defaults: 10k nodes,
    100k edges across 10 labels) and runs labelled edge MATCH queries
    via the alpha.1 + alpha.4 indexes. Reports p50/p95/p99 + adjacency
    index sanity.";

#[derive(Debug, Clone, Copy)]
enum Backend {
    Memory,
    File,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = match args.next() {
        Some(c) => c,
        None => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match cmd.as_str() {
        "version" => {
            println!("morceau {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "print-config" => match args.next() {
            Some(p) => match MorceauConfig::from_path(PathBuf::from(p)) {
                Ok(cfg) => {
                    print!("{}", cfg.to_toml_string());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            },
            None => {
                eprintln!("{USAGE}");
                ExitCode::from(2)
            }
        },
        "bench" => {
            let mut nodes: Option<usize> = None;
            let mut edges: Option<usize> = None;
            let mut lookups: Option<usize> = None;
            let mut backend = Backend::Memory;
            let mut with_controller = false;
            let mut cold_start = false;
            let mut scale = false;
            let mut seed = 0usize;
            let mut iterations = 50usize;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--nodes" => nodes = Some(parse_usize(&mut args, "--nodes")),
                    "--edges" => edges = Some(parse_usize(&mut args, "--edges")),
                    "--lookups" => lookups = Some(parse_usize(&mut args, "--lookups")),
                    "--backend" => {
                        backend = match args.next().as_deref() {
                            Some("memory") => Backend::Memory,
                            Some("file") => Backend::File,
                            other => {
                                eprintln!("invalid --backend value: {other:?}");
                                eprintln!("{USAGE}");
                                return ExitCode::from(2);
                            }
                        };
                    }
                    "--with-controller" => with_controller = true,
                    "--cold-start" => cold_start = true,
                    "--scale" => scale = true,
                    "--seed" => seed = parse_usize(&mut args, "--seed"),
                    "--iterations" => iterations = parse_usize(&mut args, "--iterations"),
                    other => {
                        eprintln!("unknown bench flag: {other}");
                        eprintln!("{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            if cold_start {
                run_cold_start(seed, iterations, backend)
            } else if scale {
                run_scale_bench(
                    nodes.unwrap_or(10_000),
                    edges.unwrap_or(100_000),
                    lookups.unwrap_or(1_000),
                    backend,
                )
            } else {
                run_bench(
                    nodes.unwrap_or(1_000),
                    lookups.unwrap_or(1_000),
                    backend,
                    with_controller,
                )
            }
        }
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn parse_usize<I: Iterator<Item = String>>(args: &mut I, label: &str) -> usize {
    let raw = args
        .next()
        .unwrap_or_else(|| panic!("missing value for {label}"));
    raw.parse()
        .unwrap_or_else(|_| panic!("{label} must be a non-negative integer"))
}

fn bench_config(path: PathBuf) -> MorceauConfig {
    MorceauConfig {
        identity: IdentityConfig {
            user_id: "bench".into(),
            device_id: "bench".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig { path },
        capability: Default::default(),
        edge: Default::default(),
    }
}

fn run_bench(nodes: usize, lookups: usize, backend: Backend, with_controller: bool) -> ExitCode {
    // Hold the TempDir for the file backend so the file lives until
    // the bench finishes; the variable is otherwise unused.
    let _tmp = match backend {
        Backend::Memory => None,
        Backend::File => Some(TempDir::new().expect("tempdir")),
    };
    let cfg = match backend {
        Backend::Memory => bench_config(PathBuf::from(":memory:")),
        Backend::File => bench_config(_tmp.as_ref().unwrap().path().join("bench.db")),
    };

    let mut engine = match backend {
        Backend::Memory => Engine::open_in_memory(&cfg).expect("open in-memory"),
        Backend::File => Engine::open(&cfg).expect("open file-backed"),
    };
    if with_controller {
        engine = engine.with_controller(InMemoryController::new());
    }

    // Seed.
    let seed_start = Instant::now();
    for i in 0..nodes {
        let id = format!("n{i}");
        engine
            .insert_node(
                Node::new(&id, "Person")
                    .with_property("idx", Value::I64(i as i64))
                    .with_property("name", Value::String(format!("name-{i}"))),
            )
            .expect("insert");
    }
    let seed_ms = seed_start.elapsed().as_secs_f64() * 1000.0;

    // Lookup loop. Uses the id() fast path through the query layer so
    // the measurement covers parser + interpreter + index + storage
    // read.
    let query = "MATCH (n) WHERE id(n) = $x RETURN n";
    let mut samples: Vec<f64> = Vec::with_capacity(lookups);
    let mut params = Params::new();
    for i in 0..lookups {
        let key = format!("n{}", i % nodes);
        params.insert("x".into(), Value::String(key));
        let t = Instant::now();
        let r = engine.query(query, &params).expect("query");
        let elapsed_us = t.elapsed().as_secs_f64() * 1_000_000.0;
        samples.push(elapsed_us);
        debug_assert_eq!(r.rows.len(), 1);
    }

    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| -> f64 {
        let idx = ((samples.len() - 1) as f64 * q).round() as usize;
        samples[idx]
    };
    let p50 = p(0.50);
    let p95 = p(0.95);
    let p99 = p(0.99);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    let backend_label = match backend {
        Backend::Memory => "in-memory",
        Backend::File => "file-backed (fsync per write)",
    };
    let controller_label = if with_controller {
        "in-memory controller (worker thread + channel)"
    } else {
        "no controller"
    };

    println!("morceau bench (native target)");
    println!("  backend ............. {backend_label}");
    println!("  controller .......... {controller_label}");
    println!("  nodes seeded ........ {nodes}");
    println!("  lookups ............. {lookups}");
    println!("  seed time ........... {seed_ms:.2} ms");
    println!("  lookup p50 .......... {p50:.2} µs");
    println!("  lookup p95 .......... {p95:.2} µs");
    println!("  lookup p99 .......... {p99:.2} µs");
    println!("  lookup mean ......... {mean:.2} µs");
    println!();
    println!("v0.0.1 exit criterion 5: p50 < 50 ms (50,000 µs).");
    if p50 > 50_000.0 {
        eprintln!("FAIL: p50 ({p50:.2} µs) exceeds 50 ms budget");
        ExitCode::FAILURE
    } else {
        println!("PASS: p50 well within budget.");
        ExitCode::SUCCESS
    }
}

/// v0.0.3-alpha.1 cold-start measurement.
///
/// Per iteration:
///  1. Create a fresh store (per-iteration tempdir for the file backend;
///     fresh `MemoryStorage::new()` for memory).
///  2. (File backend only) seed `seed` nodes via a temporary engine that
///     is dropped before timing — this leaves a recovery-shaped log on
///     disk for the timed reopen.
///  3. Time `Engine::open` (file) or `Engine::open_in_memory` (memory).
///
/// Reports p50/p95/p99/mean of the open() duration, plus the resolved
/// capability profile from the last open. Memory backend cold-start
/// only measures the empty-store floor — no recovery work happens
/// because each iteration starts fresh.
fn run_cold_start(seed: usize, iterations: usize, backend: Backend) -> ExitCode {
    if iterations == 0 {
        eprintln!("--iterations must be > 0");
        return ExitCode::from(2);
    }
    if matches!(backend, Backend::Memory) && seed > 0 {
        eprintln!(
            "warning: --seed is ignored for --backend memory \
             (no persistence; each iteration opens an empty store)"
        );
    }

    let mut samples: Vec<f64> = Vec::with_capacity(iterations);
    let mut last_capability: Option<CapabilityConfig> = None;

    for _ in 0..iterations {
        let _tmp = match backend {
            Backend::Memory => None,
            Backend::File => Some(TempDir::new().expect("tempdir")),
        };
        let path = match backend {
            Backend::Memory => PathBuf::from(":memory:"),
            Backend::File => _tmp.as_ref().unwrap().path().join("cold.db"),
        };
        let cfg = bench_config(path);

        if matches!(backend, Backend::File) && seed > 0 {
            let mut engine = Engine::open(&cfg).expect("open for seed");
            for i in 0..seed {
                engine
                    .insert_node(Node::new(format!("n{i}"), "Person"))
                    .expect("insert");
            }
            drop(engine);
        }

        let t = Instant::now();
        let engine = match backend {
            Backend::Memory => Engine::open_in_memory(&cfg).expect("open in-memory"),
            Backend::File => Engine::open(&cfg).expect("open file-backed"),
        };
        let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
        last_capability = Some(engine.capability().clone());
        drop(engine);
        samples.push(elapsed_ms);
    }

    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| -> f64 {
        let idx = ((samples.len() - 1) as f64 * q).round() as usize;
        samples[idx]
    };
    let p50 = p(0.50);
    let p95 = p(0.95);
    let p99 = p(0.99);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    let backend_label = match backend {
        Backend::Memory => "in-memory (empty-store floor)",
        Backend::File => "file-backed (seeded then reopened)",
    };

    println!("morceau bench --cold-start (native target)");
    println!("  backend ............. {backend_label}");
    println!("  seed records ........ {seed}");
    println!("  iterations .......... {iterations}");
    println!("  open p50 ............ {p50:.3} ms");
    println!("  open p95 ............ {p95:.3} ms");
    println!("  open p99 ............ {p99:.3} ms");
    println!("  open mean ........... {mean:.3} ms");
    if let Some(cap) = last_capability {
        println!();
        println!("resolved capability (declared overrides probe):");
        println!("  ram_mb .............. {:?}", cap.ram_mb);
        println!("  cpu_cores ........... {:?}", cap.cpu_cores);
        println!("  storage_mb .......... {:?}", cap.storage_mb);
        println!("  arch ................ {:?}", cap.arch);
        println!("  gpu_present ......... {:?}", cap.gpu_present);
    }
    ExitCode::SUCCESS
}

/// Seed a gigue-target graph (10k nodes / 100k edges / 10 labels by
/// default), time `MATCH (a)-[r:KNOWS]->(b) WHERE id(a) = $x`,
/// report p50/p95/p99/mean.
fn run_scale_bench(nodes: usize, edges: usize, lookups: usize, backend: Backend) -> ExitCode {
    use morceau_core::{Edge, Engine, Node};

    let _tmp = match backend {
        Backend::Memory => None,
        Backend::File => Some(TempDir::new().expect("tempdir")),
    };
    let cfg = match backend {
        Backend::Memory => bench_config(PathBuf::from(":memory:")),
        Backend::File => bench_config(_tmp.as_ref().unwrap().path().join("scale.db")),
    };

    let mut engine = match backend {
        Backend::Memory => Engine::open_in_memory(&cfg).expect("open in-memory"),
        Backend::File => Engine::open(&cfg).expect("open file-backed"),
    };

    // Seed nodes.
    let seed_start = Instant::now();
    for i in 0..nodes {
        engine
            .insert_node(Node::new(format!("n{i}"), "Person"))
            .expect("insert node");
    }
    let nodes_ms = seed_start.elapsed().as_secs_f64() * 1000.0;

    // Seed edges across 10 labels, source uniformly across nodes,
    // target rotated +k so adjacency is non-trivial.
    let labels = [
        "KNOWS",
        "FOLLOWS",
        "WORKED_WITH",
        "MANAGED",
        "MENTORED",
        "INVITED",
        "BLOCKED",
        "STARRED",
        "FORKED",
        "PR",
    ];
    let edges_start = Instant::now();
    for i in 0..edges {
        let from = i % nodes;
        let to = (i + 17) % nodes;
        let label = labels[i % labels.len()];
        engine
            .insert_edge(Edge::new(
                format!("e{i}"),
                label,
                format!("n{from}"),
                format!("n{to}"),
            ))
            .expect("insert edge");
    }
    let edges_ms = edges_start.elapsed().as_secs_f64() * 1000.0;

    // Run labelled edge MATCH queries. Each lookup picks a random
    // source via `i % nodes` and asks for one of the 10 labels.
    let query = "MATCH (a)-[r:KNOWS]->(b) WHERE id(a) = $x RETURN r";
    let mut samples: Vec<f64> = Vec::with_capacity(lookups);
    let mut params = Params::new();
    for i in 0..lookups {
        params.insert("x".into(), Value::String(format!("n{}", i % nodes)));
        let t = Instant::now();
        let _ = engine.query(query, &params).expect("query");
        samples.push(t.elapsed().as_secs_f64() * 1_000_000.0);
    }

    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| -> f64 {
        let idx = ((samples.len() - 1) as f64 * q).round() as usize;
        samples[idx]
    };
    let p50 = p(0.50);
    let p95 = p(0.95);
    let p99 = p(0.99);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    let backend_label = match backend {
        Backend::Memory => "in-memory",
        Backend::File => "file-backed (fsync per write)",
    };

    println!("morceau bench --scale (native target)");
    println!("  backend ............. {backend_label}");
    println!("  nodes seeded ........ {nodes}");
    println!(
        "  edges seeded ........ {edges} across {} labels",
        labels.len()
    );
    println!("  node seed time ...... {nodes_ms:.2} ms");
    println!("  edge seed time ...... {edges_ms:.2} ms");
    println!("  match lookups ....... {lookups} of `MATCH (a)-[r:KNOWS]->(b) WHERE id(a) = $x`");
    println!("  match p50 ........... {p50:.2} µs");
    println!("  match p95 ........... {p95:.2} µs");
    println!("  match p99 ........... {p99:.2} µs");
    println!("  match mean .......... {mean:.2} µs");
    ExitCode::SUCCESS
}
