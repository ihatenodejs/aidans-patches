//! Public CLI integration tests for the benchmark `--prepare` mode.
//!
//! Preparation runs each selected manifest workload once and records accepted
//! output pins. It is not a benchmark: no samples, durations, or speed claims
//! belong in the report. The oracle is the real, pre-synced Python package
//! (`apk_lab.cli`), never a fake interpreter, so a workload that merely exits 0
//! is not enough to be accepted.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Exact stdout of `apk_lab.cli asm ret --format hex`: little-endian ARM64
/// `ret` (`0xd65f03c0`) rendered as space-separated lowercase bytes, plus the
/// formatter's trailing newline. This is independent of the prepare code under
/// test and matches `tests/test_asm.py`'s known `ret` opcode.
const RET_HEX_STDOUT: &str = "c0 03 5f d6\n";

/// SHA-256 of `RET_HEX_STDOUT` (`sha256sum` of the exact bytes).
const RET_HEX_STDOUT_SHA256: &str =
    "5d07935d10463503c7ba8b6dcc472afdddbc213ef0442b439a52d8ca66304d54";

/// SHA-256 of empty stdout/stderr.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("tools/apk-lab must live two levels below the repository root")
        .to_path_buf()
}

/// Prefer an explicitly selected interpreter, otherwise the pre-synced uv
/// virtual environment. A missing interpreter is a real environment failure,
/// never a skipped or faked test.
fn oracle_interpreter() -> PathBuf {
    let interpreter = match std::env::var_os("APK_LAB_PYTHON") {
        Some(value) => PathBuf::from(value),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python"),
    };
    assert!(
        interpreter.is_file(),
        "prepared Python interpreter not found at {}; sync tools/apk-lab/.venv or set APK_LAB_PYTHON",
        interpreter.display()
    );
    interpreter
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn manifest_json(interpreter: &Path, root: &Path, expected: Value) -> String {
    serde_json::to_string_pretty(&json!({
        "schemaVersion": 1,
        "profile": "local",
        "repositoryRoot": root.display().to_string(),
        "description": "Isolated prepare integration test manifest.",
        "engine": {
            "name": "python",
            "executable": interpreter.display().to_string(),
            "cwd": "tools/apk-lab",
        },
        "workloads": [{
            "id": "asm.ret.hex",
            "category": "code",
            "size": "small",
            "costly": false,
            "description": "ARM64 ret assembled to hex bytes.",
            "argv": ["asm", "ret", "--format", "hex"],
            "expected": expected,
            "timeoutSeconds": 60,
        }],
    }))
    .expect("manifest serialization")
}

/// Runs the migration binary with an explicit deadline so a hung oracle cannot
/// stall the test suite.
fn run_with_timeout(mut command: Command) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .expect("failed to spawn apk-lab-migrate integration binary");
    let started = Instant::now();
    loop {
        match child.try_wait().expect("failed to poll migration child") {
            Some(_) => break,
            None if started.elapsed() >= PREPARE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "apk-lab-migrate prepare exceeded {} seconds",
                    PREPARE_TIMEOUT.as_secs()
                );
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    }
    child
        .wait_with_output()
        .expect("failed to collect migration output")
}

struct PreparedRun {
    output: Output,
    report: Value,
    report_text: String,
    manifest_sha256: String,
    private_paths: Vec<String>,
}

fn run_prepare(root: &Path, interpreter: &Path, expected: Value) -> PreparedRun {
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-prepare-test-")
        .tempdir()
        .expect("cannot create isolated prepare test root");
    let sandbox_path = sandbox.path();
    let manifest_path = sandbox_path.join("manifest.json");
    let out_path = sandbox_path.join("pins.json");

    let manifest_text = manifest_json(interpreter, root, expected);
    fs::write(manifest_path.as_path(), manifest_text.as_bytes())
        .expect("cannot write test manifest");

    for relative in ["home", "xdg/config", "xdg/cache", "xdg/data", "tmp"] {
        fs::create_dir_all(sandbox_path.join(relative))
            .expect("cannot create isolated test directories");
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_apk-lab-migrate"));
    command
        .current_dir(root)
        .arg("bench")
        .arg("--prepare")
        .arg("--engine")
        .arg("python")
        .arg("--profile")
        .arg("local")
        .arg("--manifest")
        .arg(manifest_path.as_os_str())
        .arg("--out")
        .arg(out_path.as_os_str())
        .arg("--repo-root")
        .arg(root)
        .env("HOME", sandbox_path.join("home"))
        .env("XDG_CONFIG_HOME", sandbox_path.join("xdg/config"))
        .env("XDG_CACHE_HOME", sandbox_path.join("xdg/cache"))
        .env("XDG_DATA_HOME", sandbox_path.join("xdg/data"))
        .env("TMPDIR", sandbox_path.join("tmp"))
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONSTARTUP")
        .env_remove("PYTHONUSERBASE")
        .env_remove("VIRTUAL_ENV")
        .env_remove("APK_LAB_PYTHON")
        .env_remove("GITHUB_TOKEN")
        .env_remove("OPENAI_API_KEY")
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY");

    let output = run_with_timeout(command);

    let report_text = match fs::read_to_string(out_path.as_path()) {
        Ok(text) => text,
        Err(error) => panic!(
            "prepare did not leave a report at {}: {error}\nstdout:\n{}\nstderr:\n{}",
            out_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    };
    let report: Value =
        serde_json::from_str(&report_text).expect("prepare report must be valid JSON");

    let private_paths = vec![
        sandbox_path.display().to_string(),
        manifest_path.display().to_string(),
        out_path.display().to_string(),
        interpreter.display().to_string(),
        root.display().to_string(),
    ];

    PreparedRun {
        output,
        report,
        report_text,
        manifest_sha256: sha256_hex(manifest_text.as_bytes()),
        private_paths,
    }
}

fn workload_row(report: &Value) -> &Value {
    let workloads = report["workloads"]
        .as_array()
        .expect("prepare report must contain a workloads array");
    assert_eq!(workloads.len(), 1, "test manifest selects one workload");
    &workloads[0]
}

fn assert_no_private_paths(report_text: &str, private_paths: &[String]) {
    for private_path in private_paths {
        assert!(
            !report_text.contains(private_path),
            "prepare report leaked private path {private_path:?}:\n{report_text}"
        );
    }
}

fn assert_no_measurements(report: &Value) {
    assert!(
        report.get("measurements").is_none(),
        "prepare must not publish benchmark measurements: {report}"
    );
    let row = workload_row(report);
    for field in ["samples", "summary", "wallMs", "cpuMs", "peakChildRssBytes"] {
        assert!(
            row.get(field).is_none(),
            "prepare workload row must not contain {field}: {row}"
        );
    }
    assert!(
        !report.to_string().contains("wallMs"),
        "prepare report must not claim performance: {report}"
    );
}

#[test]
fn prepare_selected_asm_ret_hex_accepts_known_arm64_output_pin() {
    let root = repo_root();
    let interpreter = oracle_interpreter();
    let expected = json!({
        "exitCode": 0,
        "stdoutSha256": RET_HEX_STDOUT_SHA256,
        "stdoutContains": ["c0 03 5f d6"],
    });

    let prepared = run_prepare(&root, &interpreter, expected);
    assert!(
        prepared.output.status.success(),
        "prepare of a single valid selected workload must exit 0\nstderr:\n{}",
        String::from_utf8_lossy(&prepared.output.stderr)
    );

    let report = &prepared.report;
    assert_eq!(report["schemaVersion"], json!(1));
    assert_eq!(report["kind"], json!("apk-lab-benchmark-output-pins"));
    assert_eq!(report["status"], json!("complete"));
    assert_eq!(report["engine"], json!("python"));
    assert_eq!(report["profile"], json!("local"));
    assert_eq!(
        report["manifestSha256"],
        json!(prepared.manifest_sha256),
        "report must pin the exact manifest bytes it prepared"
    );
    assert!(
        report["source"].is_object(),
        "report must carry reconstructible source identity: {report}"
    );

    let acceptance = &report["acceptance"];
    assert_eq!(acceptance["valid"], json!(true), "{report}");
    assert!(
        acceptance["reasons"].is_array(),
        "acceptance.reasons must be an array: {report}"
    );

    let row = workload_row(report);
    assert_eq!(row["id"], json!("asm.ret.hex"));
    assert_eq!(
        row["status"],
        json!("prepared"),
        "the workload actually ran and matched its semantic assertions: {row}"
    );
    assert_eq!(row["exitCode"], json!(0));
    assert_eq!(
        row["stdoutSha256"],
        json!(RET_HEX_STDOUT_SHA256),
        "accepted stdout pin must equal the known independent hash of {RET_HEX_STDOUT:?}"
    );
    assert_eq!(
        row["rawStdoutSha256"],
        json!(RET_HEX_STDOUT_SHA256),
        "asm output contains no sandbox paths, so normalization must not change it"
    );
    assert_eq!(row["stderrSha256"], json!(EMPTY_SHA256));
    assert_eq!(row["rawStderrSha256"], json!(EMPTY_SHA256));
    assert!(
        row["outputNormalization"].is_array(),
        "outputNormalization must be a list: {row}"
    );
    assert_eq!(
        row["validationErrors"],
        json!([]),
        "a prepared row must have no validation errors"
    );

    assert_no_measurements(report);
    assert_no_private_paths(&prepared.report_text, &prepared.private_paths);
}

#[test]
fn prepare_keeps_source_digest_stable_across_manifest_expectation_changes() {
    let root = repo_root();
    let interpreter = oracle_interpreter();
    let valid_expected = json!({
        "exitCode": 0,
        "stdoutSha256": RET_HEX_STDOUT_SHA256,
        "stdoutContains": ["c0 03 5f d6"],
    });
    let invalid_expected = json!({
        "exitCode": 0,
        "stdoutContains": ["token-that-the-oracle-never-prints"],
    });

    let valid = run_prepare(&root, &interpreter, valid_expected);
    assert!(
        valid.output.status.success(),
        "valid baseline preparation must exit 0\nstderr:\n{}",
        String::from_utf8_lossy(&valid.output.stderr)
    );
    assert_eq!(valid.report["acceptance"]["valid"], json!(true));
    assert_eq!(workload_row(&valid.report)["status"], json!("prepared"));

    let invalid = run_prepare(&root, &interpreter, invalid_expected);
    assert!(
        !invalid.output.status.success(),
        "changing only the manifest expectation must fail preparation\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&invalid.output.stdout),
        String::from_utf8_lossy(&invalid.output.stderr)
    );
    assert_eq!(invalid.report["acceptance"]["valid"], json!(false));
    let invalid_row = workload_row(&invalid.report);
    assert_eq!(invalid_row["status"], json!("failed"));
    assert_eq!(
        invalid_row["stdoutSha256"],
        Value::Null,
        "failed validation must not publish an accepted stdout pin: {invalid_row}"
    );
    assert_eq!(
        invalid_row["stderrSha256"],
        Value::Null,
        "failed validation must not publish an accepted stderr pin: {invalid_row}"
    );

    assert_ne!(
        valid.manifest_sha256, invalid.manifest_sha256,
        "the two runs must prepare different manifest bytes"
    );
    assert_ne!(
        valid.report["manifestSha256"], invalid.report["manifestSha256"],
        "report manifest pins must follow the prepared manifest bytes"
    );
    assert_eq!(
        valid.report["manifestSha256"],
        json!(valid.manifest_sha256),
        "the valid report must pin the exact bytes it prepared"
    );
    assert_eq!(
        invalid.report["manifestSha256"],
        json!(invalid.manifest_sha256),
        "the invalid report must pin the exact bytes it prepared"
    );
    let valid_source_digest = valid.report["source"]["worktreeDigest"]
        .as_str()
        .expect("valid report must publish a source worktreeDigest string");
    let invalid_source_digest = invalid.report["source"]["worktreeDigest"]
        .as_str()
        .expect("invalid report must publish a source worktreeDigest string");
    assert_eq!(
        valid_source_digest, invalid_source_digest,
        "a manifest-expectation-only change must not alter executed-source identity"
    );

    assert_no_measurements(&valid.report);
    assert_no_private_paths(&valid.report_text, &valid.private_paths);
    assert_no_measurements(&invalid.report);
    assert_no_private_paths(&invalid.report_text, &invalid.private_paths);
}

#[test]
fn prepare_rejects_exit_zero_stdout_with_wrong_required_text() {
    let root = repo_root();
    let interpreter = oracle_interpreter();
    // The command really does exit 0; only the required stdout token is wrong.
    // A driver that trusts exit status alone would wrongly accept this pin.
    let expected = json!({
        "exitCode": 0,
        "stdoutContains": ["token-that-the-oracle-never-prints"],
    });

    let prepared = run_prepare(&root, &interpreter, expected);
    assert!(
        !prepared.output.status.success(),
        "prepare must fail closed when a selected workload misses required stdout\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&prepared.output.stdout),
        String::from_utf8_lossy(&prepared.output.stderr)
    );

    let report = &prepared.report;
    assert_eq!(report["schemaVersion"], json!(1));
    assert_eq!(report["kind"], json!("apk-lab-benchmark-output-pins"));
    assert_eq!(
        report["status"],
        json!("incomplete"),
        "failed preparation must be reported as incomplete, not success: {report}"
    );

    let acceptance = &report["acceptance"];
    assert_eq!(acceptance["valid"], json!(false));
    let reasons = acceptance["reasons"]
        .as_array()
        .expect("acceptance.reasons must be an array");
    assert!(
        !reasons.is_empty(),
        "incomplete acceptance needs an explicit reason: {report}"
    );

    let row = workload_row(report);
    assert_eq!(row["id"], json!("asm.ret.hex"));
    assert_eq!(
        row["status"],
        json!("failed"),
        "the command ran but failed semantic output validation: {row}"
    );
    assert_eq!(row["exitCode"], json!(0), "the oracle really exited 0");
    assert_eq!(
        row["stdoutSha256"],
        Value::Null,
        "a failed row must not publish an accepted stdout pin: {row}"
    );
    assert_eq!(
        row["stderrSha256"],
        Value::Null,
        "a failed row must not publish an accepted stderr pin: {row}"
    );
    let validation_errors = row["validationErrors"]
        .as_array()
        .expect("validationErrors must be an array");
    let mentions_stdout = validation_errors
        .iter()
        .filter_map(Value::as_str)
        .any(|text| text.contains("stdout"));
    assert!(
        mentions_stdout,
        "the report must retain the stdout semantic mismatch: {row}"
    );

    assert_no_measurements(report);
    assert_no_private_paths(&prepared.report_text, &prepared.private_paths);
}
