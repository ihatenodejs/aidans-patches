//! Command-line parsing and output formatting.
//!
//! Only commands that are implemented natively appear in this grammar.
//! Unknown commands and flags fail with exit 2; nothing is delegated to the
//! Python implementation and no placeholder command reports success.
//!
//! `--json` accepts an optional path because the Python CLI does:
//! bare `--json` and `--json -` write to stdout, `--json FILE` writes the same
//! document to `FILE`, and duplicates or empty paths are usage errors.

use crate::error::Error;
use crate::models::{Envelope, ExitCode};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

const HELP: &str = "\
Native APK Lab host CLI.

Usage:
  apk-lab <command> [options]
  apk-lab --help
  apk-lab --version

Commands:
  selftest    Validate native CLI, model, and exit-code contracts

Options:
  -h, --help     Print help
  -V, --version  Print version

Run 'apk-lab selftest --help' for command options.
";

const SELFTEST_HELP: &str = "\
Usage: apk-lab selftest [--json [FILE|-]]

Validate the native exit-code table and JSON envelope without external tools.

Options:
  --json [FILE|-]  Emit a JSON envelope; '-' or no value writes stdout
  -h, --help       Print help
";

const USAGE: &str = "\
Usage: apk-lab <command> [options]
       apk-lab --help
       apk-lab --version
";

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonTarget {
    Stdout,
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SelftestCheck {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "passed")]
    passed: bool,
    #[serde(rename = "detail")]
    detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SelftestReport {
    #[serde(rename = "checks")]
    checks: Vec<SelftestCheck>,
}

/// Runs the native CLI. Arguments are what followed the program name.
///
/// Output is written through fallible helpers so a closed pipe can be mapped to
/// success at the binary boundary instead of panicking (`println!` panics).
pub fn run(raw_args: &[OsString]) -> Result<ExitCode, Error> {
    let args = decode_args(raw_args)?;
    let Some((command, rest)) = args.split_first() else {
        write_stderr(USAGE);
        return Err(Error::usage("a command is required"));
    };

    match command.as_str() {
        "-h" | "--help" => {
            write_stdout(HELP)?;
            Ok(ExitCode::Success)
        }
        "-V" | "--version" => {
            write_stdout(&format!("apk-lab {}\n", env!("CARGO_PKG_VERSION")))?;
            Ok(ExitCode::Success)
        }
        "selftest" => run_selftest(rest),
        other if other.starts_with('-') => {
            write_stderr(USAGE);
            Err(Error::usage(format!("unrecognized option '{other}'")))
        }
        other => {
            write_stderr(USAGE);
            Err(Error::usage(format!("unknown command '{other}'")))
        }
    }
}

fn decode_args(raw_args: &[OsString]) -> Result<Vec<String>, Error> {
    let mut args = Vec::with_capacity(raw_args.len());
    for arg in raw_args {
        match arg.to_str() {
            Some(text) => args.push(text.to_owned()),
            None => return Err(Error::usage("arguments must be valid UTF-8")),
        }
    }
    Ok(args)
}

fn run_selftest(args: &[String]) -> Result<ExitCode, Error> {
    let mut json_target: Option<JsonTarget> = None;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => {
                write_stdout(SELFTEST_HELP)?;
                return Ok(ExitCode::Success);
            }
            "--json" => {
                if json_target.is_some() {
                    write_stderr(SELFTEST_HELP);
                    return Err(Error::usage("--json may be specified only once"));
                }
                match args.get(index + 1).map(String::as_str) {
                    Some("-") => {
                        json_target = Some(JsonTarget::Stdout);
                        index += 2;
                    }
                    Some(value) if !value.starts_with('-') => {
                        if value.is_empty() {
                            write_stderr(SELFTEST_HELP);
                            return Err(Error::usage(
                                "--json requires a non-empty file path or '-'",
                            ));
                        }
                        json_target = Some(JsonTarget::File(PathBuf::from(value)));
                        index += 2;
                    }
                    _ => {
                        json_target = Some(JsonTarget::Stdout);
                        index += 1;
                    }
                }
            }
            _ if arg.starts_with("--json=") => {
                if json_target.is_some() {
                    write_stderr(SELFTEST_HELP);
                    return Err(Error::usage("--json may be specified only once"));
                }
                let value = &arg["--json=".len()..];
                if value.is_empty() {
                    write_stderr(SELFTEST_HELP);
                    return Err(Error::usage("--json requires a non-empty file path or '-'"));
                }
                json_target = Some(if value == "-" {
                    JsonTarget::Stdout
                } else {
                    JsonTarget::File(PathBuf::from(value))
                });
                index += 1;
            }
            _ if arg.starts_with('-') => {
                write_stderr(SELFTEST_HELP);
                return Err(Error::usage(format!("unrecognized option '{arg}'")));
            }
            _ => {
                write_stderr(SELFTEST_HELP);
                return Err(Error::usage(format!("unexpected argument '{arg}'")));
            }
        }
    }

    let report = build_selftest_report();
    let all_passed = report.checks.iter().all(|check| check.passed);
    if !all_passed {
        return Err(Error::runtime("native selftest failed"));
    }

    match json_target {
        Some(target) => {
            let envelope = Envelope::success("selftest", report);
            write_json(&envelope, &target)?;
        }
        None => {
            let mut text = String::from("apk-lab selftest\n");
            for check in &report.checks {
                let status = if check.passed { "ok" } else { "FAIL" };
                text.push_str(&format!("  [{status}] {}: {}\n", check.name, check.detail));
            }
            text.push_str("Status: OK\n");
            write_stdout(&text)?;
        }
    }

    Ok(ExitCode::Success)
}

fn build_selftest_report() -> SelftestReport {
    let exit_codes_ok = ExitCode::ALL
        .iter()
        .all(|code| ExitCode::from_u8(code.as_u8()) == Some(*code));
    let unknown_exit_rejected = ExitCode::from_u8(6).is_none();

    let envelope_ok = envelope_round_trips();

    SelftestReport {
        checks: vec![
            SelftestCheck {
                name: "exitCodes".to_owned(),
                passed: exit_codes_ok && unknown_exit_rejected,
                detail: "six Python exit classes round-trip; unknown values rejected".to_owned(),
            },
            SelftestCheck {
                name: "models".to_owned(),
                passed: envelope_ok,
                detail: "JSON envelope round-trips with stable renames".to_owned(),
            },
        ],
    }
}

fn envelope_round_trips() -> bool {
    let sample = Envelope::success("selftest", SelftestReport { checks: Vec::new() });
    match serde_json::to_string(&sample) {
        Ok(text) => match serde_json::from_str::<Envelope<SelftestReport>>(&text) {
            Ok(decoded) => decoded == sample,
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn write_json<T: Serialize>(value: &T, target: &JsonTarget) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| Error::runtime(format!("cannot serialize JSON: {error}")))?;

    match target {
        JsonTarget::Stdout => {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(&bytes)
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush())
                .map_err(|error| Error::io("failed to write JSON to stdout", error))
        }
        JsonTarget::File(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).map_err(|error| {
                        Error::io(
                            format!("cannot create directory '{}'", parent.display()),
                            error,
                        )
                    })?;
                }
            }
            fs::write(path, &bytes).map_err(|error| {
                Error::io(
                    format!("cannot write JSON file '{}'", path.display()),
                    error,
                )
            })
        }
    }
}

fn write_stdout(text: &str) -> Result<(), Error> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| Error::io("failed to write stdout", error))
}

fn write_stderr(text: &str) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
    let _ = stderr.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn missing_command_is_a_usage_error() {
        let error = run(&[]).expect_err("missing command must fail");
        assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
    }

    #[test]
    fn unknown_command_is_a_usage_error() {
        let error = run(&args(&["inspect"])).expect_err("unknown command must fail");
        assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
    }

    #[test]
    fn duplicate_json_flag_is_a_usage_error() {
        let error = run(&args(&["selftest", "--json", "--json"])).expect_err("duplicate --json");
        assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
    }

    #[test]
    fn empty_json_path_is_a_usage_error() {
        let error = run(&args(&["selftest", "--json="])).expect_err("empty --json path");
        assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
    }

    #[test]
    fn non_utf8_arguments_are_a_usage_error() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let raw = OsString::from_vec(vec![0xff, 0xfe]);
            let error = run(&[raw]).expect_err("non-UTF-8 argument");
            assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
        }
    }
}
