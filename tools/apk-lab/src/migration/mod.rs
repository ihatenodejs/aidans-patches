//! Migration harness for the apk-lab Python oracle and its Rust port.
//!
//! Each subcommand owns its implementation module. The binary is intentionally
//! thin: every command returns `Err` for unsupported or incomplete work so the
//! process can never report success without doing what was requested.
//!
//! The sanitizer tests build a complete benchmark-result fixture with nested
//! `serde_json::json!` literals; expansion depth needs more than the default.
//! The crate-level limit lives in `src/lib.rs`.

pub mod acquisition;
pub mod benchmark;
pub mod compare;
pub mod icons;
pub mod inventory;
pub mod oracle;
pub mod parity;
pub mod play;
pub mod prerequisites;

use std::io::Write;

mod preflight {
    use super::icons;

    /// Routes `preflight <area> ...`.
    pub fn run(args: &[String]) -> Result<(), String> {
        let Some((subcommand, rest)) = args.split_first() else {
            print_usage();
            return Err("preflight requires a subcommand".to_string());
        };
        match subcommand.as_str() {
            "acquisition" => super::acquisition::run(rest),
            "icons" => icons::run(rest),
            "-h" | "--help" | "help" => {
                print_usage();
                Ok(())
            }
            other => {
                print_usage();
                Err(format!("unknown preflight subcommand '{other}'"))
            }
        }
    }

    fn print_usage() {
        println!(
            "apk-lab-migrate preflight <area> [options]\n\
             \n\
             Areas:\n\
             \x20 acquisition  Validate native Play delivery evidence and capability coverage\n\
             \x20 icons        Prove native icon pixels against the pinned Pillow baseline\n\
             \x20              (capture, --check, or --sanitize a public summary)"
        );
    }
}

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
        "preflight" => preflight::run(rest),
        "play" => play::run(rest),
        "capture" | "verify" => oracle::run(&args[0..]),
        "parity" => parity::run(rest),
        // Internal process boundary for the acquisition-ci parity scenarios.
        // It is intentionally absent from the public help text.
        "acquisition-ci-scenario" => parity::run_acquisition_ci_scenario_cli(rest),
        "bench" => benchmark::run(rest),
        "compare" => compare::run(rest),
        "report-check" => compare::report_check(rest),
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
  preflight      Prove asset-pipeline compatibility (icons)
  play           Probe native Play delivery (development-only)
  capture        Record the independent Python oracle (offline profile)
  verify         Compare two oracle capture documents
  parity         Compare native core/analysis/patching-device with the Python oracle
  bench          Run process-level baseline or paired benchmarks
  compare        Recompute paired statistics and apply the performance policy
  report-check   Validate TO-RUST.md against stored data and charts

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

    #[test]
    fn preflight_icons_help_is_dispatched() {
        assert!(run(&[
            "preflight".to_string(),
            "icons".to_string(),
            "--help".to_string(),
        ])
        .is_ok());
        assert!(run(&["preflight".to_string()]).is_err());
        assert!(run(&["preflight".to_string(), "unknown".to_string(),]).is_err());
    }
}
