//! Real-executable contracts for the native `apk-lab` binary and process
//! adapter. These tests exercise argv boundaries without a shell, stdout
//! purity, the JSON flag forms, exit classes, and closed pipes.

use apk_lab::error::Error;
use apk_lab::models::ExitCode;
use apk_lab::process::{self, OutputMode, ProcessSpec};
use apk_lab::workspace::{RunRequest, WorkspaceManager};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn run_bin(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apk-lab"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("failed to execute apk-lab")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "apk-lab failed: status={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be pure JSON")
}

#[test]
fn help_exits_zero_and_uses_stdout() {
    for flag in ["--help", "-h"] {
        let output = run_bin(&[flag]);
        assert_success(&output);
        assert!(output.stderr.is_empty(), "help must not write stderr");
        let text = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(text.contains("Usage:"), "help must show usage");
        assert!(text.contains("selftest"), "help must list selftest");
        for command in ["setup", "doctor", "inspect", "clean"] {
            assert!(text.contains(command), "help must list {command}");
        }
    }
}

#[test]
fn version_exits_zero_and_uses_stdout() {
    let output = run_bin(&["--version"]);
    assert_success(&output);
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("apk-lab {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_exits_two_without_success_output() {
    let output = run_bin(&["frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "unknown command must not print success"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown command"), "stderr was {stderr}");
    assert!(stderr.contains("Usage:"), "stderr must include usage");
}

#[test]
fn unknown_flag_exits_two() {
    let output = run_bin(&["selftest", "--definitely-not-a-flag"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized option"));
}

#[test]
fn no_arguments_exits_two() {
    let output = run_bin(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}

#[test]
fn selftest_human_output_exits_zero() {
    let output = run_bin(&["selftest"]);
    assert_success(&output);
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).expect("selftest text is UTF-8");
    assert!(text.contains("Status: OK"));
}

#[test]
fn bare_json_writes_one_pure_json_document_to_stdout() {
    let output = run_bin(&["selftest", "--json"]);
    assert_success(&output);
    assert!(output.stderr.is_empty(), "JSON mode must not write stderr");

    let value = parse_json_stdout(&output);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "selftest");
    assert_eq!(value["success"], true);
    assert_eq!(value["exitCode"], 0);
    assert!(value["data"]["checks"].is_array());
    assert_eq!(output.stdout.last(), Some(&b'\n'));
}

#[test]
fn json_dash_is_identical_to_bare_json() {
    let bare = run_bin(&["selftest", "--json"]);
    let dash = run_bin(&["selftest", "--json", "-"]);
    assert_success(&dash);
    assert!(dash.stderr.is_empty());
    assert_eq!(bare.stdout, dash.stdout);
}

#[test]
fn json_file_mode_writes_the_document_and_keeps_stdout_empty() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("nested").join("report.json");
    let path_arg = path.to_string_lossy().into_owned();

    let output = run_bin(&["selftest", "--json", &path_arg]);
    assert_success(&output);
    assert!(output.stdout.is_empty(), "file mode must not write stdout");
    assert!(output.stderr.is_empty());

    let bytes = std::fs::read(&path).expect("JSON report file");
    let value: Value = serde_json::from_slice(&bytes).expect("file must contain pure JSON");
    assert_eq!(value["command"], "selftest");
    assert_eq!(value["exitCode"], 0);
}

#[test]
fn json_equals_form_supports_paths_with_spaces() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("report with spaces.json");
    let arg = format!("--json={}", path.display());

    let output = run_bin(&["selftest", &arg]);
    assert_success(&output);
    assert!(output.stdout.is_empty());
    let value: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("JSON file")).expect("pure JSON");
    assert_eq!(value["success"], true);
}

#[test]
fn json_option_value_with_spaces_is_one_argument() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("space value.json");
    let path_arg = path.to_string_lossy().into_owned();

    let output = run_bin(&["selftest", "--json", &path_arg]);
    assert_success(&output);
    assert!(std::fs::read(&path).is_ok());
}

#[test]
fn duplicate_json_flags_are_rejected() {
    let output = run_bin(&["selftest", "--json", "--json", "-"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only once"));
}

#[test]
fn conflicting_json_forms_are_rejected() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("report.json");
    let path_arg = path.to_string_lossy().into_owned();

    let output = run_bin(&["selftest", "--json", "-", "--json", &path_arg]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!path.exists(), "rejected parse must not write a file");
}

#[test]
fn empty_json_value_is_malformed() {
    let output = run_bin(&["selftest", "--json="]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-empty"));
}

#[test]
fn closed_stdout_pipe_exits_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_apk-lab"))
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn apk-lab");
    drop(child.stdout.take());
    let status = child.wait().expect("wait apk-lab");
    assert_eq!(status.code(), Some(0), "closed pipe must not panic or fail");
}

#[test]
fn process_argv_keeps_spaces_inside_single_arguments() {
    let spec = ProcessSpec::new("/bin/echo").args(["a b", "c"]);
    let output = process::run(&spec).expect("echo runs");
    assert!(output.success());
    assert_eq!(output.stdout.text_lossy(), "a b c\n");
}

#[test]
fn process_reports_unavailable_executable_as_usage_error() {
    let spec = ProcessSpec::new("/nonexistent/apk-lab-contract-binary");
    let error = process::run(&spec).expect_err("missing executable must fail");
    assert_eq!(error.exit_code(), ExitCode::UsageOrToolError);
    assert!(error.message().contains("executable not found"));
}

#[test]
fn process_preserves_non_zero_exit_status() {
    let spec = ProcessSpec::new("/bin/sh").args(["-c", "exit 7"]);
    let output = process::run(&spec).expect("sh runs");
    assert!(!output.success());
    assert_eq!(output.exit_code(), Some(7));
    assert!(!output.timed_out);
}

#[test]
fn process_timeout_kills_the_child() {
    let started = Instant::now();
    let spec = ProcessSpec::new("/bin/sleep")
        .args(["30"])
        .timeout(Duration::from_millis(100));
    let output = process::run(&spec).expect("sleep starts");
    assert!(output.timed_out, "child must be reported as timed out");
    assert!(!output.success());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "timeout must not wait for the child"
    );
}

#[test]
fn process_output_capture_is_bounded() {
    let spec = ProcessSpec::new("/bin/sh")
        .args([
            "-c",
            "i=0; while [ $i -lt 2000 ]; do printf 0123456789; i=$((i+1)); done",
        ])
        .max_output_bytes(64);
    let output = process::run(&spec).expect("sh runs");
    assert_eq!(output.stdout.bytes.len(), 64);
    assert!(output.stdout.truncated);
}

#[test]
fn process_diagnostics_redact_configured_secrets() {
    let spec = ProcessSpec::new("/bin/sh")
        .args(["-c", "printf 'token=s3cr3t-value' >&2"])
        .secret("s3cr3t-value");
    let output = process::run(&spec).expect("sh runs");
    let diagnostics = output.diagnostics(&spec);
    assert!(!diagnostics.contains("s3cr3t-value"));
    assert!(diagnostics.contains("[REDACTED]"));

    let spec = spec.stdout(OutputMode::Null);
    assert!(!spec.describe_command().contains("s3cr3t-value"));
}

#[test]
fn six_exit_classes_round_trip() {
    for raw in 0u8..=5 {
        let code = ExitCode::from_u8(raw).expect("known exit class");
        assert_eq!(code.as_u8(), raw);
        let text = serde_json::to_string(&code).expect("serialize");
        let decoded: ExitCode = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(decoded, code);
    }
    assert!(ExitCode::from_u8(6).is_none());

    let cases: [(Error, ExitCode); 5] = [
        (Error::compatibility("x"), ExitCode::CompatibilityFailure),
        (Error::usage("x"), ExitCode::UsageOrToolError),
        (Error::invalid_artifact("x"), ExitCode::InvalidArtifact),
        (Error::infrastructure("x"), ExitCode::InfrastructureFailure),
        (Error::runtime("x"), ExitCode::RuntimeFailure),
    ];
    for (error, expected) in cases {
        assert_eq!(error.exit_code(), expected);
    }
    assert_eq!(ExitCode::Success.as_u8(), 0);
}

#[test]
fn non_utf8_arguments_exit_two() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![b'-', b'-', 0xff]);
        let output = Command::new(env!("CARGO_BIN_EXE_apk-lab"))
            .arg(raw)
            .stdin(Stdio::null())
            .output()
            .expect("run apk-lab");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

fn create_run(root: &Path, package: &str, version_code: u64, config: &Value) -> PathBuf {
    let manager = WorkspaceManager::new(root);
    let tools = BTreeMap::new();
    let input_sha256 = "a".repeat(64);
    let request = RunRequest {
        package_name: package,
        version_code,
        input_sha256: &input_sha256,
        input_path: Path::new("input.apk"),
        command: "inspect",
        config,
        tool_versions: &tools,
    };
    manager.create_run_dir(&request).expect("create run")
}

fn run_clean(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apk-lab"));
    command
        .arg("--workspace-root")
        .arg(root)
        .arg("clean")
        .args(args)
        .stdin(Stdio::null());
    command.output().expect("run apk-lab clean")
}

#[test]
fn clean_dry_run_validates_without_deleting() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let run = create_run(&root, "com.example.app", 1, &json!({"kind": "contract"}));
    let run_arg = run.to_string_lossy().into_owned();

    let output = run_clean(&root, &["--run", &run_arg, "--dry-run"]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(text, format!("Would remove run: {run_arg}\n"));
    assert!(output.stderr.is_empty());
    assert!(run.is_dir(), "dry run must not delete");
}

#[test]
fn clean_run_removes_a_validated_run() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let run = create_run(&root, "com.example.app", 1, &json!({"kind": "contract"}));
    let run_arg = run.to_string_lossy().into_owned();

    let output = run_clean(&root, &["--run", &run_arg]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("Removed run: {run_arg}\n")
    );
    assert!(!run.exists());
}

#[test]
fn clean_package_removes_only_validated_runs() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let first = create_run(&root, "com.example.app", 1, &json!({"kind": "one"}));
    let second = create_run(&root, "com.example.app", 2, &json!({"kind": "two"}));
    let unmarked = root
        .join("com.example.app")
        .join("9.9-deadbeef")
        .join("cafebabecafe");
    fs::create_dir_all(&unmarked).expect("unmarked dir");
    fs::write(unmarked.join("scratch.bin"), b"keep").expect("scratch");

    let output = run_clean(&root, &["--package", "com.example.app"]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Removed 2 runs for package com.example.app"));
    assert!(!first.exists());
    assert!(!second.exists());
    assert!(unmarked.is_dir(), "unmarked directories are never removed");
}

#[test]
fn clean_unmarked_and_outside_runs_are_refused() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let unmarked = root.join("com.example.app").join("1.0-abc").join("def");
    fs::create_dir_all(&unmarked).expect("unmarked");
    let unmarked_arg = unmarked.to_string_lossy().into_owned();
    let output = run_clean(&root, &["--run", &unmarked_arg, "--dry-run"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Workspace error"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(unmarked.is_dir());

    let outside = directory.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    let outside_arg = outside.to_string_lossy().into_owned();
    let output = run_clean(&root, &["--run", &outside_arg]);
    assert_eq!(output.status.code(), Some(2));
    assert!(outside.is_dir());
}

#[test]
fn clean_package_traversal_is_refused() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let output = run_clean(&root, &["--package", "../escape"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Workspace error"));
}

#[test]
fn clean_requires_a_selector_and_rejects_unknown_flags() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("workspace");
    let output = run_clean(&root, &["--dry-run"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Specify --run, --package, or --stale")
    );

    let output = run_clean(&root, &["--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized option"));
}

#[test]
fn inspect_requires_an_artifact_and_rejects_unknown_flags() {
    let output = run_bin(&["inspect"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact"));

    let output = run_bin(&["inspect", "app.apk", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized option"));
}

#[test]
fn inspect_missing_artifact_exits_two() {
    let output = run_bin(&["inspect", "/definitely/missing/artifact.apk"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Artifact not found"));
}

#[test]
fn setup_invalid_profile_exits_two() {
    let output = run_bin(&["setup", "--profile", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid choice"));
}

#[test]
fn doctor_unknown_flag_exits_two_before_touching_tools() {
    let output = run_bin(&["doctor", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized option"));
}

#[test]
fn workspace_root_option_requires_a_value() {
    let output = run_bin(&["--workspace-root"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--workspace-root"));
}
