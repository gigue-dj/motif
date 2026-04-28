//! Minimal CLI for development. v0.0.1-alpha.2 only knows how to load and
//! re-emit a `motif.toml`. As subsequent alphas land more functionality,
//! this binary grows the corresponding dev/smoke commands.

use std::path::PathBuf;
use std::process::ExitCode;

use motif_core::MotifConfig;

const USAGE: &str = "usage: motif print-config <path-to-motif.toml>";

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
        "print-config" => {
            let path: PathBuf = match args.next() {
                Some(p) => p.into(),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            match MotifConfig::from_path(&path) {
                Ok(cfg) => {
                    print!("{}", cfg.to_toml_string());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "version" => {
            println!("motif {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unknown command: {cmd}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
