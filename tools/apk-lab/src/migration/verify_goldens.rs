//! Python-independent golden verifier.
//!
//! This command executes the *native* `apk-lab` binary against the tiny
//! synthetic fixtures from the parity harness and compares normalized output
//! with a committed golden document. It never imports or executes Python: the
//! only fixtures it uses are built by Rust code, and every child process runs
//! with Python sentinels first on `PATH`.
//!
//! The golden document is deliberately exact. Exit status, normalized stdout,
//! normalized stderr and declared file digests must match byte for byte, so a
//! changed offset, hash, field value or message fails verification. Volatile
//! fields are limited to the explicit declarations in this module (paths,
//! durations and PIDs); nothing else is normalized away.
//!
//! `--update` exists for maintainers to regenerate the document after reviewed
//! behavior changes. It is never used by the normal verification path, and the
//! committed document is still validated for shape, coverage and determinism.

use crate::migration::parity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const GOLDEN_KIND: &str = "apk-lab-native-goldens";
const REPORT_KIND: &str = "apk-lab-golden-verification";
const GOLDEN_VERSION: u32 = 1;
const REPORT_VERSION: u32 = 1;
const DEFAULT_GOLDENS: &str = "migration/goldens/native-goldens.json";
const DEFAULT_RUST_BIN: &str = "target/release/apk-lab";
const PATCHING_PACKAGE: &str = "com.avinside";
const PATCHING_DEVICE: &str = "emulator-5554";

const GROUPS: &[&str] = &["core", "analysis", "patching-device", "acquisition-ci"];

/// Static description of one verifier case. The expected values live in the
/// committed golden document so behavior is reviewable without reading Rust.
struct Case {
    id: &'static str,
    group: &'static str,
    argv: &'static [&'static str],
    /// Case-local environment. Values beginning with `@` resolve inside the
    /// group's run directory.
    env: &'static [(&'static str, &'static str)],
}

const CASES: &[Case] = &[
    // ------------------------------------------------------------------
    // core: help/version plus the inspection boundary
    // ------------------------------------------------------------------
    Case {
        id: "core-help",
        group: "core",
        argv: &["--help"],
        env: &[],
    },
    Case {
        id: "core-version",
        group: "core",
        argv: &["--version"],
        env: &[],
    },
    Case {
        id: "core-inspect-json",
        group: "core",
        argv: &["inspect", "fixtures/core.apk", "--json"],
        env: &[],
    },
    Case {
        id: "core-inspect-human",
        group: "core",
        argv: &["inspect", "fixtures/core.apk"],
        env: &[],
    },
    Case {
        id: "core-inspect-missing",
        group: "core",
        argv: &["inspect", "fixtures/absent.apk"],
        env: &[],
    },
    Case {
        id: "core-inspect-invalid",
        group: "core",
        argv: &["inspect", "fixtures/unknown.apk"],
        env: &[],
    },
    // ------------------------------------------------------------------
    // analysis: inspect/strings/res/manifest, asm/unity/il2cpp/smali
    // ------------------------------------------------------------------
    Case {
        id: "analysis-asm-encode",
        group: "analysis",
        argv: &["asm", "bl 0x3c98ce4", "--pc", "0x1fcf6e8"],
        env: &[],
    },
    Case {
        id: "analysis-asm-range",
        group: "analysis",
        argv: &["asm", "b 0x8000000", "--pc", "0"],
        env: &[],
    },
    Case {
        id: "analysis-strings-unicode",
        group: "analysis",
        argv: &["strings", "fixtures/analysis.apk", "-q", "Grüße", "--json"],
        env: &[],
    },
    Case {
        id: "analysis-strings-malformed",
        group: "analysis",
        argv: &["strings", "fixtures/malformed.apk", "--json"],
        env: &[],
    },
    Case {
        id: "analysis-unity-offset",
        group: "analysis",
        argv: &[
            "unity",
            "fixtures/analysis.apk",
            "--gameobject",
            "Button_HelpCenter",
            "--json",
        ],
        env: &[],
    },
    Case {
        id: "analysis-il2cpp-offset",
        group: "analysis",
        argv: &[
            "il2cpp",
            "fixtures/analysis.apk",
            "--query",
            "OpenShop",
            "--json",
        ],
        env: &[],
    },
    Case {
        id: "analysis-manifest-json",
        group: "analysis",
        argv: &["manifest", "fixtures/analysis.apk", "--json"],
        env: &[],
    },
    Case {
        id: "analysis-res-scalar",
        group: "analysis",
        argv: &[
            "res",
            "fixtures/analysis.apk",
            "--query",
            "@string/app_name",
        ],
        env: &[],
    },
    Case {
        id: "analysis-res-duplicate",
        group: "analysis",
        argv: &[
            "res",
            "fixtures/analysis.apk",
            "--query",
            "@string/duplicate_value",
        ],
        env: &[],
    },
    Case {
        id: "analysis-res-alias",
        group: "analysis",
        argv: &[
            "res",
            "fixtures/analysis.apk",
            "--query",
            "@string/app_alias",
        ],
        env: &[],
    },
    Case {
        id: "analysis-smali-valid",
        group: "analysis",
        argv: &[
            "validate-smali",
            "--snippet",
            "return-void",
            "--locals",
            "0",
        ],
        env: &[],
    },
    Case {
        id: "analysis-smali-invalid",
        group: "analysis",
        argv: &[
            "validate-smali",
            "--snippet",
            "const/4 v2, 1",
            "--locals",
            "1",
        ],
        env: &[],
    },
    // ------------------------------------------------------------------
    // patching-device: check/deploy/monitor/UI, offline fake tools only
    // ------------------------------------------------------------------
    Case {
        id: "patching-check-json",
        group: "patching-device",
        argv: &[
            "check",
            "fixtures/artifact.apk",
            "--mpp",
            "fixtures/patches.mpp",
            "--package",
            PATCHING_PACKAGE,
            "--jobs",
            "1",
            "--json",
        ],
        env: &[],
    },
    Case {
        id: "patching-check-all-json",
        group: "patching-device",
        argv: &[
            "check",
            "fixtures/artifact.apk",
            "--mpp",
            "fixtures/patches.mpp",
            "--package",
            PATCHING_PACKAGE,
            "--all",
            "--force",
            "--jobs",
            "1",
            "--json",
        ],
        env: &[],
    },
    Case {
        id: "patching-check-missing-mpp",
        group: "patching-device",
        argv: &[
            "check",
            "fixtures/artifact.apk",
            "--package",
            PATCHING_PACKAGE,
            "--jobs",
            "1",
        ],
        env: &[],
    },
    Case {
        id: "patching-deploy",
        group: "patching-device",
        argv: &[
            "deploy",
            "fixtures/artifact.apk",
            "--mpp",
            "fixtures/patches.mpp",
            "--package",
            PATCHING_PACKAGE,
            "--device",
            PATCHING_DEVICE,
            "--keystore",
            "fixtures/custom.keystore",
            "--out",
            "out/deploy.apk",
        ],
        env: &[],
    },
    Case {
        id: "patching-deploy-missing-bundle",
        group: "patching-device",
        argv: &[
            "deploy",
            "fixtures/artifact.apk",
            "--package",
            PATCHING_PACKAGE,
            "--device",
            PATCHING_DEVICE,
        ],
        env: &[],
    },
    Case {
        id: "patching-monitor-healthy",
        group: "patching-device",
        argv: &[
            "monitor",
            "--package",
            PATCHING_PACKAGE,
            "--device",
            PATCHING_DEVICE,
            "--timeout",
            "0",
        ],
        env: &[],
    },
    Case {
        id: "patching-monitor-crash",
        group: "patching-device",
        argv: &[
            "monitor",
            "--package",
            PATCHING_PACKAGE,
            "--device",
            PATCHING_DEVICE,
            "--timeout",
            "3",
        ],
        env: &[("FAKE_LIVE_LOG", "@crash.log")],
    },
    Case {
        id: "patching-ui-dump",
        group: "patching-device",
        argv: &["ui", "dump", "--device", PATCHING_DEVICE, "--clickable"],
        env: &[],
    },
    Case {
        id: "patching-ui-dump-json",
        group: "patching-device",
        argv: &[
            "ui",
            "dump",
            "--device",
            PATCHING_DEVICE,
            "--text",
            "save",
            "--json",
        ],
        env: &[],
    },
    Case {
        id: "patching-ui-click",
        group: "patching-device",
        argv: &["ui", "click", "--device", PATCHING_DEVICE, "--text", "save"],
        env: &[],
    },
    Case {
        id: "patching-ui-click-missing",
        group: "patching-device",
        argv: &[
            "ui",
            "click",
            "--device",
            PATCHING_DEVICE,
            "--text",
            "absent",
        ],
        env: &[],
    },
    // ------------------------------------------------------------------
    // acquisition-ci: only offline, deterministic entrypoints
    // ------------------------------------------------------------------
    Case {
        id: "acquire-help",
        group: "acquisition-ci",
        argv: &["acquire", "--help"],
        env: &[],
    },
    Case {
        id: "acquire-invalid-option",
        group: "acquisition-ci",
        argv: &["acquire", "--not-an-option"],
        env: &[],
    },
    Case {
        id: "ci-matrix",
        group: "acquisition-ci",
        argv: &[
            "ci",
            "matrix",
            "--patches-list",
            "fixtures/patches-list.json",
        ],
        env: &[("GITHUB_EVENT_NAME", "workflow_dispatch")],
    },
    Case {
        id: "ci-matrix-missing-list",
        group: "acquisition-ci",
        argv: &[
            "ci",
            "matrix",
            "--patches-list",
            "fixtures/absent-patches-list.json",
        ],
        env: &[("GITHUB_EVENT_NAME", "workflow_dispatch")],
    },
    Case {
        id: "ci-start-missing-secret",
        group: "acquisition-ci",
        argv: &[
            "ci",
            "start",
            "--package",
            "com.example.alpha",
            "--request-id",
            "req-1",
            "--git-revision",
            "abc123",
        ],
        env: &[],
    },
    Case {
        id: "ci-run-missing-bundle",
        group: "acquisition-ci",
        argv: &["ci", "run", "--package", "com.example.alpha"],
        env: &[],
    },
    Case {
        id: "fixtures-info-unconfigured",
        group: "acquisition-ci",
        argv: &["fixtures", "info", "--package", "com.example.alpha"],
        env: &[],
    },
];

/// Cases whose primary output is JSON. The listed pointers are set to
/// `<volatile>` after parsing; no other JSON field is normalized.
const JSON_CASES: &[(&str, &[&str])] = &[
    ("core-inspect-json", &["/filePath"]),
    ("analysis-strings-unicode", &[]),
    ("analysis-strings-malformed", &[]),
    ("analysis-unity-offset", &[]),
    ("analysis-il2cpp-offset", &[]),
    ("analysis-manifest-json", &[]),
    ("patching-check-json", &["/testCases/*/durationSeconds"]),
    ("patching-check-all-json", &["/testCases/*/durationSeconds"]),
    ("patching-ui-dump-json", &[]),
];

/// Human patching output embeds wall-clock durations and process ids.
const TEXT_DURATION_CASES: &[&str] = &["patching-deploy", "patching-monitor-healthy"];

fn is_json_case(id: &str) -> Option<&'static [&'static str]> {
    JSON_CASES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, pointers)| *pointers)
}

fn is_duration_case(id: &str) -> bool {
    TEXT_DURATION_CASES.contains(&id)
}

#[derive(Debug)]
struct Options {
    group: String,
    rust_bin: PathBuf,
    report: Option<String>,
    goldens: PathBuf,
    update: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut group = "all".to_string();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut rust_bin = manifest.join(DEFAULT_RUST_BIN);
    let mut report = None;
    let mut goldens = manifest.join(DEFAULT_GOLDENS);
    let mut update = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "--group" => {
                group = require_value(args, index, "--group")?;
                index += 2;
            }
            "--rust-bin" => {
                rust_bin = PathBuf::from(require_value(args, index, "--rust-bin")?);
                index += 2;
            }
            "--report" => {
                report = Some(require_value(args, index, "--report")?);
                index += 2;
            }
            "--goldens" => {
                goldens = PathBuf::from(require_value(args, index, "--goldens")?);
                index += 2;
            }
            "--update" => {
                update = true;
                index += 1;
            }
            other => return Err(format!("unrecognized option '{other}'")),
        }
    }
    select_groups(&group)?;
    Ok(Some(Options {
        group,
        rust_bin,
        report,
        goldens,
        update,
    }))
}

fn require_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index + 1)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// Resolves `all` or a single group name. Unknown names are rejected before any
/// work starts.
fn select_groups(requested: &str) -> Result<Vec<&'static str>, String> {
    if requested == "all" {
        return Ok(GROUPS.to_vec());
    }
    GROUPS
        .iter()
        .find(|group| **group == requested)
        .map(|group| vec![*group])
        .ok_or_else(|| {
            format!(
                "unknown group '{requested}' (choose from all, {})",
                GROUPS.join(", ")
            )
        })
}

fn selected_cases(groups: &[&str]) -> Vec<&'static Case> {
    CASES
        .iter()
        .filter(|case| groups.contains(&case.group))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GoldenDocument {
    schema_version: u32,
    kind: String,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GoldenCase {
    id: String,
    group: String,
    exit: i64,
    stdout: String,
    stderr: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CaseStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseResult {
    id: String,
    group: String,
    argv: Vec<String>,
    status: CaseStatus,
    expected_exit: i64,
    actual_exit: Option<i64>,
    differences: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    total: usize,
    passed: usize,
    failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyReport {
    schema_version: u32,
    kind: &'static str,
    groups: Vec<String>,
    cases: Vec<CaseResult>,
    summary: Summary,
}

fn build_report(groups: &[&str], cases: Vec<CaseResult>) -> VerifyReport {
    let mut summary = Summary {
        total: cases.len(),
        passed: 0,
        failed: 0,
    };
    for case in &cases {
        match case.status {
            CaseStatus::Passed => summary.passed += 1,
            CaseStatus::Failed => summary.failed += 1,
        }
    }
    VerifyReport {
        schema_version: REPORT_VERSION,
        kind: REPORT_KIND,
        groups: groups.iter().map(|group| (*group).to_string()).collect(),
        cases,
        summary,
    }
}

fn render_report(report: &VerifyReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map(|mut json| {
            json.push('\n');
            json
        })
        .map_err(|error| format!("could not serialize verification report: {error}"))
}

fn load_goldens(path: &Path) -> Result<GoldenDocument, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read goldens '{}': {error}", path.display()))?;
    let document: GoldenDocument = serde_json::from_str(&text)
        .map_err(|error| format!("invalid goldens '{}': {error}", path.display()))?;
    if document.kind != GOLDEN_KIND {
        return Err(format!(
            "invalid goldens '{}': kind must be '{GOLDEN_KIND}'",
            path.display()
        ));
    }
    if document.schema_version != GOLDEN_VERSION {
        return Err(format!(
            "invalid goldens '{}': unsupported schemaVersion {} (expected {GOLDEN_VERSION})",
            path.display(),
            document.schema_version
        ));
    }
    Ok(document)
}

/// Verifies that the committed document covers exactly the selected case
/// inventory and that no selected group is silently empty.
fn validate_inventory(
    document: &GoldenDocument,
    cases: &[&'static Case],
    groups: &[&str],
) -> Result<(), String> {
    if document.cases.is_empty() {
        return Err("goldens document has no cases".to_string());
    }
    let mut expected = BTreeMap::new();
    for case in cases {
        if expected.insert(case.id, case).is_some() {
            return Err(format!("duplicate case id '{}' in case table", case.id));
        }
    }
    if expected.is_empty() {
        return Err("case inventory is empty".to_string());
    }
    for group in groups {
        if !cases.iter().any(|case| case.group == *group) {
            return Err(format!("selected group '{group}' has no cases"));
        }
    }
    let known_ids: BTreeSet<&str> = CASES.iter().map(|case| case.id).collect();
    let mut seen = BTreeSet::new();
    for golden in &document.cases {
        if !GROUPS.contains(&golden.group.as_str()) {
            return Err(format!("golden case '{}': unknown group", golden.group));
        }
        if !known_ids.contains(golden.id.as_str()) {
            return Err(format!(
                "golden case '{}' is not in the case table",
                golden.id
            ));
        }
        if !seen.insert(golden.id.clone()) {
            return Err(format!("golden case '{}' is duplicated", golden.id));
        }
        // Cases from groups outside the selection are retained in the document
        // but not validated by this invocation.
        if let Some(case) = expected.get(golden.id.as_str()) {
            if case.group != golden.group {
                return Err(format!(
                    "golden case '{}': group '{}' does not match '{}'",
                    golden.id, golden.group, case.group
                ));
            }
        }
    }
    for id in expected.keys() {
        if !seen.contains(*id) {
            return Err(format!("golden document is missing case '{id}'"));
        }
    }
    for group in groups {
        if !document.cases.iter().any(|case| case.group == *group) {
            return Err(format!("golden document has no cases for group '{group}'"));
        }
    }
    Ok(())
}

struct Normalizer {
    paths: Vec<(String, String)>,
}

impl Normalizer {
    fn new(sandbox: &Path, run_root: &Path, sdk: &Path, fake_bin: &Path) -> Self {
        let home = sandbox.join("home");
        let temp = sandbox.join("tmp");
        let mut paths = vec![
            (
                sandbox.to_string_lossy().into_owned(),
                "<sandbox>".to_string(),
            ),
            (run_root.to_string_lossy().into_owned(), "<run>".to_string()),
            (sdk.to_string_lossy().into_owned(), "<sdk>".to_string()),
            (
                fake_bin.to_string_lossy().into_owned(),
                "<tools>".to_string(),
            ),
            (home.to_string_lossy().into_owned(), "<home>".to_string()),
            (temp.to_string_lossy().into_owned(), "<temp>".to_string()),
        ];
        // The OS may resolve a temporary root through a symlink (for example
        // `/var` -> `/private/var` on macOS). Normalize both spellings so the
        // committed golden does not depend on the host's canonical path.
        for (path, placeholder) in [
            (sandbox, "<sandbox>"),
            (run_root, "<run>"),
            (sdk, "<sdk>"),
            (fake_bin, "<tools>"),
            (&home, "<home>"),
            (&temp, "<temp>"),
        ] {
            if let Ok(canonical) = fs::canonicalize(path) {
                let text = canonical.to_string_lossy().into_owned();
                if text != path.to_string_lossy() {
                    paths.push((text, placeholder.to_string()));
                }
            }
        }
        // Longest prefixes first so a nested root is not half-replaced.
        paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
        Self { paths }
    }

    fn apply(&self, text: &str) -> String {
        let mut normalized = text.replace("\r\n", "\n");
        for (path, placeholder) in &self.paths {
            normalized = normalized.replace(path, placeholder);
        }
        normalized
    }

    fn case(&self, case: &Case, stdout: &str, stderr: &str) -> (String, String) {
        let mut stdout = self.apply(stdout);
        let stderr = self.apply(stderr);
        if let Some(pointers) = is_json_case(case.id) {
            stdout = normalize_json(&stdout, pointers);
        }
        if is_duration_case(case.id) {
            stdout = normalize_durations(&stdout);
        }
        (stdout, stderr)
    }
}

fn normalize_json(text: &str, pointers: &[&str]) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return text.to_string();
    };
    for pointer in pointers {
        apply_pointer(&mut value, pointer);
    }
    serde_json::to_string(&value).unwrap_or_else(|_| text.to_string())
}

fn apply_pointer(value: &mut Value, pointer: &str) {
    let segments: Vec<&str> = pointer.trim_start_matches('/').split('/').collect();
    apply_segments(value, &segments);
}

fn apply_segments(value: &mut Value, segments: &[&str]) {
    let Some((segment, rest)) = segments.split_first() else {
        *value = Value::String("<volatile>".to_string());
        return;
    };
    match value {
        Value::Object(map) => {
            if let Some(child) = map.get_mut(*segment) {
                apply_segments(child, rest);
            }
        }
        Value::Array(items) => {
            if *segment == "*" {
                for item in items {
                    apply_segments(item, rest);
                }
            } else if let Ok(index) = segment.parse::<usize>() {
                if let Some(item) = items.get_mut(index) {
                    apply_segments(item, rest);
                }
            }
        }
        _ => {}
    }
}

/// Replaces decimal durations with `<duration>` and process ids with `<pid>`.
/// Only cases explicitly listed in `TEXT_DURATION_CASES` use this.
fn normalize_durations(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'.' {
                let mut lookahead = index + 1;
                while lookahead < bytes.len() && bytes[lookahead].is_ascii_digit() {
                    lookahead += 1;
                }
                if lookahead > index + 1 && lookahead < bytes.len() && bytes[lookahead] == b's' {
                    output.push_str("<duration>");
                    index = lookahead + 1;
                    continue;
                }
            }
            output.push_str(&text[start..index]);
            continue;
        }
        output.push(bytes[index] as char);
        index += 1;
    }
    output
}

fn require_executable(path: &Path, flag: &str) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("{flag} '{}': {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{flag} '{}' is not a file", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{flag} '{}' is not executable", path.display()));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read '{}': {error}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// One prepared group sandbox: fixtures plus the environment every case in the
/// group shares.
struct Sandbox {
    root: PathBuf,
    run_root: PathBuf,
    normalizer: Normalizer,
    base_env: BTreeMap<String, String>,
}

fn create_sentinels(fake_bin: &Path, log: &Path) -> Result<(), String> {
    for name in ["python", "python3", "pip", "pip3", "uv", "pytest"] {
        let path = fake_bin.join(name);
        let script = format!(
            "#!/bin/sh\nprintf '%s %s\\n' '{name}' \"$*\" >> '{}'\nexit 97\n",
            log.display()
        );
        fs::write(&path, script)
            .map_err(|error| format!("could not write {name} sentinel: {error}"))?;
        parity::make_executable_for_goldens(&path)?;
    }
    Ok(())
}

fn path_entries(fake_bin: &Path, sdk: &Path) -> String {
    let mut entries = vec![fake_bin.to_path_buf()];
    let build_tools = sdk.join("build-tools/36.0.0");
    if build_tools.is_dir() {
        entries.push(build_tools);
    }
    let platform_tools = sdk.join("platform-tools");
    if platform_tools.is_dir() {
        entries.push(platform_tools);
    }
    entries.push(PathBuf::from("/usr/bin"));
    entries.push(PathBuf::from("/bin"));
    entries
        .iter()
        .map(|entry| entry.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

fn prepare_sandbox(group: &str, root: &Path, project: &Path) -> Result<Sandbox, String> {
    let run_root = root.join("run");
    let sdk = root.join("fake-sdk");
    let fake_bin = root.join("fake-bin");
    let home = root.join("home");
    let temp = root.join("tmp");
    for directory in [&run_root, &sdk, &fake_bin, &home, &temp] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    match group {
        "core" => parity::create_core_golden_fixtures(&run_root, &sdk)?,
        "analysis" => parity::create_analysis_fixtures(&run_root, &sdk, &home, &fake_bin, project)?,
        "patching-device" => {
            parity::create_patching_device_fixtures(&run_root, &sdk, &home, &fake_bin, project)?
        }
        "acquisition-ci" => create_ci_fixtures(&run_root)?,
        other => return Err(format!("unknown group '{other}'")),
    }
    create_sentinels(&fake_bin, &root.join("python-invocations.log"))?;

    let normalizer = Normalizer::new(root, &run_root, &sdk, &fake_bin);
    let mut base_env = BTreeMap::new();
    base_env.insert("PATH".to_string(), path_entries(&fake_bin, &sdk));
    base_env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    base_env.insert("TMPDIR".to_string(), temp.to_string_lossy().into_owned());
    base_env.insert("TZ".to_string(), "UTC".to_string());
    base_env.insert(
        "ANDROID_HOME".to_string(),
        sdk.to_string_lossy().into_owned(),
    );
    base_env.insert(
        "ANDROID_SDK_ROOT".to_string(),
        sdk.to_string_lossy().into_owned(),
    );
    if group == "patching-device" {
        for (name, relative) in [
            ("FAKE_LOG", "calls.log"),
            ("FAKE_BADGING", "badging.txt"),
            ("FAKE_LOGCAT", "logcat.txt"),
            ("FAKE_UI_XML", "ui.xml"),
            ("FAKE_DUMPSYS", "dumpsys.txt"),
            ("FAKE_PATCHED_APK", "fixtures/patched.apk"),
            ("FAKE_INSTALLED", "installed.apk"),
            ("FAKE_PID", "pid.txt"),
            ("FAKE_LIVE_LOG", "empty.log"),
        ] {
            base_env.insert(
                name.to_string(),
                run_root.join(relative).to_string_lossy().into_owned(),
            );
        }
    }
    Ok(Sandbox {
        root: root.to_path_buf(),
        run_root,
        normalizer,
        base_env,
    })
}

const CI_PATCHES_LIST: &str = r#"{
  "version": "1.0.0",
  "patches": [
    {
      "name": "Alpha Patch",
      "compatiblePackages": [
        {"name": "Alpha", "packageName": "com.example.alpha", "targets": [{"version": "1.0.0"}]}
      ]
    },
    {
      "name": "Beta Patch",
      "compatiblePackages": [
        {"name": "Beta", "packageName": "com.example.beta", "targets": [{"version": "2.0.0"}]}
      ]
    }
  ]
}
"#;

fn create_ci_fixtures(run_root: &Path) -> Result<(), String> {
    let fixtures = run_root.join("fixtures");
    fs::create_dir_all(&fixtures)
        .map_err(|error| format!("could not create {}: {error}", fixtures.display()))?;
    fs::write(fixtures.join("patches-list.json"), CI_PATCHES_LIST)
        .map_err(|error| format!("could not write patches-list fixture: {error}"))?;
    Ok(())
}

struct ProcessOutput {
    exit: i64,
    stdout: String,
    stderr: String,
}

fn execute_case(case: &Case, sandbox: &Sandbox, rust_bin: &Path) -> Result<ProcessOutput, String> {
    let mut environment = sandbox.base_env.clone();
    for (name, value) in case.env {
        let resolved = match value.strip_prefix('@') {
            Some(relative) => sandbox
                .run_root
                .join(relative)
                .to_string_lossy()
                .into_owned(),
            None => (*value).to_string(),
        };
        environment.insert((*name).to_string(), resolved);
    }
    let mut command = Command::new(rust_bin);
    command
        .args(case.argv)
        .current_dir(&sandbox.run_root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &environment {
        command.env(name, value);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run '{}': {error}", rust_bin.display()))?;
    Ok(ProcessOutput {
        exit: output.status.code().map(i64::from).unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Files declared as postconditions for a case. The digest is verified whether
/// or not the case exits zero, so a failed run that mutates an output fails.
fn case_files(case: &Case, sandbox: &Sandbox) -> (Vec<&'static str>, BTreeMap<String, String>) {
    let paths: Vec<&'static str> = match case.id {
        "patching-deploy" => vec!["out/deploy.apk"],
        _ => Vec::new(),
    };
    let mut digests = BTreeMap::new();
    for relative in &paths {
        let path = sandbox.run_root.join(relative);
        if let Ok(digest) = sha256_file(&path) {
            digests.insert((*relative).to_string(), digest);
        }
    }
    (paths, digests)
}

fn compare_case(
    golden: &GoldenCase,
    case: &Case,
    output: &ProcessOutput,
    sandbox: &Sandbox,
) -> CaseResult {
    let mut differences = Vec::new();
    if golden.exit != output.exit {
        differences.push(format!(
            "exit: expected {}, got {}",
            golden.exit, output.exit
        ));
    }
    let (stdout, stderr) = sandbox
        .normalizer
        .case(case, &output.stdout, &output.stderr);
    if golden.stdout != stdout {
        differences.push(describe_difference("stdout", &golden.stdout, &stdout));
    }
    if golden.stderr != stderr {
        differences.push(describe_difference("stderr", &golden.stderr, &stderr));
    }
    let (declared, observed) = case_files(case, sandbox);
    for relative in &declared {
        match observed.get(*relative) {
            Some(actual) => match golden.files.get(*relative) {
                Some(expected) if expected == actual => {}
                Some(expected) => differences.push(format!(
                    "file '{relative}': expected sha256 {expected}, got {actual}"
                )),
                None => differences.push(format!(
                    "file '{relative}': no golden digest (got {actual})"
                )),
            },
            None => differences.push(format!("file '{relative}': expected file is missing")),
        }
    }
    for relative in golden.files.keys() {
        if !declared.contains(&relative.as_str()) {
            differences.push(format!(
                "golden file '{relative}' is not a declared postcondition"
            ));
        }
    }
    CaseResult {
        id: golden.id.clone(),
        group: golden.group.clone(),
        argv: case.argv.iter().map(|value| (*value).to_string()).collect(),
        status: if differences.is_empty() {
            CaseStatus::Passed
        } else {
            CaseStatus::Failed
        },
        expected_exit: golden.exit,
        actual_exit: Some(output.exit),
        differences,
    }
}

fn describe_difference(stream: &str, expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    for index in 0..expected_lines.len().max(actual_lines.len()) {
        let expected_line = expected_lines.get(index).copied().unwrap_or("<missing>");
        let actual_line = actual_lines.get(index).copied().unwrap_or("<missing>");
        if expected_line != actual_line {
            return format!(
                "{stream} differs at line {}: expected {expected_line:?}, got {actual_line:?}",
                index + 1
            );
        }
    }
    format!("{stream} differs")
}

fn sandbox_failed_python(sandbox: &Sandbox) -> Option<String> {
    let log = sandbox.root.join("python-invocations.log");
    match fs::read_to_string(&log) {
        Ok(text) if !text.trim().is_empty() => Some(format!(
            "a child process invoked a Python sentinel: {}",
            text.trim()
        )),
        _ => None,
    }
}

/// Runs the selected groups and returns the deterministic report.
fn verify(options: &Options, groups: &[&str]) -> Result<VerifyReport, String> {
    require_executable(&options.rust_bin, "--rust-bin")?;
    let document = load_goldens(&options.goldens)?;
    let cases = selected_cases(groups);
    validate_inventory(&document, &cases, groups)?;
    let goldens: BTreeMap<&str, &GoldenCase> = document
        .cases
        .iter()
        .map(|golden| (golden.id.as_str(), golden))
        .collect();
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut results = Vec::with_capacity(cases.len());
    for group in groups {
        let temp = tempfile::Builder::new()
            .prefix(&format!("apk-lab-goldens-{group}-"))
            .tempdir()
            .map_err(|error| format!("could not create sandbox: {error}"))?;
        let sandbox = prepare_sandbox(group, temp.path(), &project)?;
        for case in cases.iter().filter(|case| case.group == *group) {
            if let Some(reason) = sandbox_failed_python(&sandbox) {
                results.push(CaseResult {
                    id: case.id.to_string(),
                    group: case.group.to_string(),
                    argv: case.argv.iter().map(|value| (*value).to_string()).collect(),
                    status: CaseStatus::Failed,
                    expected_exit: goldens[case.id].exit,
                    actual_exit: None,
                    differences: vec![reason],
                });
                continue;
            }
            let golden = goldens
                .get(case.id)
                .copied()
                .ok_or_else(|| format!("golden case '{}' is missing", case.id))?;
            let output = execute_case(case, &sandbox, &options.rust_bin)?;
            let mut result = compare_case(golden, case, &output, &sandbox);
            if let Some(reason) = sandbox_failed_python(&sandbox) {
                result.status = CaseStatus::Failed;
                result.differences.push(reason);
            }
            results.push(result);
        }
    }
    results.sort_by(|left, right| {
        (left.group.as_str(), left.id.as_str()).cmp(&(right.group.as_str(), right.id.as_str()))
    });
    Ok(build_report(groups, results))
}

/// Regenerates the golden document from observed native output. Maintainers
/// review the diff; verification never calls this path.
fn update_goldens(options: &Options, groups: &[&str]) -> Result<(), String> {
    require_executable(&options.rust_bin, "--rust-bin")?;
    let cases = selected_cases(groups);
    if cases.is_empty() {
        return Err("case inventory is empty".to_string());
    }
    // Preserve unselected groups when regenerating one group.
    let mut merged: BTreeMap<String, GoldenCase> = match load_goldens(&options.goldens) {
        Ok(document) => document
            .cases
            .into_iter()
            .map(|golden| (golden.id.clone(), golden))
            .collect(),
        Err(_) if !options.goldens.exists() => BTreeMap::new(),
        Err(error) => return Err(error),
    };
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for group in groups {
        let temp = tempfile::Builder::new()
            .prefix(&format!("apk-lab-goldens-{group}-"))
            .tempdir()
            .map_err(|error| format!("could not create sandbox: {error}"))?;
        let sandbox = prepare_sandbox(group, temp.path(), &project)?;
        for case in cases.iter().filter(|case| case.group == *group) {
            let output = execute_case(case, &sandbox, &options.rust_bin)?;
            let (stdout, stderr) = sandbox
                .normalizer
                .case(case, &output.stdout, &output.stderr);
            let (_, files) = case_files(case, &sandbox);
            merged.insert(
                case.id.to_string(),
                GoldenCase {
                    id: case.id.to_string(),
                    group: case.group.to_string(),
                    exit: output.exit,
                    stdout,
                    stderr,
                    files,
                },
            );
        }
    }
    let mut goldens: Vec<GoldenCase> = merged.into_values().collect();
    goldens.sort_by(|left, right| {
        (left.group.as_str(), left.id.as_str()).cmp(&(right.group.as_str(), right.id.as_str()))
    });
    let document = GoldenDocument {
        schema_version: GOLDEN_VERSION,
        kind: GOLDEN_KIND.to_string(),
        cases: goldens,
    };
    if let Some(parent) = options.goldens.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut rendered = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("could not serialize goldens: {error}"))?;
    rendered.push('\n');
    fs::write(&options.goldens, rendered).map_err(|error| {
        format!(
            "could not write goldens '{}': {error}",
            options.goldens.display()
        )
    })
}

fn write_report(path: Option<&str>, rendered: &str) -> Result<(), String> {
    match path {
        None | Some("-") => {
            let mut out = std::io::stdout().lock();
            out.write_all(rendered.as_bytes())
                .map_err(|error| format!("could not write report: {error}"))
        }
        Some(path) => fs::write(path, rendered)
            .map_err(|error| format!("could not write report '{path}': {error}")),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_args(args)? else {
        return Ok(());
    };
    let groups = select_groups(&options.group)?;
    if options.update {
        return update_goldens(&options, &groups);
    }
    let report = verify(&options, &groups)?;
    let rendered = render_report(&report)?;
    write_report(options.report.as_deref(), &rendered)?;
    if report.summary.failed > 0 {
        return Err(format!(
            "{} of {} golden cases failed",
            report.summary.failed, report.summary.total
        ));
    }
    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n\
         \x20 apk-lab-migrate verify-goldens --group all|core|analysis|patching-device|acquisition-ci\n\
         \x20                               [--rust-bin PATH] [--report PATH|-]\n\
         \x20                               [--goldens PATH] [--update]\n\
         \n\
         Executes the native apk-lab binary against tiny synthetic fixtures and\n\
         compares normalized exit status, stdout, stderr and declared file digests\n\
         with the committed golden document. No Python module or interpreter is\n\
         imported; Python sentinels are first on PATH and any invocation fails the\n\
         run. The acquisition-ci group covers only deterministic offline flows.\n\
         \n\
         Groups:\n\
         \x20 core               help/version, inspect (JSON/human/error)\n\
         \x20 analysis           strings/res/manifest, asm/unity/il2cpp/smali\n\
         \x20 patching-device    check/deploy/monitor/UI with fake offline tools\n\
         \x20 acquisition-ci     CI matrix and offline acquisition/CI error paths\n\
         \x20 all                every group above\n\
         \n\
         Options:\n\
         \x20 --group NAME       Group to verify (default: all)\n\
         \x20 --rust-bin PATH    Native apk-lab binary (default: target/release/apk-lab)\n\
         \x20 --report PATH|-    Write the deterministic JSON report (default stdout)\n\
         \x20 --goldens PATH     Golden document (default: migration/goldens/native-goldens.json)\n\
         \x20 --update           Regenerate goldens from observed output (maintainers)\n\
         \x20 -h, --help         Print help\n\
         \n\
         Verification fails when the binary is missing, a selected group is empty,\n\
         the golden document is missing/empty, or any normalized value differs."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_document() -> GoldenDocument {
        GoldenDocument {
            schema_version: GOLDEN_VERSION,
            kind: GOLDEN_KIND.to_string(),
            cases: CASES
                .iter()
                .map(|case| GoldenCase {
                    id: case.id.to_string(),
                    group: case.group.to_string(),
                    exit: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    files: BTreeMap::new(),
                })
                .collect(),
        }
    }

    fn case(id: &str) -> &'static Case {
        CASES
            .iter()
            .find(|case| case.id == id)
            .expect("case exists")
    }

    #[test]
    fn selects_all_and_single_groups() {
        assert_eq!(select_groups("all").unwrap(), GROUPS.to_vec());
        assert_eq!(select_groups("core").unwrap(), vec!["core"]);
        assert_eq!(
            select_groups("patching-device").unwrap(),
            vec!["patching-device"]
        );
        assert!(select_groups("nope").is_err());
    }

    #[test]
    fn every_group_has_cases() {
        for group in GROUPS {
            assert!(
                CASES.iter().any(|case| case.group == *group),
                "group {group} is empty"
            );
        }
    }

    #[test]
    fn selected_cases_follow_the_group() {
        let cases = selected_cases(&["core"]);
        assert!(!cases.is_empty());
        assert!(cases.iter().all(|case| case.group == "core"));
        let all = selected_cases(GROUPS);
        assert_eq!(all.len(), CASES.len());
    }

    #[test]
    fn empty_inventory_is_rejected() {
        let empty = GoldenDocument {
            schema_version: GOLDEN_VERSION,
            kind: GOLDEN_KIND.to_string(),
            cases: Vec::new(),
        };
        let cases = selected_cases(&["core"]);
        let error = validate_inventory(&empty, &cases, &["core"]).unwrap_err();
        assert!(error.contains("no cases"), "{error}");

        // A group with no case table entries is rejected even if other groups
        // are populated.
        let error =
            validate_inventory(&test_document(), &cases, &["analysis", "core"]).unwrap_err();
        assert!(error.contains("analysis") || error.contains("not in the selected"));
    }

    #[test]
    fn missing_group_in_document_is_rejected() {
        let mut document = test_document();
        document
            .cases
            .retain(|golden| golden.group != "patching-device");
        let cases = selected_cases(GROUPS);
        let error = validate_inventory(&document, &cases, &["patching-device"]).unwrap_err();
        assert!(error.contains("missing case"), "{error}");
    }

    #[test]
    fn duplicate_and_unknown_cases_are_rejected() {
        let mut duplicate = test_document();
        duplicate.cases.push(duplicate.cases[0].clone());
        let cases = selected_cases(GROUPS);
        assert!(validate_inventory(&duplicate, &cases, GROUPS)
            .unwrap_err()
            .contains("duplicated"));

        let mut unknown = test_document();
        unknown.cases[0].id = "not-a-case".to_string();
        assert!(validate_inventory(&unknown, &cases, GROUPS)
            .unwrap_err()
            .contains("not in the case table"));
    }

    #[test]
    fn mutated_golden_values_are_rejected() {
        let sandbox_root = Path::new("/tmp/goldens-sandbox");
        let run_root = sandbox_root.join("run");
        let sdk = sandbox_root.join("fake-sdk");
        let fake_bin = sandbox_root.join("fake-bin");
        let normalizer = Normalizer::new(sandbox_root, &run_root, &sdk, &fake_bin);
        let sandbox = Sandbox {
            root: sandbox_root.to_path_buf(),
            run_root: run_root.clone(),
            normalizer,
            base_env: BTreeMap::new(),
        };
        let target = case("analysis-asm-encode");
        let output = ProcessOutput {
            exit: 0,
            stdout: "bl 0x3c98ce4 => 0x9400f309\n".to_string(),
            stderr: String::new(),
        };
        let mut golden = GoldenCase {
            id: target.id.to_string(),
            group: target.group.to_string(),
            exit: 0,
            stdout: output.stdout.clone(),
            stderr: String::new(),
            files: BTreeMap::new(),
        };
        let mut result = compare_case(&golden, target, &output, &sandbox);
        assert_eq!(result.status, CaseStatus::Passed);

        golden.stdout = "bl 0x3c98ce4 => 0x9400f000\n".to_string();
        result = compare_case(&golden, target, &output, &sandbox);
        assert_eq!(result.status, CaseStatus::Failed);
        assert!(result.differences[0].contains("stdout"));

        golden.stdout = output.stdout.clone();
        golden.exit = 1;
        result = compare_case(&golden, target, &output, &sandbox);
        assert_eq!(result.status, CaseStatus::Failed);
    }

    #[test]
    fn json_volatile_pointer_is_normalized_without_hiding_other_fields() {
        let text = r#"{"filePath":"/tmp/x.apk","sha256":"abc","sizeBytes":7}"#;
        let normalized = normalize_json(text, &["/filePath"]);
        assert!(normalized.contains("\"filePath\":\"<volatile>\""));
        assert!(normalized.contains("\"sha256\":\"abc\""));
        let untouched = normalize_json(text, &[]);
        assert!(untouched.contains("/tmp/x.apk"));
    }

    #[test]
    fn duration_normalization_only_touches_decimal_seconds() {
        assert_eq!(
            normalize_durations("Patching (1.25s) done\n"),
            "Patching (<duration>) done\n"
        );
        assert_eq!(normalize_durations("v1.2.3\n"), "v1.2.3\n");
    }

    #[test]
    fn report_is_deterministic() {
        let cases = selected_cases(&["core"]);
        let results: Vec<CaseResult> = cases
            .iter()
            .map(|case| CaseResult {
                id: case.id.to_string(),
                group: case.group.to_string(),
                argv: case.argv.iter().map(|value| (*value).to_string()).collect(),
                status: CaseStatus::Passed,
                expected_exit: 0,
                actual_exit: Some(0),
                differences: Vec::new(),
            })
            .collect();
        let report = build_report(&["core"], results.clone());
        let first = render_report(&report).unwrap();
        let second = render_report(&build_report(&["core"], results)).unwrap();
        assert_eq!(first, second);
        assert!(first.contains("\"kind\": \"apk-lab-golden-verification\""));
        assert!(first.contains("\"passed\": 6"));
    }

    #[test]
    fn missing_rust_binary_is_rejected() {
        let error = require_executable(
            Path::new("/definitely/absent/apk-lab-goldens"),
            "--rust-bin",
        )
        .expect_err("missing binary must fail");
        assert!(error.contains("--rust-bin"), "{error}");
    }

    #[test]
    fn golden_document_round_trips() {
        let document = test_document();
        let text = serde_json::to_string(&document).unwrap();
        let parsed: GoldenDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.cases.len(), CASES.len());
    }
}
