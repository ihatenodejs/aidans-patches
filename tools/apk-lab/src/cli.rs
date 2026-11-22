//! Command-line parsing and output formatting.
//!
//! Only commands that are implemented natively appear in this grammar.
//! Unknown commands and flags fail with exit 2; nothing is delegated to the
//! Python implementation and no placeholder command reports success.
//!
//! `--json` accepts an optional path because the Python CLI does:
//! bare `--json` and `--json -` write to stdout, `--json FILE` writes the same
//! document to `FILE`, and duplicates or empty paths are usage errors.
//!
//! `--workspace-root` is a global option before the command, matching the
//! Python parser. `setup`, `doctor`, `inspect` and `clean` keep the Python
//! stdout/stderr and exit-code contracts.

use crate::error::Error;
use crate::inspection;
use crate::models::{ArtifactInspection, Envelope, ExitCode};
use crate::tools::{
    self, DoctorOptions, DoctorReport, Profile, ToolEnvironment, ToolManager,
    DEFAULT_WORKSPACE_ROOT,
};
use crate::workspace::WorkspaceManager;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const HELP: &str = "\
Native APK Lab host CLI.

Usage:
  apk-lab [--workspace-root DIR] <command> [options]
  apk-lab --help
  apk-lab --version

Commands:
  setup       Download and verify pinned tools
  doctor      Verify host prerequisites and cached tools
  inspect     Inspect an APK/APKM/XAPK/APKS artifact
  clean       Clean validated managed workspaces
  selftest    Validate native CLI, model, and exit-code contracts

Options:
  -h, --help     Print help
  -V, --version  Print version

Run 'apk-lab <command> --help' for command options.
";

const SELFTEST_HELP: &str = "\
Usage: apk-lab selftest [--json [FILE|-]]

Validate the native exit-code table and JSON envelope without external tools.

Options:
  --json [FILE|-]  Emit a JSON envelope; '-' or no value writes stdout
  -h, --help       Print help
";

const SETUP_HELP: &str = "\
Usage: apk-lab setup [--profile analysis|ci]

Download, verify and publish every tool in a profile.

Options:
  --profile PROFILE  Toolchain profile (default: analysis)
  -h, --help         Print help
";

const DOCTOR_HELP: &str = "\
Usage: apk-lab doctor [--ci] [--profile analysis|ci] [--json [FILE|-]]

Verify host prerequisites and cached tools without installing anything.

Options:
  --ci               Include CI credential checks
  --profile PROFILE  Prerequisite profile (default: ci with --ci, else analysis)
  --json [FILE|-]    Emit the doctor report as JSON
  -h, --help         Print help
";

const INSPECT_HELP: &str = "\
Usage: apk-lab inspect <artifact> [--json [FILE|-]]

Inspect an APK, APKM, XAPK or APKS artifact.

Options:
  --json [FILE|-]  Emit the inspection as JSON
  -h, --help       Print help
";

const CLEAN_HELP: &str = "\
Usage: apk-lab [--workspace-root DIR] clean [--run DIR | --package NAME | --stale DAYS] [--dry-run]

Remove only validated managed runs beneath the workspace root.

Options:
  --run DIR        Clean a specific run directory
  --package NAME   Clean all validated runs for a package
  --stale DAYS     Clean runs older than DAYS days
  --dry-run        List candidates without deleting
  -h, --help       Print help
";

const USAGE: &str = "\
Usage: apk-lab [--workspace-root DIR] <command> [options]
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
    let (workspace_root, args) = split_global_options(&args)?;
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
        "setup" => run_setup(rest),
        "doctor" => run_doctor(rest, &workspace_root),
        "inspect" => run_inspect(rest),
        "clean" => run_clean(rest, &workspace_root),
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

/// Extracts the global `--workspace-root` option before the command.
fn split_global_options(args: &[String]) -> Result<(PathBuf, Vec<String>), Error> {
    let mut workspace_root = PathBuf::from(DEFAULT_WORKSPACE_ROOT);
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if let Some(value) = arg.strip_prefix("--workspace-root=") {
            if value.is_empty() {
                write_stderr(USAGE);
                return Err(Error::usage("--workspace-root requires a non-empty path"));
            }
            workspace_root = PathBuf::from(value);
            index += 1;
        } else if arg == "--workspace-root" {
            let Some(value) = args.get(index + 1) else {
                write_stderr(USAGE);
                return Err(Error::usage("--workspace-root requires a path"));
            };
            if value.is_empty() {
                write_stderr(USAGE);
                return Err(Error::usage("--workspace-root requires a non-empty path"));
            }
            workspace_root = PathBuf::from(value);
            index += 2;
        } else {
            break;
        }
    }
    Ok((workspace_root, args[index..].to_vec()))
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
            _ if arg == "--json" || arg.starts_with("--json=") => {
                index = parse_json_occurrence(args, index, &mut json_target, SELFTEST_HELP)?;
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

fn run_setup(args: &[String]) -> Result<ExitCode, Error> {
    let mut profile = Profile::Analysis;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => {
                write_stdout(SETUP_HELP)?;
                return Ok(ExitCode::Success);
            }
            "--profile" => {
                let Some(value) = args.get(index + 1) else {
                    write_stderr(SETUP_HELP);
                    return Err(Error::usage("--profile requires a value"));
                };
                profile = parse_profile(value)?;
                index += 2;
            }
            _ if arg.starts_with("--profile=") => {
                profile = parse_profile(&arg["--profile=".len()..])?;
                index += 1;
            }
            _ if arg.starts_with('-') => {
                write_stderr(SETUP_HELP);
                return Err(Error::usage(format!("unrecognized option '{arg}'")));
            }
            _ => {
                write_stderr(SETUP_HELP);
                return Err(Error::usage(format!("unexpected argument '{arg}'")));
            }
        }
    }

    let manager = ToolManager::with_defaults().map_err(Error::from)?;
    let mut text = format!(
        "Setting up apk-lab toolchain (profile: {})...\n",
        profile.as_str()
    );
    let results = manager.setup_profile(profile).map_err(Error::from)?;
    let mut failed = false;
    for result in &results {
        if result.installed {
            text.push_str(&format!(
                "  [OK] {} v{} installed: {}\n",
                result.name,
                result.configured_version,
                result.executable_path.as_deref().unwrap_or("")
            ));
        } else {
            text.push_str(&format!(
                "  [FAILED] {}: {}\n",
                result.name,
                result.error.as_deref().unwrap_or("unknown error")
            ));
            failed = true;
        }
    }
    write_stdout(&text)?;
    Ok(if failed {
        ExitCode::UsageOrToolError
    } else {
        ExitCode::Success
    })
}

fn parse_profile(value: &str) -> Result<Profile, Error> {
    Profile::parse(value).ok_or_else(|| {
        Error::usage(format!(
            "argument --profile: invalid choice: '{value}' (choose from 'analysis', 'ci')"
        ))
    })
}

fn run_doctor(args: &[String], workspace_root: &Path) -> Result<ExitCode, Error> {
    let mut json_target: Option<JsonTarget> = None;
    let mut ci = false;
    let mut profile: Option<Profile> = None;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => {
                write_stdout(DOCTOR_HELP)?;
                return Ok(ExitCode::Success);
            }
            "--ci" => {
                ci = true;
                index += 1;
            }
            "--profile" => {
                let Some(value) = args.get(index + 1) else {
                    write_stderr(DOCTOR_HELP);
                    return Err(Error::usage("--profile requires a value"));
                };
                profile = Some(parse_profile(value)?);
                index += 2;
            }
            _ if arg.starts_with("--profile=") => {
                profile = Some(parse_profile(&arg["--profile=".len()..])?);
                index += 1;
            }
            _ if arg == "--json" || arg.starts_with("--json=") => {
                index = parse_json_occurrence(args, index, &mut json_target, DOCTOR_HELP)?;
            }
            _ if arg.starts_with('-') => {
                write_stderr(DOCTOR_HELP);
                return Err(Error::usage(format!("unrecognized option '{arg}'")));
            }
            _ => {
                write_stderr(DOCTOR_HELP);
                return Err(Error::usage(format!("unexpected argument '{arg}'")));
            }
        }
    }

    let manager = ToolManager::with_defaults().map_err(Error::from)?;
    let env = ToolEnvironment::from_process(&tools::default_repo_root());
    let report = tools::check_host_prerequisites(
        &manager,
        &env,
        &DoctorOptions {
            ci_mode: ci,
            profile,
            workspace_root: workspace_root.to_path_buf(),
        },
    );

    if let Some(target) = &json_target {
        write_sorted_json(&report, target)?;
    } else {
        write_stdout(&format_doctor_human(&report))?;
    }

    Ok(if report.all_ready {
        ExitCode::Success
    } else {
        ExitCode::UsageOrToolError
    })
}

fn format_doctor_human(report: &DoctorReport) -> String {
    let separator = "=".repeat(60);
    let mut text = String::new();
    text.push_str(&separator);
    text.push('\n');
    text.push_str("APK-LAB DOCTOR REPORT\n");
    text.push_str(&separator);
    text.push('\n');
    text.push_str("Host Prerequisites:\n");
    for prerequisite in &report.prerequisites {
        let status = if prerequisite.satisfied {
            "[OK]"
        } else {
            "[MISSING]"
        };
        let info = prerequisite
            .version_or_path
            .as_deref()
            .map(|value| format!(": {value}"))
            .unwrap_or_default();
        let error = prerequisite
            .error
            .as_deref()
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        text.push_str(&format!("  {status} {}{info}{error}\n", prerequisite.name));
    }

    text.push_str("\nTools in Cache:\n");
    for tool in &report.tools {
        let status = if tool.installed { "[OK]" } else { "[MISSING]" };
        let path = tool
            .executable_path
            .as_deref()
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        let error = tool
            .error
            .as_deref()
            .map(|value| format!(" - {value}"))
            .unwrap_or_default();
        text.push_str(&format!(
            "  {status} {} v{}{path}{error}\n",
            tool.name, tool.configured_version
        ));
    }

    text.push_str(&format!(
        "\nWorkspace writable: {}\n",
        if report.workspace_writable {
            "YES"
        } else {
            "NO"
        }
    ));
    if report.ci_mode {
        text.push_str("\nCI Credentials (presence only):\n");
        for (credential, present) in &report.credentials_present {
            text.push_str(&format!(
                "  {credential}: {}\n",
                if *present { "SET" } else { "UNSET" }
            ));
        }
    }

    text.push_str(&separator);
    text.push('\n');
    if report.all_ready {
        text.push_str("Status: ALL READY\n");
    } else {
        text.push_str("Status: PREREQUISITES OR TOOLS MISSING\n");
    }
    text
}

fn run_inspect(args: &[String]) -> Result<ExitCode, Error> {
    let mut json_target: Option<JsonTarget> = None;
    let mut artifact: Option<String> = None;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => {
                write_stdout(INSPECT_HELP)?;
                return Ok(ExitCode::Success);
            }
            _ if arg == "--json" || arg.starts_with("--json=") => {
                index = parse_json_occurrence(args, index, &mut json_target, INSPECT_HELP)?;
            }
            _ if arg.starts_with('-') => {
                write_stderr(INSPECT_HELP);
                return Err(Error::usage(format!("unrecognized option '{arg}'")));
            }
            _ if artifact.is_none() => {
                artifact = Some(arg.to_string());
                index += 1;
            }
            _ => {
                write_stderr(INSPECT_HELP);
                return Err(Error::usage(format!("unexpected argument '{arg}'")));
            }
        }
    }

    let Some(artifact) = artifact else {
        write_stderr(INSPECT_HELP);
        return Err(Error::usage(
            "the following arguments are required: artifact",
        ));
    };

    match inspection::inspect_artifact(&artifact) {
        Ok(inspection) => {
            if let Some(target) = &json_target {
                write_sorted_json(&inspection, target)?;
            } else {
                write_stdout(&format_inspection_human(&inspection))?;
            }
            Ok(ExitCode::Success)
        }
        Err(error) => {
            if matches!(error, Error::Infrastructure(_) | Error::Io { .. }) {
                write_stderr(&format!("Unexpected error: {error}\n"));
                Ok(ExitCode::InfrastructureFailure)
            } else {
                write_stderr(&format!("Inspection error: {error}\n"));
                Ok(error.exit_code())
            }
        }
    }
}

fn format_inspection_human(inspection: &ArtifactInspection) -> String {
    let mut text = String::new();
    text.push_str(&format!("Artifact:          {}\n", inspection.file_path));
    text.push_str(&format!(
        "Container Type:    {}\n",
        container_type_label(inspection.container_type)
    ));
    text.push_str(&format!(
        "Size:              {} bytes\n",
        format_thousands(inspection.file_size)
    ));
    text.push_str(&format!("SHA-256:           {}\n", inspection.sha256));
    text.push_str(&format!("Package Name:      {}\n", inspection.package_name));
    text.push_str(&format!("Version Name:      {}\n", inspection.version_name));
    text.push_str(&format!("Version Code:      {}\n", inspection.version_code));
    text.push_str(&format!(
        "Min / Target SDK:  {} / {}\n",
        python_sdk(inspection.min_sdk),
        python_sdk(inspection.target_sdk)
    ));
    text.push_str(&format!(
        "Signer SHA-256:    {}\n",
        inspection
            .signing_certificate_sha256
            .as_deref()
            .unwrap_or("None")
    ));
    text.push_str(&format!("Splits:            {}\n", inspection.splits.len()));
    for split in &inspection.splits {
        text.push_str(&format!(
            "  - {} ({} bytes){}\n",
            split.filename,
            format_thousands(split.size),
            if split.is_base { " [BASE]" } else { "" }
        ));
    }
    text.push_str(&format!(
        "DEX Stats:         {} classes, {} methods ({} dex files)\n",
        format_thousands(inspection.dex_classes_count),
        format_thousands(inspection.dex_methods_count),
        inspection.dex_files.len()
    ));
    text.push_str(&format!(
        "Native Libraries:  {}\n",
        inspection.native_libraries.len()
    ));
    text.push_str(&format!("Assets:            {}\n", inspection.assets.len()));
    if !inspection.warnings.is_empty() {
        text.push_str(&format!("Warnings ({}):\n", inspection.warnings.len()));
        for warning in inspection.warnings.iter().take(5) {
            text.push_str(&format!("  ! {warning}\n"));
        }
    }
    text
}

fn container_type_label(container_type: crate::models::ContainerType) -> &'static str {
    match container_type {
        crate::models::ContainerType::Apk => "APK",
        crate::models::ContainerType::Apkm => "APKM",
        crate::models::ContainerType::Xapk => "XAPK",
        crate::models::ContainerType::Apks => "APKS",
        crate::models::ContainerType::Unknown => "UNKNOWN",
    }
}

fn python_sdk(value: Option<u32>) -> String {
    value.map_or_else(|| "None".to_string(), |number| number.to_string())
}

fn format_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn run_clean(args: &[String], workspace_root: &Path) -> Result<ExitCode, Error> {
    let mut run: Option<String> = None;
    let mut package: Option<String> = None;
    let mut stale: Option<f64> = None;
    let mut dry_run = false;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => {
                write_stdout(CLEAN_HELP)?;
                return Ok(ExitCode::Success);
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--run" => {
                run = Some(require_value(args, index, "--run", CLEAN_HELP)?);
                index += 2;
            }
            _ if arg.starts_with("--run=") => {
                run = Some(non_empty_value(
                    &arg["--run=".len()..],
                    "--run",
                    CLEAN_HELP,
                )?);
                index += 1;
            }
            "--package" => {
                package = Some(require_value(args, index, "--package", CLEAN_HELP)?);
                index += 2;
            }
            _ if arg.starts_with("--package=") => {
                package = Some(non_empty_value(
                    &arg["--package=".len()..],
                    "--package",
                    CLEAN_HELP,
                )?);
                index += 1;
            }
            "--stale" => {
                let value = require_value(args, index, "--stale", CLEAN_HELP)?;
                stale = Some(parse_stale(&value)?);
                index += 2;
            }
            _ if arg.starts_with("--stale=") => {
                stale = Some(parse_stale(&arg["--stale=".len()..])?);
                index += 1;
            }
            _ if arg.starts_with('-') => {
                write_stderr(CLEAN_HELP);
                return Err(Error::usage(format!("unrecognized option '{arg}'")));
            }
            _ => {
                write_stderr(CLEAN_HELP);
                return Err(Error::usage(format!("unexpected argument '{arg}'")));
            }
        }
    }

    let manager = WorkspaceManager::new(workspace_root);
    if let Some(run_path) = &run {
        let action = if dry_run { "Would remove" } else { "Removed" };
        if let Err(error) = manager.clean_run(run_path, dry_run) {
            return workspace_clean_error(&error);
        }
        write_stdout(&format!("{action} run: {run_path}\n"))?;
        return Ok(ExitCode::Success);
    }

    if let Some(package_name) = &package {
        let action = if dry_run { "Would remove" } else { "Removed" };
        match manager.clean_package(package_name, dry_run) {
            Ok(runs) => {
                let mut text = format!("{action} {} runs for package {package_name}\n", runs.len());
                for path in &runs {
                    text.push_str(&format!("  - {}\n", path.display()));
                }
                write_stdout(&text)?;
                Ok(ExitCode::Success)
            }
            Err(error) => workspace_clean_error(&error),
        }
    } else if let Some(days) = stale {
        let action = if dry_run { "Would remove" } else { "Removed" };
        match manager.clean_stale(days, dry_run) {
            Ok(runs) => {
                let mut text = format!(
                    "{action} {} stale runs (older than {:?} days)\n",
                    runs.len(),
                    days
                );
                for path in &runs {
                    text.push_str(&format!("  - {}\n", path.display()));
                }
                write_stdout(&text)?;
                Ok(ExitCode::Success)
            }
            Err(error) => workspace_clean_error(&error),
        }
    } else {
        write_stderr("Error: Specify --run, --package, or --stale\n");
        Ok(ExitCode::UsageOrToolError)
    }
}

fn workspace_clean_error(error: &crate::workspace::WorkspaceError) -> Result<ExitCode, Error> {
    write_stderr(&format!("Workspace error: {error}\n"));
    Ok(ExitCode::UsageOrToolError)
}

fn require_value(args: &[String], index: usize, option: &str, help: &str) -> Result<String, Error> {
    let Some(value) = args.get(index + 1) else {
        write_stderr(help);
        return Err(Error::usage(format!("{option} requires a value")));
    };
    non_empty_value(value, option, help)
}

fn non_empty_value(value: &str, option: &str, help: &str) -> Result<String, Error> {
    if value.is_empty() {
        write_stderr(help);
        return Err(Error::usage(format!("{option} requires a non-empty value")));
    }
    Ok(value.to_string())
}

fn parse_stale(value: &str) -> Result<f64, Error> {
    let days: f64 = value
        .parse()
        .map_err(|_| Error::usage(format!("Invalid float value: {value}")))?;
    if days < 0.0 {
        return Err(Error::usage(format!(
            "Stale age must be non-negative, got {value}"
        )));
    }
    Ok(days)
}

/// Parses one `--json` or `--json=...` occurrence and returns the next index.
fn parse_json_occurrence(
    args: &[String],
    index: usize,
    slot: &mut Option<JsonTarget>,
    help: &str,
) -> Result<usize, Error> {
    let arg = args[index].as_str();
    if arg == "--json" {
        if slot.is_some() {
            write_stderr(help);
            return Err(Error::usage("--json may be specified only once"));
        }
        match args.get(index + 1).map(String::as_str) {
            Some("-") => {
                *slot = Some(JsonTarget::Stdout);
                Ok(index + 2)
            }
            Some(value) if !value.starts_with('-') => {
                if value.is_empty() {
                    write_stderr(help);
                    return Err(Error::usage("--json requires a non-empty file path or '-'"));
                }
                *slot = Some(JsonTarget::File(PathBuf::from(value)));
                Ok(index + 2)
            }
            _ => {
                *slot = Some(JsonTarget::Stdout);
                Ok(index + 1)
            }
        }
    } else {
        if slot.is_some() {
            write_stderr(help);
            return Err(Error::usage("--json may be specified only once"));
        }
        let value = &arg["--json=".len()..];
        if value.is_empty() {
            write_stderr(help);
            return Err(Error::usage("--json requires a non-empty file path or '-'"));
        }
        *slot = Some(if value == "-" {
            JsonTarget::Stdout
        } else {
            JsonTarget::File(PathBuf::from(value))
        });
        Ok(index + 1)
    }
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

/// Serializes through `serde_json::Value` so object keys are sorted, matching
/// Python's `json.dumps(..., sort_keys=True)`, then escapes non-ASCII exactly
/// like `ensure_ascii=True` (serde_json always emits raw UTF-8).
fn write_sorted_json<T: Serialize>(value: &T, target: &JsonTarget) -> Result<(), Error> {
    let value = serde_json::to_value(value)
        .map_err(|error| Error::runtime(format!("cannot serialize JSON: {error}")))?;
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| Error::runtime(format!("cannot serialize JSON: {error}")))?;
    let escaped = escape_non_ascii(&text);
    let bytes = escaped.as_bytes();
    match target {
        JsonTarget::Stdout => {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(bytes)
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
            fs::write(path, bytes).map_err(|error| {
                Error::io(
                    format!("cannot write JSON file '{}'", path.display()),
                    error,
                )
            })
        }
    }
}

/// Escapes every non-ASCII scalar as a JSON `\uXXXX` sequence, using a
/// surrogate pair above the BMP. Python's JSON encoder does the same whenever
/// `ensure_ascii` is true (the default).
fn escape_non_ascii(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        let code = character as u32;
        if code < 0x7f {
            escaped.push(character);
        } else if code <= 0xffff {
            escaped.push_str(&format!("\\u{code:04x}"));
        } else {
            let offset = code - 0x1_0000;
            let high = 0xd800 + (offset >> 10);
            let low = 0xdc00 + (offset & 0x3ff);
            escaped.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
        }
    }
    escaped
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
    use crate::tools::ToolCheckResult;

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
        let error = run(&args(&["frobnicate"])).expect_err("unknown command must fail");
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

    #[test]
    fn global_workspace_root_is_parsed_before_the_command() {
        let (root, rest) = split_global_options(&args_strings(&[
            "--workspace-root",
            "space root",
            "clean",
            "--dry-run",
        ]))
        .expect("parse");
        assert_eq!(root, PathBuf::from("space root"));
        assert_eq!(rest, vec!["clean", "--dry-run"]);

        let (root, rest) =
            split_global_options(&args_strings(&["--workspace-root=other", "doctor"]))
                .expect("parse");
        assert_eq!(root, PathBuf::from("other"));
        assert_eq!(rest, vec!["doctor"]);
    }

    fn args_strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn non_ascii_json_escaping_matches_python_ensure_ascii() {
        assert_eq!(escape_non_ascii("plain"), "plain");
        assert_eq!(escape_non_ascii("\u{7f}"), "\\u007f");
        assert_eq!(escape_non_ascii("\u{394}"), "\\u0394");
        assert_eq!(escape_non_ascii("\u{1f600}"), "\\ud83d\\ude00");
        assert_eq!(escape_non_ascii("a\\nb"), "a\\nb");
    }

    #[test]
    fn thousands_format_matches_python() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(2147483647), "2,147,483,647");
    }

    #[test]
    fn tool_check_row_names_match_the_wire_contract() {
        let row = ToolCheckResult {
            name: "jadx".into(),
            configured_version: "1.5.6".into(),
            installed: false,
            actual_version: None,
            executable_path: None,
            error: Some("Not installed in cache".into()),
        };
        let value = serde_json::to_value(&row).expect("serialize");
        assert_eq!(value["configuredVersion"], "1.5.6");
        assert_eq!(value["actualVersion"], serde_json::Value::Null);
        assert_eq!(value["error"], "Not installed in cache");
    }
}
