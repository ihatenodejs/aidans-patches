//! Independent recorder and verifier for the Python `apk-lab` oracle.
//!
//! Capture executes the real Python CLI against synthetic, isolated fixtures.
//! The resulting document records exit status, exact stdout/stderr, structured
//! JSON, and the provisional file tree. Verification compares two documents
//! exactly, except for the explicitly named volatile classes below. No Rust
//! candidate implementation participates in capture or verification.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCHEMA_VERSION: u32 = 1;
const CAPTURE_KIND: &str = "apk-lab-python-contracts";

/// Active volatile classes. Captures retain exact ANSI bytes for their own
/// terminal mode; no capture is normalized across terminal modes. Timestamps
/// and durations are omitted from case records instead of being normalized.
const NORMALIZATION: &[&str] = &[
    "sandbox-root-prefix",
    "repository-root-prefix",
    "python-executable-path",
];

/// Only these explicit case + JSON-pointer pairs may be normalized during
/// verification. There are currently no measured values in captured cases, so
/// the list is empty. A nested field named `duration` or `timestamp` is never
/// treated as volatile by name alone.
const APPROVED_VOLATILE_JSON_PATHS: &[(&str, &str)] = &[];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDocument {
    pub schema_version: u32,
    pub kind: String,
    pub engine: String,
    pub profile: String,
    pub python: PythonInfo,
    pub provenance: Provenance,
    pub normalization: Vec<String>,
    pub fixtures: Vec<FixtureRecord>,
    pub cases: Vec<CaseRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PythonInfo {
    pub implementation: String,
    pub version: String,
    pub executable: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub harness: String,
    pub source_revision: Option<String>,
    pub worktree_dirty: Option<bool>,
    pub source_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FixtureRecord {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub media_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaseRecord {
    pub id: String,
    pub argv: Vec<String>,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_json: Option<Value>,
    pub files: Vec<FileRecord>,
    pub inputs: Vec<InputRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InputRecord {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// Entry point for the `capture` and `verify` subcommands.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        return Err("capture/verify requires a subcommand".to_string());
    };
    match command.as_str() {
        "capture" => capture_command(rest),
        "verify" => verify_command(rest),
        other => Err(format!("oracle does not handle subcommand '{other}'")),
    }
}

fn capture_command(args: &[String]) -> Result<(), String> {
    let mut engine: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut repo_root: Option<PathBuf> = None;
    let mut python_override: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--engine" => engine = Some(take_value(args, &mut index, "--engine")?),
            "--profile" => profile = Some(take_value(args, &mut index, "--profile")?),
            "--out" => out = Some(PathBuf::from(take_value(args, &mut index, "--out")?)),
            "--repo-root" => {
                repo_root = Some(PathBuf::from(take_value(args, &mut index, "--repo-root")?))
            }
            "--python" => {
                python_override = Some(PathBuf::from(take_value(args, &mut index, "--python")?))
            }
            "-h" | "--help" => {
                println!(
                    "usage: apk-lab-migrate capture --engine python --profile offline --out PATH [--repo-root PATH] [--python PATH]\n\
                     \n\
                     Executes the real Python CLI against synthetic fixtures in an isolated temporary sandbox.\n\
                     The engine must be `python` and the profile must be `offline`."
                );
                return Ok(());
            }
            other => return Err(format!("unexpected capture argument '{other}'")),
        }
        index += 1;
    }

    let engine = engine.ok_or("capture requires --engine python")?;
    if engine != "python" {
        return Err(format!(
            "unsupported capture engine '{engine}'; only 'python' is implemented"
        ));
    }
    let profile = profile.ok_or("capture requires --profile offline")?;
    if profile != "offline" {
        return Err(format!(
            "unsupported capture profile '{profile}'; only 'offline' is implemented"
        ));
    }
    let out = out.ok_or("capture requires --out PATH")?;
    let repo_root = find_repo_root(repo_root.as_deref())?;

    let document = capture_document(&repo_root, python_override.as_deref())?;
    let json = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("failed to serialize capture document: {error}"))?;
    if out == Path::new("-") {
        println!("{json}");
    } else {
        if let Some(parent) = out.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create output directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&out, format!("{json}\n"))
            .map_err(|error| format!("failed to write {}: {error}", out.display()))?;
        println!(
            "wrote {} ({} cases, {} fixtures)",
            out.display(),
            document.cases.len(),
            document.fixtures.len()
        );
    }
    Ok(())
}

fn verify_command(args: &[String]) -> Result<(), String> {
    let mut expected: Option<PathBuf> = None;
    let mut actual: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--expected" => {
                expected = Some(PathBuf::from(take_value(args, &mut index, "--expected")?))
            }
            "--actual" => actual = Some(PathBuf::from(take_value(args, &mut index, "--actual")?)),
            "-h" | "--help" => {
                println!(
                    "usage: apk-lab-migrate verify --expected PATH --actual PATH\n\
                     \n\
                     Compares two capture documents, normalizing only the named volatile classes.\n\
                     Exit status is nonzero when any observable result differs."
                );
                return Ok(());
            }
            other => return Err(format!("unexpected verify argument '{other}'")),
        }
        index += 1;
    }

    let expected_path = expected.ok_or("verify requires --expected PATH")?;
    let actual_path = actual.ok_or("verify requires --actual PATH")?;
    let expected = read_document(&expected_path)?;
    let actual = read_document(&actual_path)?;
    verify_documents(&expected, &actual)?;
    println!(
        "verified {} case(s) and {} fixture(s)",
        expected.cases.len(),
        expected.fixtures.len()
    );
    Ok(())
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn read_document(path: &Path) -> Result<CaptureDocument, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn find_repo_root(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("invalid --repo-root {}: {error}", path.display()))?;
        if !canonical.join("tools/apk-lab/apk_lab/cli.py").is_file() {
            return Err(format!(
                "--repo-root {} does not contain tools/apk-lab/apk_lab/cli.py",
                canonical.display()
            ));
        }
        return Ok(canonical);
    }

    let mut cursor = std::env::current_dir()
        .map_err(|error| format!("cannot determine current directory: {error}"))?;
    loop {
        if cursor.join("tools/apk-lab/apk_lab/cli.py").is_file() {
            return Ok(cursor);
        }
        if !cursor.pop() {
            return Err(
                "could not locate repository root containing tools/apk-lab/apk_lab/cli.py; \
                 pass --repo-root"
                    .to_string(),
            );
        }
    }
}

fn resolve_python(repo_root: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(path) = std::env::var_os("APK_LAB_PYTHON") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let venv = repo_root.join("tools/apk-lab/.venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from("python3")
}

struct CaptureContext {
    sandbox: PathBuf,
    repo_root: PathBuf,
    python: PathBuf,
    fake_sdk: PathBuf,
    missing_sdk: PathBuf,
    empty_path: PathBuf,
}

fn capture_document(
    repo_root: &Path,
    python_override: Option<&Path>,
) -> Result<CaptureDocument, String> {
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-migration-oracle-")
        .tempdir()
        .map_err(|error| format!("failed to create isolated sandbox: {error}"))?;
    let sandbox_path = sandbox.path().to_path_buf();

    let fake_sdk = sandbox_path.join("fake-sdk");
    let missing_sdk = sandbox_path.join("missing-sdk");
    let empty_path = sandbox_path.join("empty-path");
    let home = sandbox_path.join("home");
    let temp = sandbox_path.join("tmp");
    for dir in [&fake_sdk, &missing_sdk, &empty_path, &home, &temp] {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }

    let python = resolve_python(repo_root, python_override);
    let python_info = probe_python(&python, repo_root, &sandbox_path)?;

    let fixtures_dir = sandbox_path.join("fixtures");
    create_fixtures(&fixtures_dir, &fake_sdk)?;

    let context = CaptureContext {
        sandbox: sandbox_path,
        repo_root: repo_root.to_path_buf(),
        python,
        fake_sdk,
        missing_sdk,
        empty_path,
    };

    let fixture_records = collect_fixture_records(&context)?;
    let fixture_index = build_fixture_index(&fixture_records);

    let cases = case_specs()
        .into_iter()
        .map(|spec| run_case(&spec, &context, &fixture_index, &home, &temp))
        .collect::<Result<Vec<_>, String>>()?;
    validate_capture_invariants(&cases, &fixture_records)?;

    let provenance = capture_provenance(repo_root)?;

    Ok(CaptureDocument {
        schema_version: SCHEMA_VERSION,
        kind: CAPTURE_KIND.to_string(),
        engine: "python".to_string(),
        profile: "offline".to_string(),
        python: python_info,
        provenance,
        normalization: NORMALIZATION.iter().map(|item| item.to_string()).collect(),
        fixtures: fixture_records,
        cases,
    })
}

fn probe_python(python: &Path, repo_root: &Path, sandbox: &Path) -> Result<PythonInfo, String> {
    let probe = "import json,sys\n\
                 if sys.version_info < (3, 12):\n\
                 \x20\x20\x20\x20print(json.dumps({'error': 'python 3.12+ required'}))\n\
                 \x20\x20\x20\x20sys.exit(9)\n\
                 print(json.dumps({'implementation': sys.implementation.name, 'version': '%d.%d.%d' % sys.version_info[:3]}))";
    let output = Command::new(python)
        .args(["-B", "-c", probe])
        .env_clear()
        .env("PYTHONPATH", repo_root.join("tools/apk-lab"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("HOME", sandbox.join("home"))
        .env("TMPDIR", sandbox.join("tmp"))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("failed to execute Python '{}': {error}", python.display()))?;

    if !output.status.success() {
        return Err(format!(
            "Python '{}' cannot run apk-lab (expected 3.12+): {}",
            python.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid Python probe output: {error}"))?;
    let implementation = value
        .get("implementation")
        .and_then(Value::as_str)
        .ok_or("Python probe did not report implementation")?;
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .ok_or("Python probe did not report version")?;
    Ok(PythonInfo {
        implementation: implementation.to_string(),
        version: version.to_string(),
        executable: "<python>".to_string(),
    })
}

struct CaseSpec {
    id: &'static str,
    argv: Vec<&'static str>,
    environment: CaseEnvironment,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaseEnvironment {
    Standard,
    MissingTool,
}

fn case_specs() -> Vec<CaseSpec> {
    vec![
        CaseSpec {
            id: "asm-ret-hex",
            argv: vec!["asm", "ret", "--format", "hex"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "asm-branch-kotlin",
            argv: vec![
                "asm",
                "bl 0x3c98ce4",
                "--pc",
                "0x1000",
                "--format",
                "kotlin",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "asm-invalid-register",
            argv: vec!["asm", "ret x99"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "validate-smali-valid",
            argv: vec![
                "validate-smali",
                "--snippet",
                "const/4 v0, 0x1\nreturn v0",
                "--locals",
                "2",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "validate-smali-invalid",
            argv: vec![
                "validate-smali",
                "--snippet",
                "invoke-virtual {v9}, Lx;->y()V",
                "--locals",
                "1",
                "--params",
                "0",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-json-stdout",
            argv: vec!["inspect", "../../fixtures/base.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-wide-version-code",
            argv: vec!["inspect", "../../fixtures/wide.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-json-file",
            argv: vec![
                "inspect",
                "../../fixtures/base.apk",
                "--json",
                "inspect.json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-split-container",
            argv: vec!["inspect", "../../fixtures/app.apkm", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-traversal-rejected",
            argv: vec!["inspect", "../../fixtures/traversal.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-missing-artifact",
            argv: vec!["inspect", "../../fixtures/absent.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-missing-tool",
            argv: vec!["inspect", "../../fixtures/base.apk", "--json"],
            environment: CaseEnvironment::MissingTool,
        },
        CaseSpec {
            id: "strings-json-stdout",
            argv: vec!["strings", "../../fixtures/base.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "strings-case-sensitive",
            argv: vec![
                "strings",
                "../../fixtures/base.apk",
                "--case-sensitive",
                "-q",
                "Special",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "strings-unicode-human",
            argv: vec![
                "strings",
                "../../fixtures/base.apk",
                "--case-sensitive",
                "-q",
                "Δ",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "strings-json-file",
            argv: vec![
                "strings",
                "../../fixtures/app.apkm",
                "-q",
                "Δ",
                "--json",
                "strings.json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "manifest-json",
            argv: vec!["manifest", "../../fixtures/base.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "manifest-provider-json",
            argv: vec![
                "manifest",
                "../../fixtures/base.apk",
                "--type",
                "provider",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "manifest-human-unicode",
            argv: vec!["manifest", "../../fixtures/base.apk"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "compare-json",
            argv: vec![
                "compare",
                "../../fixtures/base.apk",
                "../../fixtures/base2.apk",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "usage-invalid-option",
            argv: vec!["inspect", "--bogus"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "clean-valid-workspace-run",
            argv: vec![
                "--workspace-root",
                ".apk-lab",
                "clean",
                "--run",
                ".apk-lab/com.example.synthetic/2147483647-abcdefabcdef/0123456789ab",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "clean-malformed-marker-rejected",
            argv: vec![
                "--workspace-root",
                ".apk-lab",
                "clean",
                "--run",
                ".apk-lab/com.example.synthetic/2147483647-abcdefabcdef/0123456789ab",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "archive-absolute-path-rejected",
            argv: vec!["inspect", "../../fixtures/absolute.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "archive-duplicate-entry-rejected",
            argv: vec!["inspect", "../../fixtures/duplicate.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "archive-truncated-rejected",
            argv: vec!["inspect", "../../fixtures/truncated.apk", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "inspect-nested-split-container",
            argv: vec!["inspect", "../../fixtures/nested.apkm", "--json"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "unity-gameobject-json",
            argv: vec![
                "unity",
                "../../fixtures/unity.apk",
                "--gameobject",
                "Button_HelpCenter",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "unity-gameobject-missing",
            argv: vec![
                "unity",
                "../../fixtures/unity.apk",
                "--gameobject",
                "Button_Missing",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "il2cpp-symbol-json",
            argv: vec![
                "il2cpp",
                "../../fixtures/il2cpp.apk",
                "--query",
                "OpenShop",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "il2cpp-corrupt-table",
            argv: vec![
                "il2cpp",
                "../../fixtures/il2cpp-corrupt.apk",
                "--query",
                "OpenShop",
                "--json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "res-type-name-query",
            argv: vec![
                "res",
                "../../fixtures/base.apk",
                "--query",
                "@drawable/icon",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "res-unicode-path-query",
            argv: vec![
                "res",
                "../../fixtures/base.apk",
                "--query",
                "res/drawable-xxhdpi/εικόνα.webp",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "res-invalid-query",
            argv: vec!["res", "../../fixtures/base.apk", "--query", "../escape.png"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "compatibility-ci-matrix",
            argv: vec![
                "module:apk_lab.compatibility_ci",
                "matrix",
                "--patches-list",
                "../../fixtures/patches-list.json",
                "--output-file",
                "matrix.out",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "morphe-independent-boolean",
            argv: vec![
                "script:morphe-independent-boolean",
                "../../fixtures/patches-list.json",
            ],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "resource-dump-boundaries",
            argv: vec!["script:resource-dump-boundaries"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "adb-log-boundaries",
            argv: vec!["script:adb-log-boundaries"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "tty-non-tty-diff",
            argv: vec!["script:tty-non-tty-diff"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "broken-pipe-success",
            argv: vec!["script:broken-pipe-success", "../../fixtures/base.apk"],
            environment: CaseEnvironment::Standard,
        },
        CaseSpec {
            id: "ci-callback-persistence",
            argv: vec![
                "script:ci-callback-persistence",
                "../../fixtures/patches-list.json",
            ],
            environment: CaseEnvironment::Standard,
        },
    ]
}

fn required_case_ids() -> Vec<&'static str> {
    case_specs().into_iter().map(|spec| spec.id).collect()
}

fn validate_capture_invariants(
    cases: &[CaseRecord],
    fixtures: &[FixtureRecord],
) -> Result<(), String> {
    let required = required_case_ids();
    let mut seen = BTreeSet::new();
    for case in cases {
        if !seen.insert(case.id.as_str()) {
            return Err(format!("capture contains duplicate case '{}'", case.id));
        }
    }
    for id in &required {
        if !seen.iter().any(|seen_id| seen_id == id) {
            return Err(format!("capture is missing required case '{id}'"));
        }
    }
    for id in &seen {
        if !required.iter().any(|required_id| required_id == id) {
            return Err(format!("capture contains non-contract case '{id}'"));
        }
    }

    for fixture_name in [
        "base.apk",
        "wide.apk",
        "base2.apk",
        "split_config.arm64_v8a.apk",
        "app.apkm",
        "nested.apkm",
        "unity.apk",
        "il2cpp.apk",
        "il2cpp-corrupt.apk",
        "patches-list.json",
        "absolute.apk",
        "duplicate.apk",
        "truncated.apk",
        "traversal.apk",
    ] {
        if !fixtures
            .iter()
            .any(|fixture| fixture.path.ends_with(&format!("/{fixture_name}")))
        {
            return Err(format!("capture is missing fixture '{fixture_name}'"));
        }
    }

    for case in cases {
        validate_case_invariants(case)?;
    }
    Ok(())
}

fn validate_case_invariants(case: &CaseRecord) -> Result<(), String> {
    let expected_error = match case.id.as_str() {
        "asm-invalid-register"
        | "validate-smali-invalid"
        | "inspect-missing-artifact"
        | "inspect-missing-tool"
        | "usage-invalid-option"
        | "clean-malformed-marker-rejected"
        | "il2cpp-corrupt-table"
        | "res-invalid-query" => Some(2),
        "inspect-traversal-rejected"
        | "archive-absolute-path-rejected"
        | "archive-duplicate-entry-rejected"
        | "archive-truncated-rejected" => Some(3),
        _ => None,
    };
    if let Some(expected) = expected_error {
        if case.exit_code != expected {
            return Err(format!(
                "case '{}' expected exit {expected}, got {} (stderr: {})",
                case.id,
                case.exit_code,
                trim_for_error(&case.stderr)
            ));
        }
    } else if case.exit_code != 0 {
        return Err(format!(
            "case '{}' expected success, got exit {} (stderr: {})",
            case.id,
            case.exit_code,
            trim_for_error(&case.stderr)
        ));
    }

    match case.id.as_str() {
        "inspect-json-stdout" => {
            let value = case_json(case)?;
            require_json_eq(case, value, "/versionCode", &serde_json::json!(2147483647))?;
            require_json_eq(case, value, "/signingCertificateSha256", &Value::Null)?;
            require_json_eq(case, value, "/containerType", &serde_json::json!("APK"))?;
        }
        "inspect-wide-version-code" => {
            let value = case_json(case)?;
            require_json_eq(
                case,
                value,
                "/versionCode",
                &serde_json::json!(9_007_199_254_740_993i64),
            )?;
            require(
                case,
                value.get("versionCode").and_then(Value::as_i64) == Some(9_007_199_254_740_993),
                "versionCode must stay an exact 64-bit JSON integer",
            )?;
            require(
                case,
                case.stdout.contains("9007199254740993"),
                "raw versionCode digits were rounded or normalized",
            )?;
        }
        "inspect-split-container" | "inspect-nested-split-container" => {
            let value = case_json(case)?;
            let splits = value
                .get("splits")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("case '{}' lacks splits array", case.id))?;
            require(case, splits.len() == 2, "expected exactly two splits")?;
            require(
                case,
                splits
                    .iter()
                    .any(|split| split.get("isBase").and_then(Value::as_bool) == Some(true)),
                "expected one base split",
            )?;
            require(
                case,
                splits.iter().any(|split| {
                    split.get("splitName").and_then(Value::as_str) == Some("config.arm64_v8a")
                }),
                "expected the config.arm64_v8a split",
            )?;
        }
        "inspect-json-file" => {
            let value = case_json_file(case, "/inspect.json")?;
            require_json_eq(case, &value, "/versionCode", &serde_json::json!(2147483647))?;
        }
        "strings-json-stdout" | "strings-case-sensitive" => {
            let value = case_json(case)?;
            require(
                case,
                value.as_array().map(|items| !items.is_empty()) == Some(true),
                "expected at least one string match",
            )?;
        }
        "strings-json-file" => {
            let value = case_json_file(case, "/strings.json")?;
            require(
                case,
                value.as_array().map(|items| !items.is_empty()) == Some(true),
                "expected at least one string match in the JSON file",
            )?;
        }
        "strings-unicode-human" => {
            require(
                case,
                case.stdout.contains('Δ'),
                "lost Unicode string output",
            )?;
        }
        "manifest-provider-json" => {
            let value = case_json(case)?;
            let items = value
                .as_array()
                .ok_or_else(|| format!("case '{}' did not emit a JSON array", case.id))?;
            require(
                case,
                items.len() == 1
                    && items[0].get("type").and_then(Value::as_str) == Some("provider")
                    && items[0].get("exported").and_then(Value::as_bool) == Some(false),
                "expected one non-exported provider",
            )?;
        }
        "manifest-human-unicode" => {
            require(
                case,
                case.stdout.contains("Δραστηριότητα"),
                "lost Unicode component output",
            )?;
        }
        "unity-gameobject-json" => {
            let value = case_json(case)?;
            let items = value
                .as_array()
                .ok_or_else(|| format!("case '{}' did not emit a JSON array", case.id))?;
            require(
                case,
                items.len() == 1
                    && items[0].get("name").and_then(Value::as_str) == Some("Button_HelpCenter")
                    && items[0].get("is_active_value").and_then(Value::as_u64) == Some(0),
                "expected the synthetic inactive GameObject",
            )?;
        }
        "unity-gameobject-missing" => {
            let value = case_json(case)?;
            require(
                case,
                value.as_array().map(|items| items.is_empty()) == Some(true),
                "expected no matching GameObject",
            )?;
        }
        "il2cpp-symbol-json" => {
            let value = case_json(case)?;
            let items = value
                .as_array()
                .ok_or_else(|| format!("case '{}' did not emit a JSON array", case.id))?;
            require(
                case,
                items.len() == 1
                    && items[0].get("type").and_then(Value::as_str) == Some("BlackjackApplication")
                    && items[0].get("method").and_then(Value::as_str) == Some("OpenShop")
                    && items[0].get("parameters_count").and_then(Value::as_u64) == Some(1),
                "expected the synthetic IL2CPP symbol",
            )?;
        }
        "res-type-name-query" => {
            require(
                case,
                case.stdout.contains("drawable/icon"),
                "resource type/name query did not match",
            )?;
        }
        "res-unicode-path-query" => {
            require(
                case,
                case.stdout.contains("εικόνα"),
                "resource Unicode path query did not match",
            )?;
        }
        "compatibility-ci-matrix" => {
            require(
                case,
                case.stdout.contains("packages=[\"com.example.synthetic\"]"),
                "CI matrix did not select the synthetic package",
            )?;
            let content = case_file_content(case, "/matrix.out")
                .ok_or("CI matrix did not write its declared output file")?;
            require(
                case,
                content.contains("com.example.synthetic"),
                "CI matrix output file lacks the package",
            )?;
        }
        "clean-valid-workspace-run" => {
            require(
                case,
                case.files
                    .iter()
                    .any(|file| file.path.ends_with("/keep.txt")),
                "clean removed the unmanaged sentinel",
            )?;
            require(
                case,
                !case.files.iter().any(|file| {
                    file.path.ends_with("/payload.bin") || file.path.ends_with("/.marker.json")
                }),
                "clean left part of the managed run behind",
            )?;
        }
        "clean-malformed-marker-rejected" => {
            require(
                case,
                case.files
                    .iter()
                    .any(|file| file.path.ends_with("/.marker.json")),
                "malformed marker was removed despite rejection",
            )?;
        }
        "morphe-independent-boolean" => {
            let value = case_json(case)?;
            let patch_cases = value
                .get("patches")
                .and_then(Value::as_array)
                .and_then(|patches| patches.first())
                .and_then(|patch| patch.get("cases"))
                .and_then(Value::as_array)
                .ok_or("Morphe probe did not emit patch cases")?;
            require(
                case,
                patch_cases.len() == 3,
                "expected default plus two independent boolean boundary cases",
            )?;
            let alpha: Vec<Value> = patch_cases
                .iter()
                .map(|item| {
                    item.get("options")
                        .and_then(|options| options.get("alpha"))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            let beta: Vec<Value> = patch_cases
                .iter()
                .map(|item| {
                    item.get("options")
                        .and_then(|options| options.get("beta"))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            require(
                case,
                alpha == vec![Value::Bool(false), Value::Bool(true), Value::Bool(false)],
                "alpha boolean boundary was not inverted independently",
            )?;
            require(
                case,
                beta == vec![Value::Bool(true), Value::Bool(true), Value::Bool(false)],
                "beta boolean boundary was not inverted independently",
            )?;
        }
        "resource-dump-boundaries" => {
            let value = case_json(case)?;
            require(
                case,
                value
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|items| items.len() == 3)
                    == Some(true),
                "resource dump parser changed the expected entry count",
            )?;
            require(
                case,
                value.get("entries").and_then(Value::as_array).map(|items| {
                    items.iter().any(|item| {
                        item.get("value")
                            .and_then(Value::as_str)
                            .map(|value| value.contains("Appli Δ"))
                            == Some(true)
                    })
                }) == Some(true),
                "resource dump parser lost the Unicode scalar value",
            )?;
            require(
                case,
                value
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().all(|item| item.get("error").is_some()))
                    == Some(true),
                "invalid resource queries were not all rejected",
            )?;
        }
        "adb-log-boundaries" => {
            let value = case_json(case)?;
            require_json_eq(
                case,
                value,
                "/devices/2/state",
                &serde_json::json!("unauthorized"),
            )?;
            require_json_eq(case, value, "/pids/invalid", &Value::Null)?;
            require_json_eq(
                case,
                value,
                "/records/0/reason_name",
                &serde_json::json!("ANR"),
            )?;
            require_json_eq(
                case,
                value,
                "/records/1/reason_name",
                &serde_json::json!("CRASH_NATIVE"),
            )?;
        }
        "tty-non-tty-diff" => {
            let value = case_json(case)?;
            let tty = value.get("tty").and_then(Value::as_str).unwrap_or("");
            let plain = value.get("nonTty").and_then(Value::as_str).unwrap_or("");
            require(case, tty.contains("\u{1b}["), "TTY output lacks ANSI color")?;
            require(
                case,
                !plain.contains("\u{1b}["),
                "non-TTY output contains ANSI color",
            )?;
            require(
                case,
                plain.contains("+new value"),
                "non-TTY output lost diff content",
            )?;
            require_json_eq(case, value, "/ttyExit", &serde_json::json!(0))?;
            require_json_eq(case, value, "/nonTtyExit", &serde_json::json!(0))?;
            require_json_eq(case, value, "/ttyIsatty", &serde_json::json!(true))?;
            require_json_eq(case, value, "/nonTtyIsatty", &serde_json::json!(false))?;
            require_json_eq(case, value, "/ttyColored", &serde_json::json!(true))?;
            require_json_eq(case, value, "/nonTtyColored", &serde_json::json!(false))?;
            require_json_eq(case, value, "/ttySource", &serde_json::json!("os.openpty"))?;
            require_json_eq(
                case,
                value,
                "/nonTtySource",
                &serde_json::json!("subprocess.PIPE"),
            )?;
            let tty_bytes = value
                .get("ttyBytesHex")
                .and_then(Value::as_str)
                .unwrap_or("");
            require(
                case,
                tty_bytes.contains("1b5b"),
                "captured PTY bytes lack the ANSI escape sequence",
            )?;
        }
        "broken-pipe-success" => {
            let value = case_json(case)?;
            require_json_eq(case, value, "/code", &serde_json::json!(0))?;
            require_json_eq(case, value, "/processExit", &serde_json::json!(0))?;
            require_json_eq(case, value, "/raised", &serde_json::json!(true))?;
            require_json_eq(case, value, "/errnoName", &serde_json::json!("EPIPE"))?;
            require_json_eq(case, value, "/isatty", &serde_json::json!(false))?;
            require_json_eq(
                case,
                value,
                "/shutdownFlushFailure",
                &serde_json::json!(false),
            )?;
            require_json_eq(
                case,
                value,
                "/source",
                &serde_json::json!("os.pipe consumer-closed"),
            )?;
            let writes = value
                .get("writes")
                .and_then(Value::as_array)
                .ok_or("broken-pipe case did not record attempted writes")?;
            let captured: String = writes
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("");
            require(
                case,
                captured.contains("Αρχείο") && captured.contains('Δ'),
                "broken-pipe case did not attempt the real Unicode output",
            )?;
            require(
                case,
                value
                    .get("attemptedBytesHex")
                    .and_then(Value::as_str)
                    .is_some_and(|hex| !hex.is_empty()),
                "broken-pipe case did not record attempted UTF-8 bytes",
            )?;
        }
        "ci-callback-persistence" => {
            let value = case_json(case)?;
            require_json_eq(case, value, "/success/startCode", &serde_json::json!(0))?;
            require_json_eq(case, value, "/success/runCode", &serde_json::json!(0))?;
            require_json_eq(case, value, "/success/marker", &serde_json::json!("ok"))?;
            require_json_eq(
                case,
                value,
                "/success/submission/requestId",
                &serde_json::json!("req-123"),
            )?;
            require_json_eq(
                case,
                value,
                "/success/submission/results/0/status",
                &serde_json::json!("compatible"),
            )?;
            require(
                case,
                value
                    .pointer("/success/requests")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items.iter().any(|item| {
                            item.get("path").and_then(Value::as_str)
                                == Some("/api/compatibility-runs")
                        }) && items.iter().any(|item| {
                            item.get("path").and_then(Value::as_str)
                                == Some("/api/compatibility-results")
                        }) && items.iter().all(|item| {
                            item.get("authorization").and_then(Value::as_str)
                                == Some("Bearer test-secret")
                        })
                    })
                    == Some(true),
                "successful callback did not reach both local HTTP endpoints with authorization",
            )?;
            require_json_eq(case, value, "/failure/runCode", &serde_json::json!(4))?;
            require_json_eq(case, value, "/failure/marker", &Value::Null)?;
            require_json_eq(
                case,
                value,
                "/failure/submission/requestId",
                &serde_json::json!("req-fail"),
            )?;
            require(
                case,
                case.files
                    .iter()
                    .any(|file| file.path.ends_with("/callback.marker")),
                "successful callback marker was not persisted",
            )?;
            require(
                case,
                !case
                    .files
                    .iter()
                    .any(|file| file.path.ends_with("/failure.marker")),
                "failed callback persisted a success marker",
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn case_json(case: &CaseRecord) -> Result<&Value, String> {
    case.stdout_json
        .as_ref()
        .ok_or_else(|| format!("case '{}' did not emit parseable JSON", case.id))
}

fn case_file_content<'a>(case: &'a CaseRecord, suffix: &str) -> Option<&'a str> {
    case.files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .and_then(|file| file.content.as_deref())
}

fn case_json_file(case: &CaseRecord, suffix: &str) -> Result<Value, String> {
    let content = case_file_content(case, suffix)
        .ok_or_else(|| format!("case '{}' did not write {suffix}", case.id))?;
    serde_json::from_str(content)
        .map_err(|error| format!("case '{}' wrote invalid JSON to {suffix}: {error}", case.id))
}

fn require(case: &CaseRecord, condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(format!("case '{}': {message}", case.id))
    }
}

fn require_json_eq(
    case: &CaseRecord,
    value: &Value,
    pointer: &str,
    expected: &Value,
) -> Result<(), String> {
    let actual = value.pointer(pointer);
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(format!(
            "case '{}' JSON pointer {pointer}: expected {}, got {}",
            case.id,
            expected,
            actual
                .map(Value::to_string)
                .unwrap_or_else(|| "<missing>".to_string())
        ))
    }
}

fn trim_for_error(text: &str) -> String {
    let trimmed = text.trim();
    let mut result: String = trimmed.chars().take(240).collect();
    if result.len() < trimmed.len() {
        result.push('…');
    }
    result
}

fn prepare_case(case_id: &str, case_root: &Path) -> Result<(), String> {
    if case_id != "clean-valid-workspace-run" && case_id != "clean-malformed-marker-rejected" {
        return Ok(());
    }
    let workspace = case_root.join(".apk-lab");
    let run = workspace
        .join("com.example.synthetic")
        .join("2147483647-abcdefabcdef")
        .join("0123456789ab");
    fs::create_dir_all(&run)
        .map_err(|error| format!("failed to create workspace case {}: {error}", run.display()))?;
    let schema_version = if case_id == "clean-malformed-marker-rejected" {
        999
    } else {
        1
    };
    let marker = serde_json::json!({
        "schemaVersion": schema_version,
        "inputCanonicalPath": "<sandbox>/fixtures/base.apk",
        "inputSha256": "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
        "command": "inspect",
        "configDigest": "0123456789ab",
        "toolVersions": {},
        "createdAt": "2026-01-01T00:00:00+00:00",
    });
    fs::write(
        run.join(".marker.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&marker)
                .map_err(|error| format!("failed to serialize workspace marker: {error}"))?
        ),
    )
    .map_err(|error| format!("failed to write workspace marker: {error}"))?;
    fs::write(run.join("payload.bin"), b"synthetic workspace payload")
        .map_err(|error| format!("failed to write workspace payload: {error}"))?;
    fs::write(workspace.join("keep.txt"), b"kept outside the managed run")
        .map_err(|error| format!("failed to write workspace sentinel: {error}"))?;
    Ok(())
}

fn run_case(
    spec: &CaseSpec,
    context: &CaptureContext,
    fixture_index: &BTreeMap<String, FixtureRecord>,
    home: &Path,
    temp: &Path,
) -> Result<CaseRecord, String> {
    let case_root = context.sandbox.join("cases").join(spec.id);
    fs::create_dir_all(&case_root)
        .map_err(|error| format!("failed to create {}: {error}", case_root.display()))?;
    prepare_case(spec.id, &case_root)?;

    let mut command = Command::new(&context.python);
    command.arg("-B");
    if let Some(script) = script_for(spec.id) {
        command.arg("-c").arg(script).args(&spec.argv[1..]);
    } else if let Some(module) = spec.argv[0].strip_prefix("module:") {
        command.arg("-m").arg(module).args(&spec.argv[1..]);
    } else {
        command.arg("-m").arg("apk_lab.cli").args(&spec.argv);
    }
    command
        .current_dir(&case_root)
        .env_clear()
        .env("PYTHONPATH", context.repo_root.join("tools/apk-lab"))
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("TMPDIR", temp)
        .env("TZ", "UTC")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match spec.environment {
        CaseEnvironment::Standard => {
            command
                .env("ANDROID_HOME", &context.fake_sdk)
                .env("PATH", std::env::var_os("PATH").unwrap_or_default());
        }
        CaseEnvironment::MissingTool => {
            command
                .env("ANDROID_HOME", &context.missing_sdk)
                .env("PATH", &context.empty_path);
        }
    }

    for name in [
        "APKEEP_ACCOUNTS",
        "GOOPDL_ACCOUNT_EMAIL",
        "GOOPDL_AAS_TOKEN",
        "R2_ACCOUNT_ID",
        "R2_ACCESS_KEY_ID",
        "R2_SECRET_ACCESS_KEY",
        "ANDROID_SERIAL",
    ] {
        command.env_remove(name);
    }

    let output = command
        .output()
        .map_err(|error| format!("failed to execute Python for case '{}': {error}", spec.id))?;
    let exit_code = output.status.code().ok_or_else(|| {
        format!(
            "Python process for case '{}' terminated by a signal without an exit code",
            spec.id
        )
    })?;

    let normalizer = Normalizer::new(&context.sandbox, &context.repo_root);
    let stdout = normalizer.text(&String::from_utf8_lossy(&output.stdout));
    let stderr = normalizer.text(&String::from_utf8_lossy(&output.stderr));
    let stdout_json = parse_stdout_json(spec.id, &stdout);

    let files = collect_case_files(&case_root, &normalizer)?;
    let inputs = collect_case_inputs(&spec.argv, fixture_index);

    Ok(CaseRecord {
        id: spec.id.to_string(),
        argv: spec.argv.iter().map(|value| value.to_string()).collect(),
        exit_code,
        stdout,
        stderr,
        stdout_json,
        files,
        inputs,
    })
}

fn parse_stdout_json(case_id: &str, stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let mut value: Value = serde_json::from_str(trimmed).ok()?;
    apply_approved_volatile(case_id, &mut value);
    Some(value)
}

struct Normalizer {
    replacements: Vec<(String, &'static str)>,
}

impl Normalizer {
    fn new(sandbox: &Path, repo_root: &Path) -> Self {
        let mut replacements = Vec::new();
        for path in [
            sandbox.to_path_buf(),
            sandbox
                .canonicalize()
                .unwrap_or_else(|_| sandbox.to_path_buf()),
        ] {
            let text = path.to_string_lossy().to_string();
            if !text.is_empty() {
                replacements.push((text, "<sandbox>"));
            }
        }
        replacements.push((repo_root.to_string_lossy().to_string(), "<repo>"));
        replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0.len()));
        replacements.dedup_by(|left, right| left.0 == right.0);
        Self { replacements }
    }

    fn text(&self, input: &str) -> String {
        let mut output = input.to_string();
        for (needle, replacement) in &self.replacements {
            if !needle.is_empty() {
                output = output.replace(needle.as_str(), replacement);
            }
        }
        output
    }
}

fn collect_case_files(root: &Path, normalizer: &Normalizer) -> Result<Vec<FileRecord>, String> {
    let mut files = Vec::new();
    walk_files(root, root, &mut files)?;
    let mut records = files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| format!("{} escaped {}", path.display(), root.display()))?;
            let bytes = fs::read(&path).map_err(|error| {
                format!("failed to read side-effect {}: {error}", path.display())
            })?;
            let (sha256, size, content) = match String::from_utf8(bytes) {
                Ok(text) => {
                    let normalized = normalizer.text(&text);
                    (
                        sha256_hex(normalized.as_bytes()),
                        normalized.len() as u64,
                        Some(normalized),
                    )
                }
                Err(error) => {
                    let bytes = error.into_bytes();
                    (sha256_hex(&bytes), bytes.len() as u64, None)
                }
            };
            Ok(FileRecord {
                path: format!(
                    "<sandbox>/cases/{}",
                    relative.to_string_lossy().replace('\\', "/")
                ),
                sha256,
                size,
                content,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    records.sort_by_key(|record| record.path.clone());
    Ok(records)
}

fn walk_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|error| format!("failed to read directory {}: {error}", current.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to read entry under {}: {error}", root.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "side-effect tree contains a symlink at {}; refusing to capture it",
                path.display()
            ));
        }
        if file_type.is_dir() {
            if entry.file_name().to_string_lossy().as_ref() == "__pycache__" {
                continue;
            }
            walk_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn collect_case_inputs(
    argv: &[&str],
    fixture_index: &BTreeMap<String, FixtureRecord>,
) -> Vec<InputRecord> {
    let mut names = BTreeSet::new();
    for argument in argv {
        if let Some(name) = argument.rsplit('/').next() {
            if fixture_index.contains_key(name) {
                names.insert(name.to_string());
            }
        }
    }
    names
        .into_iter()
        .filter_map(|name| fixture_index.get(&name))
        .map(|record| InputRecord {
            path: record.path.clone(),
            sha256: record.sha256.clone(),
            size: record.size,
        })
        .collect()
}

fn build_fixture_index(records: &[FixtureRecord]) -> BTreeMap<String, FixtureRecord> {
    let mut index = BTreeMap::new();
    for record in records {
        if let Some(name) = record.path.rsplit('/').next() {
            index.insert(name.to_string(), record.clone());
        }
    }
    index
}

fn collect_fixture_records(context: &CaptureContext) -> Result<Vec<FixtureRecord>, String> {
    let normalizer = Normalizer::new(&context.sandbox, &context.repo_root);
    let mut paths = Vec::new();
    walk_files(&context.sandbox, &context.sandbox, &mut paths)?;
    let mut records = paths
        .into_iter()
        .filter(|path| {
            path.starts_with(context.sandbox.join("fixtures"))
                || path.starts_with(&context.fake_sdk)
        })
        .map(|path| {
            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read fixture {}: {error}", path.display()))?;
            let relative = path
                .strip_prefix(&context.sandbox)
                .map_err(|_| format!("{} escaped sandbox", path.display()))?;
            let name = relative.to_string_lossy().replace('\\', "/");
            Ok(FixtureRecord {
                path: normalizer.text(&format!("<sandbox>/{name}")),
                sha256: sha256_hex(&bytes),
                size: bytes.len() as u64,
                media_kind: match path.extension().and_then(|value| value.to_str()) {
                    Some("apk") | Some("apkm") | Some("xapk") | Some("apks") => {
                        "android-container".to_string()
                    }
                    _ if name.contains("build-tools") => "test-executable".to_string(),
                    _ => "synthetic-input".to_string(),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    records.sort_by_key(|record| record.path.clone());
    Ok(records)
}

fn capture_provenance(repo_root: &Path) -> Result<Provenance, String> {
    let source_revision = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "HEAD"])
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty());

    let worktree_dirty = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "status",
            "--porcelain",
            "--",
            "tools/apk-lab/apk_lab",
            "tools/apk-lab/pyproject.toml",
            "tools/apk-lab/tools.lock.json",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());

    let source_digest = source_tree_digest(repo_root)?;

    Ok(Provenance {
        harness: format!("apk-lab-migrate {}", env!("CARGO_PKG_VERSION")),
        source_revision,
        worktree_dirty,
        source_digest,
    })
}

fn source_tree_digest(repo_root: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let package_dir = repo_root.join("tools/apk-lab");
    let mut paths = Vec::new();
    let source_dir = package_dir.join("apk_lab");
    if source_dir.is_dir() {
        collect_source_files(&source_dir, &mut paths)?;
    }
    for extra in ["pyproject.toml", "tools.lock.json"] {
        let path = package_dir.join(extra);
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    for path in paths {
        let relative = path
            .strip_prefix(repo_root)
            .map_err(|_| format!("{} escaped the repository", path.display()))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to hash source {}: {error}", path.display()))?;
        hasher.update(&bytes);
        hasher.update([0u8]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_source_files(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|error| format!("failed to read {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read {}: {error}", root.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            if entry.file_name().to_string_lossy().as_ref() == "__pycache__" {
                continue;
            }
            collect_source_files(&path, paths)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) != Some("pyc")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn apply_approved_volatile(case_id: &str, value: &mut Value) {
    for (approved_case, pointer) in APPROVED_VOLATILE_JSON_PATHS {
        if *approved_case == case_id {
            if let Some(slot) = value.pointer_mut(pointer) {
                *slot = Value::Null;
            }
        }
    }
}

/// Compares two capture documents. Returns a single human-readable error that
/// lists every substantive difference found (bounded for readability).
pub fn verify_documents(
    expected: &CaptureDocument,
    actual: &CaptureDocument,
) -> Result<(), String> {
    let mut differences = Vec::new();

    for (label, document) in [("expected", expected), ("actual", actual)] {
        if document.schema_version != SCHEMA_VERSION {
            differences.push(format!(
                "{label}: unsupported schemaVersion {} (required {})",
                document.schema_version, SCHEMA_VERSION
            ));
        }
        if document.kind != CAPTURE_KIND {
            differences.push(format!(
                "{label}: invalid kind '{}' (required '{}')",
                document.kind, CAPTURE_KIND
            ));
        }
        if document.engine != "python" {
            differences.push(format!(
                "{label}: invalid engine '{}' (required 'python')",
                document.engine
            ));
        }
        if document.profile != "offline" {
            differences.push(format!(
                "{label}: invalid profile '{}' (required 'offline')",
                document.profile
            ));
        }
        if document.fixtures.is_empty() {
            differences.push(format!("{label}: fixtures must not be empty"));
        }
        let mandatory = required_case_ids();
        let present: BTreeMap<String, usize> =
            document
                .cases
                .iter()
                .fold(BTreeMap::new(), |mut counts, case| {
                    *counts.entry(case.id.clone()).or_insert(0) += 1;
                    counts
                });
        for required in &mandatory {
            match present.get(*required) {
                None => differences.push(format!("{label}: missing mandatory case '{required}'")),
                Some(count) if *count > 1 => differences.push(format!(
                    "{label}: duplicate mandatory case '{required}' ({count} occurrences)"
                )),
                Some(_) => {}
            }
        }
        for case_id in present.keys() {
            if !mandatory.contains(&case_id.as_str()) {
                differences.push(format!("{label}: unexpected non-contract case '{case_id}'"));
            }
        }
    }

    if expected.schema_version != actual.schema_version {
        differences.push(format!(
            "schemaVersion: expected {}, actual {}",
            expected.schema_version, actual.schema_version
        ));
    }
    if expected.kind != actual.kind {
        differences.push(format!(
            "kind: expected '{}', actual '{}'",
            expected.kind, actual.kind
        ));
    }
    if expected.engine != actual.engine {
        differences.push(format!(
            "engine: expected '{}', actual '{}'",
            expected.engine, actual.engine
        ));
    }
    if expected.profile != actual.profile {
        differences.push(format!(
            "profile: expected '{}', actual '{}'",
            expected.profile, actual.profile
        ));
    }
    if expected.python != actual.python {
        differences.push(format!(
            "python: expected {:?}, actual {:?}",
            expected.python, actual.python
        ));
    }
    if expected.provenance != actual.provenance {
        differences.push(format!(
            "provenance: expected {:?}, actual {:?}",
            expected.provenance, actual.provenance
        ));
    }
    if expected.normalization != actual.normalization {
        differences.push(format!(
            "normalization: expected {:?}, actual {:?}",
            expected.normalization, actual.normalization
        ));
    }

    if expected.fixtures.len() != actual.fixtures.len() {
        differences.push(format!(
            "fixtures: expected {} entries, actual {}",
            expected.fixtures.len(),
            actual.fixtures.len()
        ));
    }
    for (index, (left, right)) in expected
        .fixtures
        .iter()
        .zip(actual.fixtures.iter())
        .enumerate()
    {
        if left != right {
            differences.push(format!(
                "fixtures[{index}]: expected {left:?}, actual {right:?}"
            ));
        }
    }

    if expected.cases.len() != actual.cases.len() {
        differences.push(format!(
            "cases: expected {} entries, actual {}",
            expected.cases.len(),
            actual.cases.len()
        ));
    }
    for (index, (left, right)) in expected.cases.iter().zip(actual.cases.iter()).enumerate() {
        compare_case(index, left, right, &mut differences);
        if differences.len() >= 25 {
            differences.push("additional differences truncated".to_string());
            break;
        }
    }

    if differences.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "oracle verification failed with {} difference(s):\n  - {}",
            differences.len(),
            differences.join("\n  - ")
        ))
    }
}

fn compare_case(
    index: usize,
    expected: &CaseRecord,
    actual: &CaseRecord,
    differences: &mut Vec<String>,
) {
    let label = format!("cases[{index}] '{}'", expected.id);
    if expected.id != actual.id {
        differences.push(format!(
            "{label}: id expected '{}', actual '{}'",
            expected.id, actual.id
        ));
    }
    if expected.argv != actual.argv {
        differences.push(format!(
            "{label}: argv expected {:?}, actual {:?}",
            expected.argv, actual.argv
        ));
    }
    if expected.exit_code != actual.exit_code {
        differences.push(format!(
            "{label}: exitCode expected {}, actual {}",
            expected.exit_code, actual.exit_code
        ));
    }
    if expected.stdout != actual.stdout {
        differences.push(format!(
            "{label}: stdout differs ({} vs {} bytes)",
            expected.stdout.len(),
            actual.stdout.len()
        ));
    }
    if expected.stderr != actual.stderr {
        differences.push(format!(
            "{label}: stderr differs ({} vs {} bytes)",
            expected.stderr.len(),
            actual.stderr.len()
        ));
    }
    let mut expected_json = expected.stdout_json.clone();
    let mut actual_json = actual.stdout_json.clone();
    if let Some(value) = expected_json.as_mut() {
        apply_approved_volatile(&expected.id, value);
    }
    if let Some(value) = actual_json.as_mut() {
        apply_approved_volatile(&expected.id, value);
    }
    if expected_json != actual_json {
        let mut json_differences = Vec::new();
        match (&expected_json, &actual_json) {
            (Some(left), Some(right)) => {
                compare_json(left, right, "stdoutJson", &mut json_differences)
            }
            (None, Some(right)) => json_differences.push(format!(
                "stdoutJson: expected <missing>, actual {}",
                compact(right)
            )),
            (Some(left), None) => json_differences.push(format!(
                "stdoutJson: expected {}, actual <missing>",
                compact(left)
            )),
            (None, None) => {}
        }
        if json_differences.is_empty() {
            differences.push(format!("{label}: stdoutJson differs"));
        } else {
            for difference in json_differences {
                differences.push(format!("{label}: {difference}"));
            }
        }
    }

    if expected.files.len() != actual.files.len() {
        differences.push(format!(
            "{label}: files expected {} entries, actual {}",
            expected.files.len(),
            actual.files.len()
        ));
    }
    for (file_index, (left, right)) in expected.files.iter().zip(actual.files.iter()).enumerate() {
        if left.path != right.path {
            differences.push(format!(
                "{label}: files[{file_index}] path expected '{}', actual '{}'",
                left.path, right.path
            ));
        } else if left.sha256 != right.sha256 || left.size != right.size {
            differences.push(format!(
                "{label}: files[{file_index}] '{}' content hash/size changed ({} / {} vs {} / {})",
                left.path, left.sha256, left.size, right.sha256, right.size
            ));
        } else if left.content != right.content {
            differences.push(format!(
                "{label}: files[{file_index}] '{}' normalized content differs",
                left.path
            ));
        }
    }

    if expected.inputs != actual.inputs {
        differences.push(format!(
            "{label}: inputs expected {:?}, actual {:?}",
            expected.inputs, actual.inputs
        ));
    }
}

fn compare_json(expected: &Value, actual: &Value, path: &str, differences: &mut Vec<String>) {
    if differences.len() >= 10 {
        return;
    }
    match (expected, actual) {
        (Value::Null, Value::Null) => {}
        (Value::Null, other) => {
            differences.push(format!("{path}: expected null, actual {}", compact(other)))
        }
        (other, Value::Null) => {
            differences.push(format!("{path}: expected {}, actual null", compact(other)))
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                differences.push(format!(
                    "{path}: array length expected {}, actual {}",
                    left.len(),
                    right.len()
                ));
            }
            for (index, (left_item, right_item)) in left.iter().zip(right.iter()).enumerate() {
                compare_json(
                    left_item,
                    right_item,
                    &format!("{path}[{index}]"),
                    differences,
                );
            }
        }
        (Value::Object(left), Value::Object(right)) => {
            for (key, left_item) in left {
                match right.get(key) {
                    Some(right_item) => {
                        compare_json(left_item, right_item, &format!("{path}.{key}"), differences);
                    }
                    None => differences.push(format!(
                        "{path}.{key}: expected {}, actual <missing>",
                        compact(left_item)
                    )),
                }
            }
            for key in right.keys() {
                if !left.contains_key(key) {
                    let value = right
                        .get(key)
                        .expect("key was just obtained from the same map");
                    differences.push(format!(
                        "{path}.{key}: expected <missing>, actual {}",
                        compact(value)
                    ));
                }
            }
        }
        (left, right) if left == right => {}
        (left, right) => differences.push(format!(
            "{path}: expected {}, actual {}",
            compact(left),
            compact(right)
        )),
    }
}

fn compact(value: &Value) -> String {
    let text = value.to_string();
    let mut truncated: String = text.chars().take(120).collect();
    if truncated.len() < text.len() {
        truncated.push('…');
    }
    truncated
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn create_fixtures(fixtures: &Path, fake_sdk: &Path) -> Result<(), String> {
    fs::create_dir_all(fixtures)
        .map_err(|error| format!("failed to create fixtures directory: {error}"))?;

    let base_dex = build_dex(
        &[
            "hello world",
            "Αρχείο-Δοκιμής",
            "SpecialCase",
            "specialcase",
            "callback-marker=ready",
            "alpha",
            "beta",
        ],
        3,
        7,
    );
    let second_dex = build_dex(&["second dex value", "Δεύτερο"], 1, 2);
    let split_dex = build_dex(&["split-only value", "παράρτημα"], 1, 2);

    let base_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.synthetic\"/>" as &[u8],
        ),
        ("classes.dex", base_dex.as_slice()),
        ("classes2.dex", second_dex.as_slice()),
        ("assets/unicode-\u{e9}.txt", b"unicode asset" as &[u8]),
        ("assets/callback.txt", b"callback-marker=ready" as &[u8]),
        ("lib/arm64-v8a/libnative.so", b"\x7fELF synthetic" as &[u8]),
        ("res/values/strings.xml", b"<resources/>" as &[u8]),
        (
            "res/drawable-xxhdpi/\u{3b5}\u{3b9}\u{3ba}\u{3cc}\u{3bd}\u{3b1}.webp",
            b"synthetic webp" as &[u8],
        ),
        ("resources.arsc", b"synthetic resources table" as &[u8]),
    ];
    write_zip(&fixtures.join("base.apk"), &base_entries)?;
    write_zip(&fixtures.join("wide.apk"), &base_entries)?;

    let split_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest split=\"config.arm64_v8a\"/>" as &[u8],
        ),
        ("classes.dex", split_dex.as_slice()),
        ("lib/arm64-v8a/libsplit.so", b"\x7fELF split" as &[u8]),
    ];
    write_zip(&fixtures.join("split_config.arm64_v8a.apk"), &split_entries)?;

    let base2_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.synthetic\"/>" as &[u8],
        ),
        ("classes.dex", base_dex.as_slice()),
        ("assets/version.txt", b"older synthetic fixture" as &[u8]),
    ];
    write_zip(&fixtures.join("base2.apk"), &base2_entries)?;

    let base_bytes = fs::read(fixtures.join("base.apk"))
        .map_err(|error| format!("failed to read base fixture: {error}"))?;
    let split_bytes = fs::read(fixtures.join("split_config.arm64_v8a.apk"))
        .map_err(|error| format!("failed to read split fixture: {error}"))?;
    let container_entries: Vec<(&str, &[u8])> = vec![
        ("base.apk", base_bytes.as_slice()),
        ("split_config.arm64_v8a.apk", split_bytes.as_slice()),
    ];
    write_zip(&fixtures.join("app.apkm"), &container_entries)?;

    let nested_entries: Vec<(&str, &[u8])> = vec![
        ("splits/base.apk", base_bytes.as_slice()),
        ("splits/split_config.arm64_v8a.apk", split_bytes.as_slice()),
    ];
    write_zip(&fixtures.join("nested.apkm"), &nested_entries)?;

    let traversal_entries: Vec<(&str, &[u8])> =
        vec![("../escaped.txt", b"escape attempt" as &[u8])];
    write_zip(&fixtures.join("traversal.apk"), &traversal_entries)?;

    let absolute_entries: Vec<(&str, &[u8])> =
        vec![("/absolute.txt", b"absolute path attempt" as &[u8])];
    write_zip(&fixtures.join("absolute.apk"), &absolute_entries)?;

    let duplicate_entries: Vec<(&str, &[u8])> = vec![
        ("duplicate.txt", b"first copy" as &[u8]),
        ("duplicate.txt", b"second copy" as &[u8]),
    ];
    write_zip(&fixtures.join("duplicate.apk"), &duplicate_entries)?;

    fs::write(
        fixtures.join("truncated.apk"),
        b"PK\x03\x04truncated archive",
    )
    .map_err(|error| format!("failed to write truncated fixture: {error}"))?;

    let unity_assets = build_gameobject_bytes("Button_HelpCenter", 0, 3, 32);
    let unity_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.synthetic\"/>" as &[u8],
        ),
        (
            "assets/bin/Data/sharedassets0.assets",
            unity_assets.as_slice(),
        ),
    ];
    write_zip(&fixtures.join("unity.apk"), &unity_entries)?;

    let il2cpp_metadata = build_il2cpp_metadata(29, None, None)?;
    let il2cpp_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.synthetic\"/>" as &[u8],
        ),
        (
            "assets/bin/Data/Managed/Metadata/global-metadata.dat",
            il2cpp_metadata.as_slice(),
        ),
        ("lib/arm64-v8a/libil2cpp.so", b"\x7fELF il2cpp" as &[u8]),
    ];
    write_zip(&fixtures.join("il2cpp.apk"), &il2cpp_entries)?;

    let corrupt_metadata = build_il2cpp_metadata(29, Some(33), None)?;
    let corrupt_entries: Vec<(&str, &[u8])> = vec![
        (
            "AndroidManifest.xml",
            b"<manifest package=\"com.example.synthetic\"/>" as &[u8],
        ),
        (
            "assets/bin/Data/Managed/Metadata/global-metadata.dat",
            corrupt_metadata.as_slice(),
        ),
    ];
    write_zip(&fixtures.join("il2cpp-corrupt.apk"), &corrupt_entries)?;

    let patches_list = serde_json::json!({
        "version": "test-1",
        "patches": [
            {
                "name": "Boolean Patch",
                "description": "Synthetic independent boolean options",
                "default": true,
                "dependencies": ["Base Patch"],
                "options": [
                    {
                        "key": "alpha",
                        "title": "Alpha",
                        "description": "First independent boolean",
                        "required": false,
                        "default": false,
                        "type": "Boolean"
                    },
                    {
                        "key": "beta",
                        "title": "Beta",
                        "description": "Second independent boolean",
                        "required": false,
                        "default": true,
                        "type": "Boolean"
                    },
                    {
                        "key": "mode",
                        "title": "Mode",
                        "description": "Non-boolean option",
                        "required": false,
                        "default": "fast",
                        "type": "String"
                    }
                ],
                "compatiblePackages": [
                    {
                        "name": "Synthetic App",
                        "packageName": "com.example.synthetic",
                        "apkFileType": "APK",
                        "targets": [
                            {"version": "9.9.9", "versionCode": 999}
                        ]
                    }
                ]
            }
        ]
    });
    fs::write(
        fixtures.join("patches-list.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&patches_list)
                .map_err(|error| format!("failed to serialize patches list: {error}"))?
        ),
    )
    .map_err(|error| format!("failed to write patches list fixture: {error}"))?;

    let script = fake_aapt2_script();
    let tool_dir = fake_sdk.join("build-tools/36.0.0");
    fs::create_dir_all(&tool_dir)
        .map_err(|error| format!("failed to create fake SDK {}: {error}", tool_dir.display()))?;
    let tool_path = tool_dir.join("aapt2");
    fs::write(&tool_path, script)
        .map_err(|error| format!("failed to write fake aapt2: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&tool_path)
            .map_err(|error| format!("failed to stat fake aapt2: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&tool_path, permissions)
            .map_err(|error| format!("failed to chmod fake aapt2: {error}"))?;
    }

    let apksigner_path = tool_dir.join("apksigner");
    fs::write(&apksigner_path, "#!/bin/sh\nexit 1\n")
        .map_err(|error| format!("failed to write fake apksigner: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&apksigner_path)
            .map_err(|error| format!("failed to stat fake apksigner: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&apksigner_path, permissions)
            .map_err(|error| format!("failed to chmod fake apksigner: {error}"))?;
    }

    Ok(())
}

fn fake_aapt2_script() -> &'static str {
    r#"#!/bin/sh
set -eu
mode=${1:-}
sub=${2:-}
apk=
for argument in "$@"; do apk=$argument; done
base=${apk##*/}
version_code=2147483647
version_name='1.0-Δ'
case "$base" in
  wide.apk) version_code=9007199254740993; version_name='2.0-wide-Δ' ;;
  base2.apk) version_code=2147483646; version_name='0.9-Δ' ;;
esac
if [ "$mode" != "dump" ]; then
  echo "fake aapt2 only supports dump" >&2
  exit 64
fi
if [ "$sub" = "badging" ]; then
  split=
  case "$base" in
    *config*) split=" split='config.arm64_v8a'" ;;
  esac
  printf "package: name='com.example.synthetic' versionCode='%s' versionName='%s'%s\n" "$version_code" "$version_name" "$split"
  printf "sdkVersion:'21'\n"
  printf "targetSdkVersion:'34'\n"
  printf "uses-permission: name='android.permission.INTERNET'\n"
  printf "uses-permission: name='android.permission.POST_NOTIFICATIONS'\n"
  exit 0
fi
if [ "$sub" = "xmltree" ]; then
  cat <<'XML'
E: manifest (line=2)
  A: http://schemas.android.com/apk/res/android:package="com.example.synthetic" (Raw: "com.example.synthetic")
  E: application (line=3)
    E: activity (line=4)
      A: http://schemas.android.com/apk/res/android:name="com.example.synthetic.MainActivity" (Raw: "com.example.synthetic.MainActivity")
      A: http://schemas.android.com/apk/res/android:exported="true" (Raw: "true")
      E: intent-filter (line=6)
        E: action (line=7)
          A: http://schemas.android.com/apk/res/android:name="android.intent.action.MAIN" (Raw: "android.intent.action.MAIN")
    E: activity (line=9)
      A: http://schemas.android.com/apk/res/android:name="com.example.synthetic.Δραστηριότητα" (Raw: "com.example.synthetic.Δραστηριότητα")
      A: http://schemas.android.com/apk/res/android:exported="false" (Raw: "false")
    E: service (line=12)
      A: http://schemas.android.com/apk/res/android:name="com.example.synthetic.SyncService" (Raw: "com.example.synthetic.SyncService")
      A: http://schemas.android.com/apk/res/android:permission="com.example.synthetic.permission.SYNC" (Raw: "com.example.synthetic.permission.SYNC")
    E: receiver (line=15)
      A: http://schemas.android.com/apk/res/android:name="com.example.synthetic.BootReceiver" (Raw: "com.example.synthetic.BootReceiver")
      A: http://schemas.android.com/apk/res/android:exported="true" (Raw: "true")
    E: provider (line=18)
      A: http://schemas.android.com/apk/res/android:name="com.example.synthetic.DataProvider" (Raw: "com.example.synthetic.DataProvider")
      A: http://schemas.android.com/apk/res/android:authorities="com.example.synthetic.data" (Raw: "com.example.synthetic.data")
      A: http://schemas.android.com/apk/res/android:exported="false" (Raw: "false")
XML
  exit 0
fi
if [ "$sub" = "resources" ]; then
  cat <<'RES'
Package name=com.example.synthetic id=7f
  type string id=01 entryCount=2
    resource 0x7f010001 string/app_name
      () "Synthetic Δ App"
      (fr) "Appli Δ"
    resource malformed-not-a-resource line ignored
  type drawable id=02 entryCount=1
    resource 0x7f08028e drawable/icon
      (xxhdpi) (file) res/drawable-xxhdpi/icon.webp type=webp
  type drawable id=03 entryCount=1
    resource 0x7f08028f drawable/εικόνα
      (xxhdpi) (file) res/drawable-xxhdpi/εικόνα.webp type=webp
RES
  exit 0
fi
echo "fake aapt2 only supports badging, resources, and xmltree" >&2
exit 64
"#
}

fn script_for(case_id: &str) -> Option<&'static str> {
    match case_id {
        "morphe-independent-boolean" => Some(SCRIPT_MORPHE_BOOLEAN),
        "resource-dump-boundaries" => Some(SCRIPT_RESOURCE_DUMP),
        "adb-log-boundaries" => Some(SCRIPT_ADB_LOG),
        "tty-non-tty-diff" => Some(SCRIPT_TTY_DIFF),
        "broken-pipe-success" => Some(SCRIPT_BROKEN_PIPE),
        "ci-callback-persistence" => Some(SCRIPT_CI_CALLBACK),
        _ => None,
    }
}

const SCRIPT_MORPHE_BOOLEAN: &str = r#"
import json
import sys
from apk_lab.morphe import parse_all_compatible_patches, generate_test_cases

with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
patches = parse_all_compatible_patches("com.example.synthetic", data)
result = []
for patch in patches:
    cases = generate_test_cases(patch)
    result.append(
        {
            "patch": patch.name,
            "cases": [
                {"options": case.options, "dependencies": case.dependencies}
                for case in cases
            ],
        }
    )
print(json.dumps({"patches": result}, sort_keys=True, indent=2, ensure_ascii=False))
"#;

const SCRIPT_RESOURCE_DUMP: &str = r#"
import json
from apk_lab.resources import normalize_resource_query, parse_aapt2_resource_dump

dump_text = """Package name=com.example.synthetic id=7f
  type string id=01 entryCount=2
    resource 0x7f010001 string/app_name
      () "Synthetic Δ App"
      (fr) "Appli Δ"
    resource malformed-not-a-resource line ignored
  type drawable id=02 entryCount=1
    resource 0x7f08028e drawable/icon
      (xxhdpi) (file) res/drawable-xxhdpi/icon.webp type=webp
    nonsense indented line without opening parenthesis
"""
entries = parse_aapt2_resource_dump(dump_text, "base", None)


def describe(query):
    return {
        "kind": query.kind,
        "resource_id": query.resource_id,
        "resource_type": query.resource_type,
        "resource_name": query.resource_name,
        "path": query.path,
        "text_value": query.text_value,
        "package_name": query.package_name,
    }


queries = {}
for raw in ["@drawable/icon", "text:Appli Δ", "0x7f08028e", "res/drawable-xxhdpi/icon.webp"]:
    queries[raw] = describe(normalize_resource_query(raw))
errors = []
for raw in ["../escape.png", "/etc/passwd", "res/icon\x00.png", "   "]:
    try:
        normalize_resource_query(raw)
        errors.append({"query": raw, "error": None})
    except ValueError as error:
        errors.append({"query": raw, "error": str(error)})
print(
    json.dumps(
        {"entries": entries, "queries": queries, "errors": errors},
        sort_keys=True,
        indent=2,
        ensure_ascii=False,
    )
)
"#;

const SCRIPT_ADB_LOG: &str = r#"
import json
from apk_lab.device import (
    extract_logcat_pid,
    parse_adb_devices,
    parse_bounds,
    parse_exit_info_records,
)

devices = parse_adb_devices(
    """* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554	device
emulator-5556	offline
192.168.1.100:5555	unauthorized
"""
)
pids = {
    "epoch": extract_logcat_pid("1710000005.123  1234  1234 E AndroidRuntime: FATAL EXCEPTION: main"),
    "threadtime": extract_logcat_pid("03-09 14:20:05.123  5678  5678 E AndroidRuntime: FATAL"),
    "brief": extract_logcat_pid("E( 9999) Tag: msg"),
    "invalid": extract_logcat_pid("just some random text without pid"),
}
dumpsys = """ApplicationExitInfo #0:
  timestamp=1710000005000
  pid=4321
  realUid=10100
  package=com.example.synthetic
  process=com.example.synthetic
  reason=6 (ANR)
  subreason=34 (BIND APPLICATION ANR)
  status=0
  description=Process failed to complete startup
ApplicationExitInfo #1:
  timestamp=1710000001000
  pid=4320
  realUid=10100
  package=com.example.synthetic
  process=com.example.synthetic
  reason=5 (CRASH_NATIVE)
  subreason=0
  status=139
  description=SIGSEGV
ApplicationExitInfo #2:
  timestamp=1709000000000
  pid=4310
  realUid=10100
  package=com.example.synthetic
  process=com.example.synthetic
  reason=4 (CRASH)
  status=1
  description=java.lang.NullPointerException
"""
records = parse_exit_info_records(dumpsys)
print(
    json.dumps(
        {
            "devices": [
                {"serial": device.serial, "state": device.state} for device in devices
            ],
            "pids": pids,
            "bounds": list(parse_bounds("[0,0][1080,1920]")),
            "records": [
                {
                    "timestamp": record.timestamp,
                    "pid": record.pid,
                    "reason_code": record.reason_code,
                    "reason_name": record.reason_name,
                    "subreason_code": record.subreason_code,
                    "subreason_name": record.subreason_name,
                    "description": record.description,
                }
                for record in records
            ],
        },
        sort_keys=True,
        indent=2,
        ensure_ascii=False,
    )
)
"#;

const SCRIPT_TTY_DIFF: &str = r#"
import errno
import json
import os
import pty
import subprocess
import sys

LINES = [
    "--- a/Example.java",
    "+++ b/Example.java",
    "@@ -1,1 +1,1 @@",
    "-old value",
    "+new value",
    " context line",
]

CHILD = r'''
import json
import os
import sys

from apk_lab.code_inspection import format_colored_diff

lines = json.loads(os.environ["TTY_DIFF_LINES"])
isatty = sys.stdout.isatty()
output = format_colored_diff(lines)
sys.stdout.write(output)
sys.stdout.write("\n")
sys.stdout.flush()
os.write(
    2,
    json.dumps(
        {"isatty": isatty, "colored": "\x1b[" in output},
        sort_keys=True,
    ).encode("utf-8")
    + b"\n",
)
'''


def child_environment():
    blocked = {
        "APKEEP_ACCOUNTS",
        "GOOPDL_ACCOUNT_EMAIL",
        "GOOPDL_AAS_TOKEN",
        "R2_ACCOUNT_ID",
        "R2_ACCESS_KEY_ID",
        "R2_SECRET_ACCESS_KEY",
        "ANDROID_SERIAL",
    }
    environment = {
        key: value for key, value in os.environ.items() if key not in blocked
    }
    environment["PYTHONIOENCODING"] = "utf-8"
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    environment["TTY_DIFF_LINES"] = json.dumps(LINES)
    return environment


def child_evidence(stderr_bytes):
    lines = [line for line in stderr_bytes.decode("utf-8", "replace").splitlines() if line]
    if not lines:
        raise RuntimeError("child emitted no terminal-mode evidence")
    return json.loads(lines[-1])


environment = child_environment()

master_fd, slave_fd = pty.openpty()
tty_process = subprocess.Popen(
    [sys.executable, "-B", "-c", CHILD],
    stdin=subprocess.DEVNULL,
    stdout=slave_fd,
    stderr=subprocess.PIPE,
    env=environment,
)
os.close(slave_fd)
chunks = []
while True:
    try:
        chunk = os.read(master_fd, 4096)
    except OSError as error:
        if error.errno == errno.EIO:
            break
        raise
    if not chunk:
        break
    chunks.append(chunk)
os.close(master_fd)
tty_stderr = tty_process.stderr.read()
tty_exit = tty_process.wait()
tty_bytes = b"".join(chunks)

plain_process = subprocess.run(
    [sys.executable, "-B", "-c", CHILD],
    stdin=subprocess.DEVNULL,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    env=environment,
    check=False,
)
plain_bytes = plain_process.stdout

tty_evidence = child_evidence(tty_stderr)
plain_evidence = child_evidence(plain_process.stderr)
if tty_evidence["isatty"] is not True or not tty_evidence["colored"]:
    raise RuntimeError("real PTY child did not exercise the colored TTY branch")
if plain_evidence["isatty"] is not False or plain_evidence["colored"]:
    raise RuntimeError("real pipe child did not exercise the plain non-TTY branch")

print(
    json.dumps(
        {
            "tty": tty_bytes.decode("utf-8"),
            "nonTty": plain_bytes.decode("utf-8"),
            "ttyExit": tty_exit,
            "nonTtyExit": plain_process.returncode,
            "ttyIsatty": tty_evidence["isatty"],
            "nonTtyIsatty": plain_evidence["isatty"],
            "ttyColored": tty_evidence["colored"],
            "nonTtyColored": plain_evidence["colored"],
            "ttyBytesHex": tty_bytes.hex(),
            "nonTtyBytesHex": plain_bytes.hex(),
            "ttySource": "os.openpty",
            "nonTtySource": "subprocess.PIPE",
        },
        sort_keys=True,
        ensure_ascii=False,
    )
)
"#;

const SCRIPT_BROKEN_PIPE: &str = r#"
import errno
import json
import os
import re
import subprocess
import sys

CHILD = r'''
import json
import os
import sys

from apk_lab.cli import main


class Recorder:
    def __init__(self, stream):
        self.stream = stream
        self.chunks = []
        self.errno = None

    def write(self, value):
        self.chunks.append(value)
        try:
            return self.stream.write(value)
        except BrokenPipeError as error:
            self.errno = error.errno
            raise

    def flush(self):
        try:
            return self.stream.flush()
        except BrokenPipeError as error:
            self.errno = error.errno
            raise

    def isatty(self):
        return self.stream.isatty()

    def fileno(self):
        return self.stream.fileno()

    def writable(self):
        return True

    @property
    def closed(self):
        return self.stream.closed

    @property
    def encoding(self):
        return self.stream.encoding

    @property
    def buffer(self):
        return self.stream.buffer

    def close(self):
        return self.stream.close()


# The recorder only observes the writes main() attempts and forwards them to
# the real pipe. Buffering is left untouched so the process exit code reflects
# the production CLI's own shutdown flush behavior.
recorder = Recorder(sys.stdout)
sys.stdout = recorder
isatty = recorder.isatty()
code = main(["strings", sys.argv[1], "--case-sensitive", "-q", "Δ"])
os.write(
    2,
    b"PIPE_EVIDENCE "
    + json.dumps(
        {
            "code": code,
            "chunks": list(recorder.chunks),
            "errno": recorder.errno,
            "isatty": isatty,
        },
        sort_keys=True,
        ensure_ascii=False,
    ).encode("utf-8")
    + b"\n",
)
sys.exit(code)
'''


def child_environment():
    blocked = {
        "APKEEP_ACCOUNTS",
        "GOOPDL_ACCOUNT_EMAIL",
        "GOOPDL_AAS_TOKEN",
        "R2_ACCOUNT_ID",
        "R2_ACCESS_KEY_ID",
        "R2_SECRET_ACCESS_KEY",
        "ANDROID_SERIAL",
    }
    environment = {
        key: value for key, value in os.environ.items() if key not in blocked
    }
    environment["PYTHONIOENCODING"] = "utf-8"
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    return environment


read_fd, write_fd = os.pipe()
os.close(read_fd)
process = subprocess.Popen(
    [sys.executable, "-B", "-c", CHILD, sys.argv[1]],
    stdin=subprocess.DEVNULL,
    stdout=write_fd,
    stderr=subprocess.PIPE,
    env=child_environment(),
)
os.close(write_fd)
stderr_data = process.stderr.read()
process_exit = process.wait()
text = stderr_data.decode("utf-8", "replace")
evidence = None
for line in text.splitlines():
    if line.startswith("PIPE_EVIDENCE "):
        evidence = json.loads(line[len("PIPE_EVIDENCE "):])
if evidence is None:
    raise RuntimeError(
        "real broken-pipe child produced no evidence; exit {0}: {1}".format(
            process_exit, text[-400:]
        )
    )
match = re.search(r"BrokenPipeError: \[Errno (\d+)\]", text)
errno_value = evidence["errno"]
if errno_value is None and match:
    errno_value = int(match.group(1))
errno_name = errno.errorcode.get(errno_value) if errno_value is not None else None
attempted = "".join(evidence["chunks"])
print(
    json.dumps(
        {
            "code": evidence["code"],
            "processExit": process_exit,
            "raised": errno_name == "EPIPE",
            "errno": errno_value,
            "errnoName": errno_name,
            "writes": evidence["chunks"],
            "attempted": attempted,
            "attemptedBytesHex": attempted.encode("utf-8").hex(),
            "isatty": evidence["isatty"],
            "source": "os.pipe consumer-closed",
            "shutdownFlushFailure": process_exit != evidence["code"],
        },
        sort_keys=True,
        ensure_ascii=False,
    )
)
"#;

const SCRIPT_CI_CALLBACK: &str = r#"
import contextlib
import http.server
import io
import json
import os
import sys
import threading
from pathlib import Path

from apk_lab import compatibility_ci
from apk_lab.fixtures import SlotMetadata
from apk_lab.models import PatchCompatibilityReport

package = "com.example.synthetic"
patches_path = Path(sys.argv[1]).resolve()
patches_data = json.loads(patches_path.read_text(encoding="utf-8"))
work = Path.cwd()
mpp_path = work / "synthetic.mpp"
mpp_path.write_bytes(b"synthetic patch bundle")

metadata = SlotMetadata(
    role="target",
    package_name=package,
    version_name="9.9.9",
    version_code=999,
    container_type="APK",
    sha256="a" * 64,
    signer_sha256=None,
    size_bytes=19,
    acquisition_source="synthetic",
    timestamp="2026-01-01T00:00:00+00:00",
)


class FakeR2:
    def get_slot_metadata(self, pkg, role):
        return metadata if role == "target" else None

    def download_slot(self, pkg, role, destination):
        destination = Path(destination)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(b"synthetic apk bytes")
        return destination


def fake_checker(artifact, mpp, expected_package=None, all_patches=False):
    report = PatchCompatibilityReport(
        artifact_sha256="b" * 64,
        package_name=package,
        version_name="9.9.9",
        version_code=999,
        patch_bundle_version="test-1",
        git_revision="deadbeef",
        tool_versions={"morphe": "synthetic"},
        overall_status="compatible",
        total_cases=3,
        passed_cases=3,
        failed_cases=0,
        test_cases=[],
        failure_reason=None,
    )
    return report, 0


received = []


class CallbackHandler(http.server.BaseHTTPRequestHandler):
    response_status = 200

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length).decode("utf-8", "replace")
        received.append(
            {
                "path": self.path,
                "authorization": self.headers.get("Authorization"),
                "body": json.loads(raw),
            }
        )
        self.send_response(self.response_status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, *args):
        pass


def run_once(status, request_id, submission_name, marker_name, runner_name):
    del received[:]
    handler = type("ConfiguredHandler", (CallbackHandler,), {"response_status": status})
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        worker_url = "http://127.0.0.1:%d" % server.server_address[1]
        submission_path = work / submission_name
        marker_path = work / marker_name
        if submission_path.exists():
            submission_path.unlink()
        if marker_path.exists():
            marker_path.unlink()
        os.environ.update(
            {
                "WORKER_STATUS_URL": worker_url,
                "COMPATIBILITY_STATUS_SECRET": "test-secret",
                "DISPATCH_REQUEST_ID": request_id,
                "GITHUB_SHA": "deadbeef",
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "COMPATIBILITY_SUBMISSION_PATH": str(submission_path),
                "COMPATIBILITY_CALLBACK_MARKER": str(marker_path),
            }
        )
        logs = io.StringIO()
        with contextlib.redirect_stdout(logs), contextlib.redirect_stderr(logs):
            start_code = compatibility_ci.main(
                [
                    "start",
                    "--package",
                    package,
                    "--request-id",
                    request_id,
                    "--git-revision",
                    "deadbeef",
                    "--patches-list",
                    str(patches_path),
                ]
            )
            run_code = compatibility_ci.run_ci_reconcile_and_test(
                package,
                mpp_path,
                work / runner_name,
                r2_mgr=FakeR2(),
                acquirer=None,
                checker=fake_checker,
                poster=None,
                requested_roles=None,
                patches_data=patches_data,
            )
        marker = marker_path.read_text(encoding="utf-8") if marker_path.exists() else None
        submission = (
            json.loads(submission_path.read_text(encoding="utf-8"))
            if submission_path.exists()
            else None
        )
        return {
            "startCode": start_code,
            "runCode": run_code,
            "marker": marker,
            "submission": submission,
            "requests": json.loads(json.dumps(received)),
            "logs": logs.getvalue(),
        }
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


success = run_once(200, "req-123", "submission.json", "callback.marker", "runner")
failure = run_once(500, "req-fail", "failure-submission.json", "failure.marker", "runner-fail")
print(
    json.dumps(
        {"success": success, "failure": failure},
        sort_keys=True,
        indent=2,
        ensure_ascii=False,
    )
)
"#;

fn build_gameobject_bytes(name: &str, is_active: u8, tag: u16, prefix_padding: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; prefix_padding];
    let name_bytes = name.as_bytes();
    buffer.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(name_bytes);
    let padding = (4 - (name_bytes.len() % 4)) % 4;
    buffer.resize(buffer.len() + padding, 0);
    buffer.extend_from_slice(&tag.to_le_bytes());
    buffer.push(is_active);
    buffer.resize(buffer.len() + 16, 0);
    buffer
}

fn build_il2cpp_metadata(
    version: u32,
    corrupt_methods_size: Option<u32>,
    corrupt_types_size: Option<u32>,
) -> Result<Vec<u8>, String> {
    let string_data = b"\x00TestNamespace\x00BlackjackApplication\x00OpenShop\x00OtherMethod\x00";
    let string_offset = 256usize;
    let string_size = string_data.len();

    let methods_data = if version == 29 {
        let mut data = Vec::new();
        // "<iiiiiIHHHH": name_idx, decl_type, return_type, param_start,
        // gen_container, token, flags, iflags, slot, param_count
        for (name_index, parameter_count) in [(36i32, 1u16), (45i32, 0u16)] {
            push_i32(&mut data, name_index);
            push_i32(&mut data, 0);
            push_i32(&mut data, 0);
            push_i32(&mut data, 0);
            push_i32(&mut data, -1);
            push_u32(&mut data, 0);
            push_u16(&mut data, 0);
            push_u16(&mut data, 0);
            push_u16(&mut data, 0);
            push_u16(&mut data, parameter_count);
        }
        data
    } else {
        vec![0u8; 64]
    };

    let methods_offset = string_offset + string_size + 4;
    let methods_size = corrupt_methods_size.unwrap_or(methods_data.len() as u32);
    let types_offset = methods_offset + methods_data.len() + 4;

    let mut type_data = vec![0u8; 88];
    write_i32_at(&mut type_data, 0, 15)?;
    write_i32_at(&mut type_data, 4, 1)?;
    write_i32_at(&mut type_data, 36, 0)?;
    write_u16_at(&mut type_data, 64, 2)?;
    let types_size = corrupt_types_size.unwrap_or(type_data.len() as u32);

    let total_len = types_offset + type_data.len() + 16;
    let mut buffer = vec![0u8; total_len];
    write_u32_at(&mut buffer, 0, 0xfab1_1baf)?;
    write_u32_at(&mut buffer, 4, version)?;
    write_u32_at(&mut buffer, 24, string_offset as u32)?;
    write_u32_at(&mut buffer, 28, string_size as u32)?;
    write_u32_at(&mut buffer, 48, methods_offset as u32)?;
    write_u32_at(&mut buffer, 52, methods_size)?;
    write_u32_at(&mut buffer, 160, types_offset as u32)?;
    write_u32_at(&mut buffer, 164, types_size)?;
    buffer[string_offset..string_offset + string_size].copy_from_slice(string_data);
    buffer[methods_offset..methods_offset + methods_data.len()].copy_from_slice(&methods_data);
    buffer[types_offset..types_offset + type_data.len()].copy_from_slice(&type_data);
    Ok(buffer)
}

fn push_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_i32_at(output: &mut [u8], offset: usize, value: i32) -> Result<(), String> {
    let end = offset + 4;
    let slot = output
        .get_mut(offset..end)
        .ok_or_else(|| format!("IL2CPP fixture offset {offset} out of range"))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u16_at(output: &mut [u8], offset: usize, value: u16) -> Result<(), String> {
    let end = offset + 2;
    let slot = output
        .get_mut(offset..end)
        .ok_or_else(|| format!("IL2CPP fixture offset {offset} out of range"))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32_at(output: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    let end = offset + 4;
    let slot = output
        .get_mut(offset..end)
        .ok_or_else(|| format!("IL2CPP fixture offset {offset} out of range"))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn build_dex(strings: &[&str], class_count: u32, method_count: u32) -> Vec<u8> {
    let header_size = 0x70usize;
    let string_ids_offset = header_size;
    let string_ids_size = strings.len();
    let data_offset = string_ids_offset + string_ids_size * 4;

    let mut string_data = Vec::new();
    let mut offsets = Vec::new();
    for value in strings {
        offsets.push((data_offset + string_data.len()) as u32);
        let bytes = value.as_bytes();
        encode_uleb128((value.chars().count()) as u32, &mut string_data);
        string_data.extend_from_slice(bytes);
        string_data.push(0);
    }

    let mut header = vec![0u8; header_size];
    header[0..4].copy_from_slice(b"dex\n");
    header[4..8].copy_from_slice(b"035\0");
    header[0x38..0x3c].copy_from_slice(&(string_ids_size as u32).to_le_bytes());
    header[0x3c..0x40].copy_from_slice(&(string_ids_offset as u32).to_le_bytes());
    header[0x58..0x5c].copy_from_slice(&method_count.to_le_bytes());
    header[0x60..0x64].copy_from_slice(&class_count.to_le_bytes());

    let mut output = header;
    for offset in offsets {
        output.extend_from_slice(&offset.to_le_bytes());
    }
    output.extend_from_slice(&string_data);
    output
}

fn encode_uleb128(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) -> Result<(), String> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    let mut offset = 0u32;

    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err(format!("zip entry name too long: {name}"));
        }
        let crc = crc32(data);
        let size = data.len() as u32;

        let mut local = Vec::new();
        push_u32(&mut local, 0x0403_4b50);
        push_u16(&mut local, 20);
        push_u16(&mut local, 0);
        push_u16(&mut local, 0);
        push_u16(&mut local, 0);
        push_u16(&mut local, 0x0021);
        push_u32(&mut local, crc);
        push_u32(&mut local, size);
        push_u32(&mut local, size);
        push_u16(&mut local, name_bytes.len() as u16);
        push_u16(&mut local, 0);
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);

        let mut central_entry = Vec::new();
        push_u32(&mut central_entry, 0x0201_4b50);
        push_u16(&mut central_entry, 20);
        push_u16(&mut central_entry, 20);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0x0021);
        push_u32(&mut central_entry, crc);
        push_u32(&mut central_entry, size);
        push_u32(&mut central_entry, size);
        push_u16(&mut central_entry, name_bytes.len() as u16);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0);
        push_u16(&mut central_entry, 0);
        push_u32(&mut central_entry, 0);
        push_u32(&mut central_entry, offset);
        central_entry.extend_from_slice(name_bytes);

        offset += local.len() as u32;
        output.extend_from_slice(&local);
        central.extend_from_slice(&central_entry);
    }

    let central_offset = output.len() as u32;
    output.extend_from_slice(&central);
    let central_size = central.len() as u32;

    push_u32(&mut output, 0x0605_4b50);
    push_u16(&mut output, 0);
    push_u16(&mut output, 0);
    push_u16(&mut output, entries.len() as u16);
    push_u16(&mut output, entries.len() as u16);
    push_u32(&mut output, central_size);
    push_u32(&mut output, central_offset);
    push_u16(&mut output, 0);

    fs::write(path, output).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str, sha256: &str) -> FixtureRecord {
        FixtureRecord {
            path: path.to_string(),
            sha256: sha256.to_string(),
            size: 42,
            media_kind: "android-container".to_string(),
        }
    }

    fn case() -> CaseRecord {
        CaseRecord {
            id: "inspect-json-stdout".to_string(),
            argv: vec!["inspect".to_string(), "fixtures/base.apk".to_string()],
            exit_code: 0,
            stdout: "Αρχείο callback-marker=ready\n".to_string(),
            stderr: String::new(),
            stdout_json: Some(serde_json::json!({
                "packageName": "com.example.synthetic",
                "signingCertificateSha256": null,
                "splits": [
                    {"name": "base", "order": 1},
                    {"name": "config", "order": 2}
                ],
                "components": [
                    {"name": "MainActivity", "exported": true},
                    {"name": "Hidden", "exported": false}
                ],
                "marker": "callback-marker=ready",
                "durationMs": 12
            })),
            files: vec![FileRecord {
                path: "<sandbox>/cases/inspect/inspect.json".to_string(),
                sha256: sha256_hex(b"{\"ok\":true}"),
                size: 11,
                content: Some("{\"ok\":true}".to_string()),
            }],
            inputs: vec![InputRecord {
                path: "<sandbox>/fixtures/base.apk".to_string(),
                sha256: "abc".to_string(),
                size: 42,
            }],
        }
    }

    fn document() -> CaptureDocument {
        let cases = required_case_ids()
            .into_iter()
            .map(|id| {
                let mut record = case();
                record.id = id.to_string();
                record.files[0].path = format!("<sandbox>/cases/{id}/inspect.json");
                record
            })
            .collect();
        CaptureDocument {
            schema_version: SCHEMA_VERSION,
            kind: CAPTURE_KIND.to_string(),
            engine: "python".to_string(),
            profile: "offline".to_string(),
            python: PythonInfo {
                implementation: "cpython".to_string(),
                version: "3.12.0".to_string(),
                executable: "<python>".to_string(),
            },
            provenance: Provenance {
                harness: "apk-lab-migrate 0.1.0".to_string(),
                source_revision: Some("deadbeef".to_string()),
                worktree_dirty: Some(false),
                source_digest: "digest".to_string(),
            },
            normalization: NORMALIZATION.iter().map(|item| item.to_string()).collect(),
            fixtures: vec![fixture("<sandbox>/fixtures/base.apk", "abc")],
            cases,
        }
    }

    #[test]
    fn crc32_matches_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn zip_writer_emits_local_and_central_headers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.apk");
        write_zip(&path, &[("a.txt", b"hello"), ("dir/b.bin", b"\x00\x01")]).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], &b"PK\x03\x04"[..]);
        assert!(bytes.windows(4).any(|window| window == &b"PK\x01\x02"[..]));
        assert!(bytes.windows(4).any(|window| window == &b"PK\x05\x06"[..]));
    }

    #[test]
    fn dex_fixture_has_expected_tables() {
        let dex = build_dex(&["alpha", "Δ"], 3, 7);
        assert_eq!(&dex[0..4], &b"dex\n"[..]);
        assert_eq!(u32::from_le_bytes(dex[0x38..0x3c].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(dex[0x58..0x5c].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(dex[0x60..0x64].try_into().unwrap()), 3);
    }

    #[test]
    fn normalize_only_named_volatile_classes() {
        let sandbox = Path::new("/tmp/apk-lab-migration-oracle-123");
        let repo = Path::new("/repo");
        let normalizer = Normalizer::new(sandbox, repo);
        assert_eq!(
            normalizer.text("at /tmp/apk-lab-migration-oracle-123/fixtures/base.apk from /repo"),
            "at <sandbox>/fixtures/base.apk from <repo>"
        );
        assert_eq!(
            normalizer.text("\u{1b}[31merror\u{1b}[0m"),
            "\u{1b}[31merror\u{1b}[0m"
        );
    }

    #[test]
    fn verify_accepts_identical_documents() {
        verify_documents(&document(), &document()).unwrap();
    }

    #[test]
    fn verify_rejects_empty_document_even_when_self_comparing() {
        let mut empty = document();
        empty.cases.clear();
        let error = verify_documents(&empty, &empty).unwrap_err();
        assert!(error.contains("missing mandatory case"));
    }

    #[test]
    fn verify_rejects_missing_mandatory_case() {
        let expected = document();
        let mut actual = document();
        actual.cases.remove(1);
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("missing mandatory case"));
    }

    #[test]
    fn verify_rejects_invalid_schema_and_kind_even_when_self_comparing() {
        let mut bad_schema = document();
        bad_schema.schema_version = 99;
        let error = verify_documents(&bad_schema, &bad_schema).unwrap_err();
        assert!(error.contains("unsupported schemaVersion"));

        let mut bad_kind = document();
        bad_kind.kind = "other".to_string();
        let error = verify_documents(&bad_kind, &bad_kind).unwrap_err();
        assert!(error.contains("invalid kind"));
    }

    #[test]
    fn verify_rejects_changed_exit_code() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].exit_code = 3;
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("exitCode"));
    }

    #[test]
    fn validate_rejects_rounded_wide_version_code() {
        let mut record = case();
        record.id = "inspect-wide-version-code".to_string();
        record.stdout = "{\"versionCode\": 9007199254740993}\n".to_string();
        let exact = serde_json::json!({"versionCode": 9_007_199_254_740_993i64});
        record.stdout_json = Some(exact);
        validate_case_invariants(&record).unwrap();

        record.stdout = "{\"versionCode\": 9007199254740992}\n".to_string();
        let rounded = serde_json::json!({"versionCode": 9_007_199_254_740_992i64});
        record.stdout_json = Some(rounded);
        let error = validate_case_invariants(&record).unwrap_err();
        assert!(error.contains("versionCode"));
    }

    #[test]
    fn verify_compares_wide_version_code_exactly() {
        fn set_version(document: &mut CaptureDocument, version: i64, raw: &str) {
            let record = document
                .cases
                .iter_mut()
                .find(|case| case.id == "inspect-wide-version-code")
                .unwrap();
            record.stdout = format!("{{\"versionCode\": {raw}}}\n");
            record.stdout_json = Some(serde_json::json!({ "versionCode": version }));
        }

        let mut expected = document();
        set_version(&mut expected, 9_007_199_254_740_993, "9007199254740993");
        let mut actual = expected.clone();
        verify_documents(&expected, &actual).unwrap();

        set_version(&mut actual, 9_007_199_254_740_992, "9007199254740992");
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("versionCode"));
    }

    #[test]
    fn verify_rejects_removed_json_null() {
        let expected = document();
        let mut actual = document();
        actual.cases[0]
            .stdout_json
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("signingCertificateSha256");
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("signingCertificateSha256"));
        assert!(error.contains("missing"));
    }

    #[test]
    fn verify_rejects_changed_archive_content() {
        let expected = document();
        let mut actual = document();
        actual.fixtures[0].sha256 = "different".to_string();
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("fixtures[0]"));
    }

    #[test]
    fn verify_rejects_reordered_significant_results() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout_json.as_mut().unwrap()["splits"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("stdoutJson.splits[0]"));
    }

    #[test]
    fn verify_rejects_omitted_boolean_case() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout_json.as_mut().unwrap()["components"][1]
            .as_object_mut()
            .unwrap()
            .remove("exported");
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("exported"));
    }

    #[test]
    fn verify_rejects_altered_callback_marker() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout_json.as_mut().unwrap()["marker"] =
            serde_json::json!("callback-marker=failed");
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("callback-marker"));
    }

    #[test]
    fn verify_rejects_lost_unicode() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout = "?????? callback-marker=ready\n".to_string();
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("stdout differs"));
    }

    #[test]
    fn verify_rejects_global_duration_key_normalization() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout_json.as_mut().unwrap()["durationMs"] = serde_json::json!(9999);
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("durationMs"));
    }

    #[test]
    fn verify_rejects_unnamed_timing_field_without_normalization() {
        let expected = document();
        let mut actual = document();
        actual.cases[0].stdout_json.as_mut().unwrap()["processingNanos"] = serde_json::json!(123);
        let error = verify_documents(&expected, &actual).unwrap_err();
        assert!(error.contains("processingNanos"));
    }

    #[test]
    fn cli_rejects_unsupported_engine() {
        let error = run(&[
            "capture".to_string(),
            "--engine".to_string(),
            "rust".to_string(),
            "--profile".to_string(),
            "offline".to_string(),
            "--out".to_string(),
            "-".to_string(),
        ])
        .unwrap_err();
        assert!(error.contains("unsupported capture engine"));
    }

    #[test]
    fn cli_rejects_unsupported_profile() {
        let error = run(&[
            "capture".to_string(),
            "--engine".to_string(),
            "python".to_string(),
            "--profile".to_string(),
            "network".to_string(),
            "--out".to_string(),
            "-".to_string(),
        ])
        .unwrap_err();
        assert!(error.contains("unsupported capture profile"));
    }

    #[test]
    fn fixture_hashes_are_stable_across_generation() {
        let dir = tempfile::tempdir().unwrap();
        let fixtures = dir.path().join("fixtures");
        let fake_sdk = dir.path().join("fake-sdk");
        fs::create_dir_all(&fake_sdk).unwrap();
        create_fixtures(&fixtures, &fake_sdk).unwrap();
        let first = fs::read(fixtures.join("base.apk")).unwrap();
        create_fixtures(&fixtures, &fake_sdk).unwrap();
        let second = fs::read(fixtures.join("base.apk")).unwrap();
        assert_eq!(sha256_hex(&first), sha256_hex(&second));
    }

    #[test]
    fn side_effect_tree_detects_content_change() {
        let dir = tempfile::tempdir().unwrap();
        let normalizer = Normalizer::new(dir.path(), Path::new("/repo"));
        let file = dir.path().join("out.json");
        fs::write(&file, "{\"a\":1}").unwrap();
        let first = collect_case_files(dir.path(), &normalizer).unwrap();
        fs::write(&file, "{\"a\":2}").unwrap();
        let second = collect_case_files(dir.path(), &normalizer).unwrap();
        assert_ne!(first[0].sha256, second[0].sha256);
    }

    #[test]
    fn write_zip_rejects_overlong_names() {
        let dir = tempfile::tempdir().unwrap();
        let long_name = "a".repeat(70_000);
        let error =
            write_zip(&dir.path().join("x.apk"), &[(long_name.as_str(), b"x")]).unwrap_err();
        assert!(error.contains("too long"));
    }

    #[test]
    fn read_document_reports_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "{").unwrap();
        let error = read_document(&path).unwrap_err();
        assert!(error.contains("failed to parse"));
    }
}
