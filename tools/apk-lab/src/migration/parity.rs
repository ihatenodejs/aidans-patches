//! Differential parity harness for the native `apk-lab` core.
//!
//! The Python implementation is executed as an independent oracle process and
//! the native candidate is executed with identical argv and working directory.
//! Both exit status and output streams are captured. JSON output is compared
//! structurally after removing only the volatile fields declared by the case
//! table; human output is compared after the same documented normalizations.
//!
//! The case table is deliberately explicit and committed here. It contains
//! only commands the native binary implements, and every fixture it needs is
//! created inside an isolated temporary sandbox. No case touches the network,
//! a device, Google Play, or the real tool cache.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const SCHEMA_VERSION: u32 = 1;
const REPORT_KIND: &str = "apk-lab-core-parity";
const CORE_GROUP: &str = "core";

/// The only normalizations the core group applies. Anything not listed here is
/// a substantive difference and fails the case.
const NORMALIZATION_RULES: &[&str] = &[
    "absolute cwd/sandbox/home/temp roots are replaced with <cwd>/<sandbox>/<home>/<temp>",
    "doctor host prerequisite/tool version, path and error details are replaced with <volatile>",
    "declared JSON fields are replaced with <path>/<version>",
    "CRLF line endings are normalized to LF",
];

// ---------------------------------------------------------------------------
// Committed case table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamMode {
    Stdout,
    Stderr,
}

impl StreamMode {
    fn as_str(self) -> &'static str {
        match self {
            StreamMode::Stdout => "stdout",
            StreamMode::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolatileKind {
    Path,
    Version,
}

#[derive(Debug, Clone, Copy)]
struct JsonVolatile {
    pointer: &'static str,
    kind: VolatileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextRule {
    /// Doctor output embeds host-specific tool versions, paths and error text.
    HostToolDetails,
}

#[derive(Debug, Clone, Copy)]
struct CaseSpec {
    id: &'static str,
    argv: &'static [&'static str],
    /// Stream that carries the command's primary output. Both captured streams
    /// are still compared; this records the expected mode for the contract.
    stream: StreamMode,
    /// When set, that stream is parsed as JSON and compared semantically.
    json_stream: Option<StreamMode>,
    volatile_json: &'static [JsonVolatile],
    text_rules: &'static [TextRule],
}

const INSPECT_JSON_VOLATILE: &[JsonVolatile] = &[JsonVolatile {
    pointer: "/filePath",
    kind: VolatileKind::Path,
}];

const DOCTOR_JSON_VOLATILE: &[JsonVolatile] = &[
    JsonVolatile {
        pointer: "/prerequisites/*/versionOrPath",
        kind: VolatileKind::Version,
    },
    JsonVolatile {
        pointer: "/prerequisites/*/error",
        kind: VolatileKind::Version,
    },
    JsonVolatile {
        pointer: "/tools/*/actualVersion",
        kind: VolatileKind::Version,
    },
    JsonVolatile {
        pointer: "/tools/*/executablePath",
        kind: VolatileKind::Path,
    },
    JsonVolatile {
        pointer: "/tools/*/error",
        kind: VolatileKind::Version,
    },
];

const DOCTOR_TEXT_RULES: &[TextRule] = &[TextRule::HostToolDetails];

/// Maintainer-approved native contract differences. Python remains the default
/// CLI while the native binary exposes only its implemented core, so these are
/// intentional rather than behavioral regressions.
const APPROVED_CASES: &[(&str, &str)] = &[
    (
        "help",
        "native helps only with implemented commands; Python argparse lists the full CLI",
    ),
    (
        "version",
        "native adds a conventional --version flag that Python does not define",
    ),
    (
        "doctor-json",
        "native intentionally omits Python/uv prerequisites per plan 002",
    ),
    (
        "unknown-command",
        "both exit 2; argparse and native usage text differ",
    ),
];

fn approved_reason(case_id: &str) -> Option<&'static str> {
    APPROVED_CASES
        .iter()
        .find(|(id, _)| *id == case_id)
        .map(|(_, reason)| *reason)
}

const CORE_CASES: &[CaseSpec] = &[
    CaseSpec {
        id: "help",
        argv: &["--help"],
        stream: StreamMode::Stdout,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
    CaseSpec {
        id: "version",
        argv: &["--version"],
        stream: StreamMode::Stdout,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
    CaseSpec {
        id: "inspect-json",
        argv: &["inspect", "fixtures/core.apk", "--json"],
        stream: StreamMode::Stdout,
        json_stream: Some(StreamMode::Stdout),
        volatile_json: INSPECT_JSON_VOLATILE,
        text_rules: &[],
    },
    CaseSpec {
        id: "inspect-human",
        argv: &["inspect", "fixtures/core.apk"],
        stream: StreamMode::Stdout,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
    CaseSpec {
        id: "doctor-json",
        argv: &["doctor", "--json"],
        stream: StreamMode::Stdout,
        json_stream: Some(StreamMode::Stdout),
        volatile_json: DOCTOR_JSON_VOLATILE,
        text_rules: DOCTOR_TEXT_RULES,
    },
    CaseSpec {
        id: "unknown-command",
        argv: &["definitely-not-an-apk-lab-command"],
        stream: StreamMode::Stderr,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
    CaseSpec {
        id: "inspect-missing-artifact",
        argv: &["inspect", "fixtures/absent.apk"],
        stream: StreamMode::Stderr,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
    CaseSpec {
        id: "inspect-invalid-artifact",
        argv: &["inspect", "fixtures/unknown.apk"],
        stream: StreamMode::Stderr,
        json_stream: None,
        volatile_json: &[],
        text_rules: &[],
    },
];

// ---------------------------------------------------------------------------
// Options and entry point
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Options {
    group: String,
    python_project: Option<PathBuf>,
    rust_bin: Option<PathBuf>,
    cases: Vec<String>,
    report: Option<String>,
    list: bool,
}

/// Runs `parity` with the arguments that follow the subcommand.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_args(args)? else {
        print_help();
        return Ok(());
    };

    if options.group != CORE_GROUP {
        return Err(format!(
            "unsupported parity group '{}' (available: '{CORE_GROUP}')",
            options.group
        ));
    }

    let selected = select_cases(&options.cases)?;
    if options.list {
        print_list(&selected);
        return Ok(());
    }

    let python_project = options
        .python_project
        .ok_or("--python-project is required unless --list is used")?;
    let rust_bin = options
        .rust_bin
        .ok_or("--rust-bin is required unless --list is used")?;
    let python_project = require_directory(&python_project, "--python-project")?;
    let rust_bin = require_executable(&rust_bin, "--rust-bin")?;
    let oracle = resolve_oracle(&python_project)?;

    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-core-parity-")
        .tempdir()
        .map_err(|error| format!("could not create parity sandbox: {error}"))?;
    let report = execute(&selected, &oracle, &rust_bin, sandbox.path())?;

    let rendered = render_report(&report)?;
    write_report(options.report.as_deref(), &rendered)?;

    if !report_passed(&report) {
        return Err(format!(
            "core parity failed: {} of {} case(s) did not pass",
            report.summary.failed + report.summary.missing,
            report.summary.total
        ));
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut group: Option<String> = None;
    let mut python_project: Option<PathBuf> = None;
    let mut rust_bin: Option<PathBuf> = None;
    let mut cases = Vec::new();
    let mut report: Option<String> = None;
    let mut list = false;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => return Ok(None),
            "--list" => {
                list = true;
                index += 1;
            }
            "--group" => {
                group = Some(require_value(args, &mut index, "--group")?);
            }
            _ if arg.starts_with("--group=") => {
                group = Some(non_empty(&arg["--group=".len()..], "--group")?);
                index += 1;
            }
            "--python-project" => {
                python_project = Some(PathBuf::from(require_value(
                    args,
                    &mut index,
                    "--python-project",
                )?));
            }
            _ if arg.starts_with("--python-project=") => {
                python_project = Some(PathBuf::from(non_empty(
                    &arg["--python-project=".len()..],
                    "--python-project",
                )?));
                index += 1;
            }
            "--rust-bin" => {
                rust_bin = Some(PathBuf::from(require_value(
                    args,
                    &mut index,
                    "--rust-bin",
                )?));
            }
            _ if arg.starts_with("--rust-bin=") => {
                rust_bin = Some(PathBuf::from(non_empty(
                    &arg["--rust-bin=".len()..],
                    "--rust-bin",
                )?));
                index += 1;
            }
            "--case" => {
                cases.push(require_value(args, &mut index, "--case")?);
            }
            _ if arg.starts_with("--case=") => {
                cases.push(non_empty(&arg["--case=".len()..], "--case")?);
                index += 1;
            }
            "--report" => {
                report = Some(require_value(args, &mut index, "--report")?);
            }
            _ if arg.starts_with("--report=") => {
                report = Some(non_empty(&arg["--report=".len()..], "--report")?);
                index += 1;
            }
            other => {
                return Err(format!(
                    "unrecognized parity option '{other}' (run 'parity --help')"
                ));
            }
        }
    }

    let group = group.ok_or("--group is required (available: 'core')")?;
    Ok(Some(Options {
        group,
        python_project,
        rust_bin,
        cases,
        report,
        list,
    }))
}

fn require_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let Some(value) = args.get(*index + 1) else {
        return Err(format!("{flag} requires a value"));
    };
    *index += 2;
    non_empty(value, flag)
}

fn non_empty(value: &str, flag: &str) -> Result<String, String> {
    if value.is_empty() {
        Err(format!("{flag} requires a non-empty value"))
    } else {
        Ok(value.to_string())
    }
}

fn select_cases(requested: &[String]) -> Result<Vec<&'static CaseSpec>, String> {
    if requested.is_empty() {
        return Ok(CORE_CASES.iter().collect());
    }
    let mut wanted = BTreeSet::new();
    for id in requested {
        if !CORE_CASES.iter().any(|case| case.id == id) {
            return Err(format!(
                "unknown core parity case '{id}' (run 'parity --group core --list')"
            ));
        }
        wanted.insert(id.as_str());
    }
    Ok(CORE_CASES
        .iter()
        .filter(|case| wanted.contains(case.id))
        .collect())
}

fn print_list(cases: &[&CaseSpec]) {
    let mut text = String::new();
    for case in cases {
        text.push_str(case.id);
        text.push('\t');
        text.push_str(case.stream.as_str());
        text.push('\t');
        text.push_str(&case.argv.join(" "));
        text.push('\n');
    }
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
}

// ---------------------------------------------------------------------------
// Executable resolution
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Oracle {
    /// The project virtualenv entrypoint. Preferred because it is a fixed file
    /// that needs no resolver, network access, or package installation.
    Direct(PathBuf),
    /// Deterministic fallback recommended by the migration plan.
    Uv { uv: PathBuf, project: PathBuf },
}

fn resolve_oracle(project: &Path) -> Result<Oracle, String> {
    let candidates: &[&str] = if cfg!(windows) {
        &[".venv/Scripts/apk-lab.exe", ".venv/Scripts/apk-lab"]
    } else {
        &[".venv/bin/apk-lab"]
    };
    for relative in candidates {
        let candidate = project.join(relative);
        if is_executable(&candidate) {
            return Ok(Oracle::Direct(candidate));
        }
    }
    if let Some(uv) = find_in_path("uv") {
        return Ok(Oracle::Uv {
            uv,
            project: project.to_path_buf(),
        });
    }
    Err(format!(
        "no Python oracle entrypoint found in '{}': expected {} or 'uv' on PATH",
        project.display(),
        candidates.join(" / ")
    ))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = directory.join(format!("{name}.exe"));
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn require_directory(path: &Path, flag: &str) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!(
            "{flag} path is not a directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|error| format!("{flag} cannot be resolved: {error}"))
}

fn require_executable(path: &Path, flag: &str) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!("{flag} is not a file: {}", path.display()));
    }
    if !is_executable(path) {
        return Err(format!("{flag} is not executable: {}", path.display()));
    }
    fs::canonicalize(path).map_err(|error| format!("{flag} cannot be resolved: {error}"))
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ProcessResult {
    exit: i64,
    stdout: String,
    stderr: String,
}

impl ProcessResult {
    fn missing() -> Self {
        Self {
            exit: -1,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

#[derive(Debug)]
struct Environment {
    variables: Vec<(String, String)>,
}

impl Environment {
    fn for_sandbox(sandbox: &Path, project: &Path, fake_sdk: &Path) -> Self {
        let home = sandbox.join("home");
        let temp = sandbox.join("tmp");
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
        let variables = vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), home.to_string_lossy().into_owned()),
            ("TMPDIR".to_string(), temp.to_string_lossy().into_owned()),
            ("TMP".to_string(), temp.to_string_lossy().into_owned()),
            ("TEMP".to_string(), temp.to_string_lossy().into_owned()),
            ("TZ".to_string(), "UTC".to_string()),
            (
                "ANDROID_HOME".to_string(),
                fake_sdk.to_string_lossy().into_owned(),
            ),
            (
                "ANDROID_SDK_ROOT".to_string(),
                fake_sdk.to_string_lossy().into_owned(),
            ),
            (
                "PYTHONPATH".to_string(),
                project.to_string_lossy().into_owned(),
            ),
            ("PYTHONIOENCODING".to_string(), "utf-8".to_string()),
            ("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()),
            ("PYTHONHASHSEED".to_string(), "0".to_string()),
            ("PYTHONUTF8".to_string(), "1".to_string()),
        ];
        Self { variables }
    }
}

fn execute(
    cases: &[&CaseSpec],
    oracle: &Oracle,
    rust_bin: &Path,
    sandbox: &Path,
) -> Result<ParityReport, String> {
    let run_root = sandbox.join("run");
    let fake_sdk = sandbox.join("fake-sdk");
    create_fixtures(&run_root, &fake_sdk)?;
    for directory in [sandbox.join("home"), sandbox.join("tmp")] {
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    let environment = Environment::for_sandbox(sandbox, python_project_path(oracle), &fake_sdk);
    let normalizer = Normalizer::new(NormalizationContext::new(sandbox, &run_root));

    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let python = match run_process(oracle_command(oracle, case), &run_root, &environment) {
            Ok(result) => result,
            Err(error) => {
                results.push(missing_result(case, format!("python: {error}")));
                continue;
            }
        };
        let rust = match run_binary(rust_bin, case.argv, &run_root, &environment) {
            Ok(result) => result,
            Err(error) => {
                results.push(missing_result_with_exit(
                    case,
                    &python,
                    format!("rust: {error}"),
                ));
                continue;
            }
        };
        let differences = compare_case(case, &python, &rust, &normalizer.at(case));
        let approved = !differences.is_empty() && approved_reason(case.id).is_some();
        let status = if differences.is_empty() || approved {
            CaseStatus::Passed
        } else {
            CaseStatus::Failed
        };
        results.push(CaseResult {
            id: case.id.to_string(),
            argv: case.argv.iter().map(|value| value.to_string()).collect(),
            status,
            approved,
            approval: approved_reason(case.id).map(str::to_string),
            exit: ExitPair {
                python: python.exit,
                rust: rust.exit,
            },
            differences,
        });
    }

    Ok(build_report(results))
}

fn python_project_path(oracle: &Oracle) -> &Path {
    match oracle {
        Oracle::Direct(entrypoint) => entrypoint
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .unwrap_or(Path::new(".")),
        Oracle::Uv { project, .. } => project,
    }
}

fn oracle_command(oracle: &Oracle, case: &CaseSpec) -> (PathBuf, Vec<String>) {
    match oracle {
        Oracle::Direct(entrypoint) => (
            entrypoint.clone(),
            case.argv.iter().map(|value| value.to_string()).collect(),
        ),
        Oracle::Uv { uv, project } => {
            let mut args = vec![
                "run".to_string(),
                "--project".to_string(),
                project.to_string_lossy().into_owned(),
                "apk-lab".to_string(),
            ];
            args.extend(case.argv.iter().map(|value| value.to_string()));
            (uv.clone(), args)
        }
    }
}

fn run_process(
    command: (PathBuf, Vec<String>),
    cwd: &Path,
    environment: &Environment,
) -> Result<ProcessResult, String> {
    run_binary(&command.0, &command.1, cwd, environment)
}

fn run_binary(
    program: &Path,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    cwd: &Path,
    environment: &Environment,
) -> Result<ProcessResult, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &environment.variables {
        command.env(name, value);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run '{}': {error}", program.display()))?;
    Ok(ProcessResult {
        exit: output.status.code().map(i64::from).unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn missing_result(case: &CaseSpec, difference: String) -> CaseResult {
    missing_result_with_exit(case, &ProcessResult::missing(), difference)
}

fn missing_result_with_exit(
    case: &CaseSpec,
    python: &ProcessResult,
    difference: String,
) -> CaseResult {
    CaseResult {
        id: case.id.to_string(),
        argv: case.argv.iter().map(|value| value.to_string()).collect(),
        status: CaseStatus::Missing,
        approved: false,
        approval: None,
        exit: ExitPair {
            python: python.exit,
            rust: -1,
        },
        differences: vec![difference],
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

fn compare_case(
    case: &CaseSpec,
    python: &ProcessResult,
    rust: &ProcessResult,
    normalizer: &Normalizer,
) -> Vec<String> {
    let mut differences = Vec::new();
    if python.exit != rust.exit {
        differences.push(format!(
            "exit: python {} != rust {}",
            python.exit, rust.exit
        ));
    }

    let python_stdout = normalizer.text(&python.stdout);
    let rust_stdout = normalizer.text(&rust.stdout);
    let python_stderr = normalizer.text(&python.stderr);
    let rust_stderr = normalizer.text(&rust.stderr);

    match case.json_stream {
        Some(StreamMode::Stdout) => compare_json_stream(
            "stdout",
            &python_stdout,
            &rust_stdout,
            case.volatile_json,
            normalizer,
            &mut differences,
        ),
        Some(StreamMode::Stderr) => compare_json_stream(
            "stderr",
            &python_stderr,
            &rust_stderr,
            case.volatile_json,
            normalizer,
            &mut differences,
        ),
        None => {
            compare_text("stdout", &python_stdout, &rust_stdout, &mut differences);
            compare_text("stderr", &python_stderr, &rust_stderr, &mut differences);
        }
    }

    if case.json_stream.is_some() {
        compare_text("stderr", &python_stderr, &rust_stderr, &mut differences);
    }

    differences
}

fn compare_json_stream(
    label: &str,
    python: &str,
    rust: &str,
    volatile: &[JsonVolatile],
    normalizer: &Normalizer,
    differences: &mut Vec<String>,
) {
    let Some(mut python_json) = parse_json(label, "python", python, differences) else {
        return;
    };
    let Some(mut rust_json) = parse_json(label, "rust", rust, differences) else {
        return;
    };
    normalize_json(&mut python_json, volatile, &normalizer.context);
    normalize_json(&mut rust_json, volatile, &normalizer.context);
    compare_json(&python_json, &rust_json, label, differences);
}

fn parse_json(label: &str, side: &str, text: &str, differences: &mut Vec<String>) -> Option<Value> {
    match serde_json::from_str(text) {
        Ok(value) => Some(value),
        Err(error) => {
            differences.push(format!(
                "{label}: {side} output is not JSON ({error}): {}",
                compact(text, 240)
            ));
            None
        }
    }
}

fn compare_text(label: &str, expected: &str, actual: &str, differences: &mut Vec<String>) {
    if expected != actual {
        differences.push(format!(
            "{label} differs: python {} rust {}",
            compact(expected, 240),
            compact(actual, 240)
        ));
    }
}

fn compare_json(python: &Value, rust: &Value, pointer: &str, differences: &mut Vec<String>) {
    if differences.len() >= 25 {
        return;
    }
    match (python, rust) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let child = format!("{pointer}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(a), Some(b)) => compare_json(a, b, &child, differences),
                    (Some(a), None) => differences.push(format!(
                        "{child}: present only in python ({})",
                        compact(&a.to_string(), 160)
                    )),
                    (None, Some(b)) => differences.push(format!(
                        "{child}: present only in rust ({})",
                        compact(&b.to_string(), 160)
                    )),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                differences.push(format!(
                    "{pointer}: array length python {} != rust {}",
                    left.len(),
                    right.len()
                ));
            }
            for (index, (a, b)) in left.iter().zip(right.iter()).enumerate() {
                compare_json(a, b, &format!("{pointer}/{index}"), differences);
            }
        }
        _ if python != rust => differences.push(format!(
            "{pointer}: python {} != rust {}",
            compact(&python.to_string(), 160),
            compact(&rust.to_string(), 160)
        )),
        _ => {}
    }
}

fn compact(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut result: String = trimmed.chars().take(limit).collect();
    result.push('…');
    result
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct NormalizationContext {
    replacements: Vec<(String, String)>,
}

impl NormalizationContext {
    fn new(sandbox: &Path, run_root: &Path) -> Self {
        let home = sandbox.join("home");
        let temp = sandbox.join("tmp");
        let mut replacements = vec![
            (run_root.to_string_lossy().into_owned(), "<cwd>".to_string()),
            (
                sandbox.to_string_lossy().into_owned(),
                "<sandbox>".to_string(),
            ),
            (home.to_string_lossy().into_owned(), "<home>".to_string()),
            (temp.to_string_lossy().into_owned(), "<temp>".to_string()),
        ];
        // Longest first so nested roots replace deterministically.
        replacements.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
        replacements.dedup_by(|left, right| left.0 == right.0);
        Self { replacements }
    }

    fn replace_paths(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (from, to) in &self.replacements {
            result = result.replace(from, to);
        }
        result
    }

    #[cfg(test)]
    fn for_test(cwd: &str) -> Self {
        Self {
            replacements: vec![(cwd.to_string(), "<cwd>".to_string())],
        }
    }
}

struct Normalizer {
    context: NormalizationContext,
    rules: &'static [TextRule],
}

impl Normalizer {
    fn new(context: NormalizationContext) -> Self {
        Self {
            context,
            rules: &[],
        }
    }

    fn at(&self, case: &CaseSpec) -> Self {
        Self {
            context: self.context.clone(),
            rules: case.text_rules,
        }
    }

    fn text(&self, text: &str) -> String {
        let normalized = self.context.replace_paths(&text.replace("\r\n", "\n"));
        if self.rules.contains(&TextRule::HostToolDetails) {
            normalize_host_tool_lines(&normalized)
        } else {
            normalized
        }
    }
}

/// Replaces host-specific details on doctor status lines while preserving the
/// status and the checked item name. This is the only human-output rule the
/// core group declares.
fn normalize_host_tool_lines(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        lines.push(normalize_host_tool_line(line));
    }
    lines.join("\n")
}

fn normalize_host_tool_line(line: &str) -> String {
    let (status, rest) = if let Some(rest) = line.strip_prefix("  [OK] ") {
        ("[OK]", rest)
    } else if let Some(rest) = line.strip_prefix("  [MISSING] ") {
        ("[MISSING]", rest)
    } else {
        return line.to_string();
    };

    let name_end = [":", " v", " (", " - "]
        .iter()
        .filter_map(|separator| rest.find(separator))
        .min()
        .unwrap_or(rest.len());
    let name = rest[..name_end].trim_end();
    if name.is_empty() {
        return line.to_string();
    }
    format!("  {status} {name}: <volatile>")
}

fn normalize_json(value: &mut Value, volatile: &[JsonVolatile], context: &NormalizationContext) {
    normalize_json_paths(value, context);
    for rule in volatile {
        apply_json_pointer(value, rule.pointer, rule.kind);
    }
}

fn normalize_json_paths(value: &mut Value, context: &NormalizationContext) {
    match value {
        Value::String(text) => *text = context.replace_paths(text),
        Value::Array(items) => {
            for item in items {
                normalize_json_paths(item, context);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                normalize_json_paths(item, context);
            }
        }
        _ => {}
    }
}

fn apply_json_pointer(value: &mut Value, pointer: &str, kind: VolatileKind) {
    let segments: Vec<&str> = pointer.trim_start_matches('/').split('/').collect();
    apply_pointer_segments(value, &segments, kind);
}

fn apply_pointer_segments(value: &mut Value, segments: &[&str], kind: VolatileKind) {
    let Some((segment, rest)) = segments.split_first() else {
        *value = volatile_placeholder(value, kind);
        return;
    };
    match value {
        Value::Object(map) => {
            if let Some(child) = map.get_mut(*segment) {
                apply_pointer_segments(child, rest, kind);
            }
        }
        Value::Array(items) => {
            if *segment == "*" {
                for item in items {
                    apply_pointer_segments(item, rest, kind);
                }
            } else if let Ok(index) = segment.parse::<usize>() {
                if let Some(item) = items.get_mut(index) {
                    apply_pointer_segments(item, rest, kind);
                }
            }
        }
        _ => {}
    }
}

fn volatile_placeholder(value: &Value, kind: VolatileKind) -> Value {
    match kind {
        VolatileKind::Version => Value::String("<version>".to_string()),
        VolatileKind::Path => match value {
            Value::String(text) if !looks_absolute(text) => value.clone(),
            _ => Value::String("<path>".to_string()),
        },
    }
}

fn looks_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn create_fixtures(run_root: &Path, fake_sdk: &Path) -> Result<(), String> {
    let fixtures = run_root.join("fixtures");
    fs::create_dir_all(&fixtures)
        .map_err(|error| format!("could not create fixture directory: {error}"))?;
    fs::create_dir_all(fake_sdk.join("build-tools/36.0.0"))
        .map_err(|error| format!("could not create fake SDK: {error}"))?;

    let dex = dex_header(3, 7);
    let entries: [(&str, &[u8]); 3] = [
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.parity\"/>" as &[u8],
        ),
        ("classes.dex", dex.as_slice()),
        ("assets/note.txt", b"parity fixture" as &[u8]),
    ];
    write_apk(&fixtures.join("core.apk"), &entries)?;
    write_apk(
        &fixtures.join("unknown.apk"),
        &[("note.txt", b"not an android container" as &[u8])],
    )?;

    let aapt2 = fake_sdk.join("build-tools/36.0.0/aapt2");
    fs::write(&aapt2, FAKE_AAPT2)
        .map_err(|error| format!("could not write fake aapt2: {error}"))?;
    make_executable(&aapt2)?;
    let apksigner = fake_sdk.join("build-tools/36.0.0/apksigner");
    fs::write(&apksigner, "#!/bin/sh\nexit 1\n")
        .map_err(|error| format!("could not write fake apksigner: {error}"))?;
    make_executable(&apksigner)?;
    Ok(())
}

const FAKE_AAPT2: &str = r#"#!/bin/sh
set -eu
if [ "${1:-}" != "dump" ]; then
  echo "fake aapt2 only supports dump" >&2
  exit 64
fi
if [ "${2:-}" != "badging" ]; then
  echo "fake aapt2 only supports dump badging" >&2
  exit 64
fi
printf "package: name='com.example.parity' versionCode='42' versionName='1.2.3'\n"
printf "sdkVersion:'24'\n"
printf "targetSdkVersion:'35'\n"
printf "uses-permission: name='android.permission.INTERNET'\n"
"#;

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("could not stat {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("could not make {} executable: {error}", path.display()))?;
    }
    Ok(())
}

fn dex_header(class_count: u32, method_count: u32) -> Vec<u8> {
    let mut header = vec![0u8; 0x70];
    header[0..8].copy_from_slice(b"dex\n035\0");
    header[0x58..0x5c].copy_from_slice(&method_count.to_le_bytes());
    header[0x60..0x64].copy_from_slice(&class_count.to_le_bytes());
    header
}

fn write_apk(path: &Path, entries: &[(&str, &[u8])]) -> Result<(), String> {
    let file = fs::File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, data) in entries {
        writer
            .start_file(*name, options)
            .map_err(|error| format!("could not start zip entry {name}: {error}"))?;
        writer
            .write_all(data)
            .map_err(|error| format!("could not write zip entry {name}: {error}"))?;
    }
    writer
        .finish()
        .map_err(|error| format!("could not finish {}: {error}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CaseStatus {
    Passed,
    Failed,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
struct ExitPair {
    python: i64,
    rust: i64,
}

#[derive(Debug, Clone, Serialize)]
struct CaseResult {
    id: String,
    argv: Vec<String>,
    status: CaseStatus,
    approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<String>,
    exit: ExitPair,
    differences: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Summary {
    total: usize,
    passed: usize,
    failed: usize,
    missing: usize,
    approved: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParityReport {
    schema_version: u32,
    kind: &'static str,
    cases: Vec<CaseResult>,
    summary: Summary,
}

fn build_report(cases: Vec<CaseResult>) -> ParityReport {
    let mut summary = Summary {
        total: cases.len(),
        ..Summary::default()
    };
    for case in &cases {
        if case.approved {
            summary.approved += 1;
        }
        match case.status {
            CaseStatus::Passed => summary.passed += 1,
            CaseStatus::Failed => summary.failed += 1,
            CaseStatus::Missing => summary.missing += 1,
        }
    }
    ParityReport {
        schema_version: SCHEMA_VERSION,
        kind: REPORT_KIND,
        cases,
        summary,
    }
}

fn report_passed(report: &ParityReport) -> bool {
    report.summary.failed == 0 && report.summary.missing == 0
}

fn render_report(report: &ParityReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map(|mut json| {
            json.push('\n');
            json
        })
        .map_err(|error| format!("could not serialize parity report: {error}"))
}

fn write_report(path: Option<&str>, rendered: &str) -> Result<(), String> {
    match path {
        None | Some("-") => {
            let mut out = io::stdout().lock();
            out.write_all(rendered.as_bytes())
                .map_err(|error| format!("could not write parity report: {error}"))
        }
        Some(path) => fs::write(path, rendered)
            .map_err(|error| format!("could not write parity report '{}': {error}", path)),
    }
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

fn print_help() {
    let rules = NORMALIZATION_RULES
        .iter()
        .map(|rule| format!("  - {rule}"))
        .collect::<Vec<_>>()
        .join("\n");
    println!(
        "Usage:\n\
         \x20 apk-lab-migrate parity --group core --python-project DIR --rust-bin PATH\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group core --list [--case ID ...]\n\
         \n\
         Runs the independent Python oracle and the native binary with identical\n\
         argv and cwd, then compares exit status and output. JSON output is\n\
         compared semantically after removing the volatile fields declared by the\n\
         committed core case table; human output uses the same normalization set.\n\
         \n\
         Options:\n\
         \x20 --group NAME          Parity group (available: core)\n\
         \x20 --python-project DIR  Python project containing the oracle (tools/apk-lab)\n\
         \x20 --rust-bin PATH       Native apk-lab binary to compare\n\
         \x20 --case ID             Run only this case; may be repeated\n\
         \x20 --report PATH|-       Write the deterministic JSON report (default stdout)\n\
         \x20 --list                List the selected cases without running them\n\
         \x20 -h, --help            Print help\n\
         \n\
         Normalization rules:\n\
         {rules}\n\
         \n\
         Exits nonzero when a case is missing or any captured difference remains."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(id: &str) -> &'static CaseSpec {
        CORE_CASES
            .iter()
            .find(|case| case.id == id)
            .expect("case exists")
    }

    fn result(exit: i64, stdout: &str, stderr: &str) -> ProcessResult {
        ProcessResult {
            exit,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn missing_rust_executable_is_rejected() {
        let error = require_executable(
            Path::new("/definitely/absent/apk-lab-core-parity"),
            "--rust-bin",
        )
        .expect_err("missing executable must be rejected");
        assert!(error.contains("not a file"), "{error}");
    }

    #[test]
    fn missing_python_project_is_rejected() {
        let error = require_directory(
            Path::new("/definitely/absent/apk-lab-python-project"),
            "--python-project",
        )
        .expect_err("missing project must be rejected");
        assert!(error.contains("not a directory"), "{error}");
    }

    #[test]
    fn unknown_case_is_rejected_before_any_report() {
        let error = select_cases(&["not-a-real-case".to_string()])
            .expect_err("unknown case must be rejected");
        assert!(error.contains("unknown core parity case"), "{error}");
    }

    #[test]
    fn exit_mismatch_is_reported_as_a_failure() {
        let spec = case("inspect-json");
        let context = NormalizationContext::for_test("/tmp/parity-case");
        let normalizer = Normalizer::new(context).at(spec);
        let python = result(0, "{}", "");
        let rust = result(2, "{}", "");
        let differences = compare_case(spec, &python, &rust, &normalizer);
        assert!(
            differences
                .iter()
                .any(|line| line.contains("exit: python 0 != rust 2")),
            "{differences:?}"
        );
    }

    #[test]
    fn json_volatile_path_and_version_fields_are_normalized() {
        let mut python = serde_json::json!({
            "filePath": "/tmp/one/fixtures/core.apk",
            "prerequisites": [{"name": "Java", "versionOrPath": "Java 21"}],
            "tools": [{"name": "morphe", "executablePath": "/tmp/one/bin/morphe"}],
        });
        let mut rust = serde_json::json!({
            "filePath": "/tmp/two/fixtures/core.apk",
            "prerequisites": [{"name": "Java", "versionOrPath": "Java 23"}],
            "tools": [{"name": "morphe", "executablePath": "/tmp/two/bin/morphe"}],
        });
        let mut rules = Vec::new();
        rules.extend_from_slice(INSPECT_JSON_VOLATILE);
        rules.extend_from_slice(DOCTOR_JSON_VOLATILE);
        let context = NormalizationContext::for_test("/tmp/parity-case");
        normalize_json(&mut python, &rules, &context);
        normalize_json(&mut rust, &rules, &context);
        assert_eq!(python, rust);
        assert_eq!(python["filePath"], "<path>");
        assert_eq!(python["prerequisites"][0]["versionOrPath"], "<version>");
    }

    #[test]
    fn human_output_normalizes_paths_and_doctor_details() {
        let context = NormalizationContext::for_test("/tmp/parity-case");
        let normalizer = Normalizer::new(context).at(case("doctor-json"));
        let python = normalizer.text(
            "Artifact: /tmp/parity-case/fixtures/core.apk\n\
             Host Prerequisites:\n\
             \x20 [OK] Java: Java 21\n\
             \x20 [OK] morphe v1.2.3 (/tmp/parity-case/bin/morphe)\n",
        );
        let rust = normalizer.text(
            "Artifact: /tmp/parity-case/fixtures/core.apk\n\
             Host Prerequisites:\n\
             \x20 [OK] Java: Java 23\n\
             \x20 [OK] morphe v1.9.9 (/tmp/parity-case/bin/morphe-other)\n",
        );
        assert_eq!(python, rust);
        assert!(python.contains("Artifact: <cwd>/fixtures/core.apk"));
        assert!(python.contains("  [OK] Java: <volatile>"));
    }

    #[test]
    fn failed_report_is_nonzero_and_deterministic() {
        let cases = vec![
            CaseResult {
                id: "help".to_string(),
                argv: vec!["--help".to_string()],
                status: CaseStatus::Failed,
                approved: false,
                approval: None,
                exit: ExitPair { python: 0, rust: 2 },
                differences: vec!["exit: python 0 != rust 2".to_string()],
            },
            CaseResult {
                id: "inspect-human".to_string(),
                argv: vec!["inspect".to_string(), "fixtures/core.apk".to_string()],
                status: CaseStatus::Passed,
                approved: false,
                approval: None,
                exit: ExitPair { python: 0, rust: 0 },
                differences: Vec::new(),
            },
        ];
        let report = build_report(cases);
        assert_eq!(report.summary.total, 2);
        assert_eq!(report.summary.passed, 1);
        assert_eq!(report.summary.failed, 1);
        assert!(!report_passed(&report));

        let rendered = render_report(&report).expect("report renders");
        assert!(rendered.starts_with("{\n  \"schemaVersion\": 1,"));
        assert!(rendered.contains("\"kind\": \"apk-lab-core-parity\""));
        assert!(rendered.contains("\"status\": \"failed\""));
        let parsed: Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(
            parsed["summary"],
            serde_json::json!({
                "total": 2,
                "passed": 1,
                "failed": 1,
                "missing": 0,
                "approved": 0
            })
        );
        assert_eq!(render_report(&report).expect("renders again"), rendered);
    }

    #[test]
    fn case_listing_is_deterministic_and_names_streams() {
        let selected = select_cases(&[]).expect("all cases");
        assert_eq!(selected.len(), CORE_CASES.len());
        assert_eq!(selected[0].id, "help");
        assert_eq!(selected[0].stream.as_str(), "stdout");
        assert_eq!(case("inspect-missing-artifact").stream, StreamMode::Stderr);
        assert_eq!(
            select_cases(&["version".to_string(), "help".to_string()])
                .expect("filtered")
                .iter()
                .map(|case| case.id)
                .collect::<Vec<_>>(),
            vec!["help", "version"]
        );
    }

    #[test]
    fn fixtures_form_a_tiny_valid_android_container() {
        let directory = tempfile::tempdir().expect("tempdir");
        let run_root = directory.path().join("run");
        let sdk = directory.path().join("sdk");
        create_fixtures(&run_root, &sdk).expect("fixtures");

        let apk = run_root.join("fixtures/core.apk");
        let file = fs::File::open(&apk).expect("open apk");
        let mut archive = zip::ZipArchive::new(file).expect("valid zip");
        assert!(archive.by_name("AndroidManifest.xml").is_ok());
        assert!(archive.by_name("classes.dex").is_ok());
        assert!(is_executable(&sdk.join("build-tools/36.0.0/aapt2")));
    }

    #[test]
    fn normalization_rules_are_documented() {
        assert!(NORMALIZATION_RULES
            .iter()
            .any(|rule| rule.contains("cwd/sandbox/home/temp")));
        assert!(NORMALIZATION_RULES
            .iter()
            .any(|rule| rule.contains("doctor")));
        assert!(NORMALIZATION_RULES.iter().any(|rule| rule.contains("JSON")));
    }
}
