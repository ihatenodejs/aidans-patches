//! Migration harness for the apk-lab Python oracle and its Rust port.
//!
//! Each subcommand owns its implementation module. The binary is intentionally
//! thin: every command returns `Err` for unsupported or incomplete work so the
//! process can never report success without doing what was requested.
//!
//! The sanitizer tests build a complete benchmark-result fixture with nested
//! `serde_json::json!` literals; expansion depth needs more than the default.
#![recursion_limit = "256"]

pub mod benchmark;
pub mod inventory;
pub mod oracle;
pub mod prerequisites;

use std::io::Write;

/// Runs one migration subcommand. `args` contains the arguments after the
/// subcommand name, matching what the binary received on the command line.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        print_usage();
        return Err("a subcommand is required".to_string());
    };

    match command.as_str() {
        "inventory" => inventory::run(rest),
        "prerequisites" => prerequisites::run(rest),
        "capture" | "verify" => oracle::run(&args[0..]),
        "bench" => benchmark::run(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => {
            print_usage();
            Err(format!("unknown subcommand '{other}'"))
        }
    }
}

fn print_usage() {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "apk-lab-migrate <command> [options]

Commands:
  inventory      Validate the frozen command/test/consumer contract inventory
  prerequisites  Inventory or check migration prerequisites
  capture        Record the independent Python oracle (offline profile)
  verify         Compare two oracle capture documents
  bench          Run process-level baseline benchmarks

Run a command with --help for its options."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(run(&[]).is_err());
    }

    #[test]
    fn unknown_subcommand_is_an_error() {
        assert!(run(&["nope".to_string()]).is_err());
    }
}
