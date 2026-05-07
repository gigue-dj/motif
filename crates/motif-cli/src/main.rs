//! Minimal CLI for development and v0.0.1 latency validation.
//!
//! Subcommands:
//!   motif version
//!   motif print-config <path-to-motif.toml>
//!   motif bench [--nodes N] [--lookups M]   — id-lookup latency harness

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use motif_core::{
    ControllerConfig, Engine, IdentityConfig, MotifConfig, Node, Params, StorageConfig, Value,
};

const USAGE: &str = "\
usage:
  motif version
  motif print-config <path-to-motif.toml>
  motif bench [--nodes N] [--lookups M]";

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
            println!("motif {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "print-config" => match args.next() {
            Some(p) => match MotifConfig::from_path(PathBuf::from(p)) {
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
            let mut nodes = 1_000usize;
            let mut lookups = 1_000usize;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--nodes" => {
                        nodes = parse_usize(&mut args, "--nodes");
                    }
                    "--lookups" => {
                        lookups = parse_usize(&mut args, "--lookups");
                    }
                    other => {
                        eprintln!("unknown bench flag: {other}");
                        eprintln!("{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            run_bench(nodes, lookups)
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

fn bench_config() -> MotifConfig {
    MotifConfig {
        identity: IdentityConfig {
            user_id: "bench".into(),
            device_id: "bench".into(),
        },
        controller: ControllerConfig {
            kind: "in-memory".into(),
        },
        storage: StorageConfig {
            path: PathBuf::from(":memory:"),
        },
        capability: Default::default(),
        edge: Default::default(),
    }
}

fn run_bench(nodes: usize, lookups: usize) -> ExitCode {
    let cfg = bench_config();
    let mut engine = Engine::open_in_memory(&cfg).expect("open in-memory");

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

    // Lookup loop. Use the id() fast path through the query layer so the
    // measurement covers the parser + interpreter + index + storage read.
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

    println!("motif bench (in-memory storage, native target)");
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
