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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const SCHEMA_VERSION: u32 = 1;
const REPORT_KIND: &str = "apk-lab-core-parity";
const ANALYSIS_REPORT_KIND: &str = "apk-lab-analysis-parity";
const CORE_GROUP: &str = "core";
const ANALYSIS_GROUP: &str = "analysis";
const SYNTHETIC_PROFILE: &str = "synthetic";
const REAL_TOOLS_PROFILE: &str = "real-tools";
const CORPUS_KIND: &str = "apk-lab-analysis-corpus";
const CORPUS_VERSION: u32 = 1;

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

fn approved_reason_in(
    table: &'static [(&'static str, &'static str)],
    case_id: &str,
) -> Option<&'static str> {
    table
        .iter()
        .find(|(id, _)| *id == case_id)
        .map(|(_, reason)| *reason)
}

/// Maintainer-approved analysis differences (approved by the parent agent on
/// 2026-11-22). This is deliberately a one-entry table: every other analysis
/// difference is a failure.
const ANALYSIS_APPROVED_CASES: &[(&str, &str)] = &[(
    "res-no-json",
    "both reject --json with exit 2; Python argparse prints global usage, native prints RES_HELP",
)];

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
// Analysis case table
// ---------------------------------------------------------------------------

/// Pinned external tools a real-tools analysis case executes. The synthetic
/// profile installs a deterministic fake for each of these at the exact pinned
/// version and digest from the project lock file, so both the Python oracle and
/// the native binary resolve the same identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prerequisite {
    Aapt2,
    Dexdump,
    Baksmali,
    Smali,
    Jadx,
    Apktool,
}

impl Prerequisite {
    fn name(self) -> &'static str {
        match self {
            Prerequisite::Aapt2 => "aapt2",
            Prerequisite::Dexdump => "dexdump",
            Prerequisite::Baksmali => "baksmali",
            Prerequisite::Smali => "smali",
            Prerequisite::Jadx => "jadx",
            Prerequisite::Apktool => "apktool",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AnalysisCase {
    spec: CaseSpec,
    requires: &'static [Prerequisite],
}

const INSPECT_ANALYSIS_REQUIRES: &[Prerequisite] = &[
    Prerequisite::Aapt2,
    Prerequisite::Dexdump,
    Prerequisite::Baksmali,
];

const ANALYSIS_CASES: &[AnalysisCase] = &[
    AnalysisCase {
        spec: CaseSpec {
            id: "asm-encode",
            argv: &["asm", "bl 0x3c98ce4", "--pc", "0x1fcf6e8"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "asm-range-error",
            argv: &["asm", "b 0x8000000", "--pc", "0"],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "strings-unicode-json",
            argv: &["strings", "fixtures/analysis.apk", "-q", "Grüße", "--json"],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "strings-malformed-dex",
            argv: &["strings", "fixtures/malformed.apk", "--json"],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "unity-offset-json",
            argv: &[
                "unity",
                "fixtures/analysis.apk",
                "--gameobject",
                "Button_HelpCenter",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "unity-unsupported",
            argv: &[
                "unity",
                "fixtures/unity-unsupported.apk",
                "--gameobject",
                "Button_HelpCenter",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "il2cpp-offset-json",
            argv: &[
                "il2cpp",
                "fixtures/analysis.apk",
                "--query",
                "OpenShop",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "il2cpp-unsupported-layout",
            argv: &["il2cpp", "fixtures/il2cpp-unsupported.apk", "--json"],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "smali-static-valid",
            argv: &[
                "validate-smali",
                "--snippet",
                "return-void",
                "--locals",
                "0",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Smali],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "smali-static-invalid",
            argv: &[
                "validate-smali",
                "--snippet",
                "const/4 v2, 1",
                "--locals",
                "1",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Smali],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "manifest-json",
            argv: &["manifest", "fixtures/analysis.apk", "--json"],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "res-scalar",
            argv: &[
                "res",
                "fixtures/analysis.apk",
                "--query",
                "@string/app_name",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "res-duplicate",
            argv: &[
                "res",
                "fixtures/analysis.apk",
                "--query",
                "@string/duplicate_value",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "res-alias",
            argv: &[
                "res",
                "fixtures/analysis.apk",
                "--query",
                "@string/app_alias",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "res-human-output",
            argv: &[
                "res",
                "fixtures/analysis.apk",
                "--query",
                "res/values/strings.xml",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "res-no-json",
            argv: &[
                "res",
                "fixtures/analysis.apk",
                "--query",
                "@string/app_name",
                "--json",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "inspect-code-localized",
            argv: &[
                "inspect-code",
                "fixtures/analysis.apk",
                "--class",
                "com.example.Foo",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: INSPECT_ANALYSIS_REQUIRES,
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "inspect-code-ambiguous",
            argv: &[
                "inspect-code",
                "fixtures/ambiguous.apk",
                "--class",
                "com.example.Foo",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: INSPECT_ANALYSIS_REQUIRES,
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "compare-summary-json",
            argv: &[
                "compare",
                "fixtures/analysis-old.apk",
                "fixtures/analysis-new.apk",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "compare-diff-truncation",
            argv: &[
                "compare",
                "fixtures/analysis-old.apk",
                "fixtures/analysis-new.apk",
                "--class",
                "com.example.Foo",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2, Prerequisite::Jadx],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "analyze-workspace-native",
            argv: &[
                "--workspace-root",
                "fixtures/workspace",
                "analyze",
                "fixtures/analysis.apk",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2, Prerequisite::Baksmali],
    },
    AnalysisCase {
        spec: CaseSpec {
            id: "analyze-apktool",
            argv: &[
                "--workspace-root",
                "fixtures/workspace-apktool",
                "analyze",
                "fixtures/analysis.apk",
                "--apktool",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        requires: &[Prerequisite::Aapt2, Prerequisite::Apktool],
    },
];

fn analysis_case(id: &str) -> Option<&'static AnalysisCase> {
    ANALYSIS_CASES.iter().find(|case| case.spec.id == id)
}

// ---------------------------------------------------------------------------
// Options and entry point
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Options {
    group: String,
    profile: String,
    python_project: Option<PathBuf>,
    rust_bin: Option<PathBuf>,
    cases: Vec<String>,
    corpus: Option<PathBuf>,
    report: Option<String>,
    list: bool,
}

/// Runs `parity` with the arguments that follow the subcommand.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_args(args)? else {
        print_help();
        return Ok(());
    };

    match options.group.as_str() {
        CORE_GROUP => run_core(options),
        ANALYSIS_GROUP => run_analysis(options),
        other => Err(format!(
            "unsupported parity group '{other}' (available: '{CORE_GROUP}', '{ANALYSIS_GROUP}')"
        )),
    }
}

fn run_core(options: Options) -> Result<(), String> {
    if options.profile != SYNTHETIC_PROFILE {
        return Err(format!(
            "--profile {REAL_TOOLS_PROFILE} is only supported with --group {ANALYSIS_GROUP}"
        ));
    }
    if options.corpus.is_some() {
        return Err(format!(
            "--corpus is only supported with --group {ANALYSIS_GROUP} --profile {REAL_TOOLS_PROFILE}"
        ));
    }

    let selected = select_core_cases(&options.cases)?;
    if options.list {
        print_core_list(&selected);
        return Ok(());
    }

    let (oracle, rust_bin) = resolve_tools(&options)?;
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-core-parity-")
        .tempdir()
        .map_err(|error| format!("could not create parity sandbox: {error}"))?;
    let report = execute_core(&selected, &oracle, &rust_bin, sandbox.path())?;

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

fn run_analysis(options: Options) -> Result<(), String> {
    let requested = &options.cases;
    let specs = select_analysis_specs(requested)?;
    if options.list {
        print_analysis_list(&specs);
        return Ok(());
    }

    match options.profile.as_str() {
        SYNTHETIC_PROFILE => run_analysis_synthetic(options, &specs),
        REAL_TOOLS_PROFILE => run_analysis_real_tools(options, &specs),
        other => Err(format!(
            "unsupported parity profile '{other}' (available: '{SYNTHETIC_PROFILE}', '{REAL_TOOLS_PROFILE}')"
        )),
    }
}

fn run_analysis_synthetic(options: Options, specs: &[&'static CaseSpec]) -> Result<(), String> {
    if options.corpus.is_some() {
        return Err(format!(
            "--corpus requires --profile {REAL_TOOLS_PROFILE}; the {SYNTHETIC_PROFILE} profile generates its own fixtures"
        ));
    }
    let (oracle, rust_bin) = resolve_tools(&options)?;
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-analysis-parity-")
        .tempdir()
        .map_err(|error| format!("could not create parity sandbox: {error}"))?;
    let prepared = execute_analysis_synthetic(specs, &oracle, &rust_bin, sandbox.path())?;

    finish_analysis(options.report.as_deref(), prepared, SYNTHETIC_PROFILE)
}

fn run_analysis_real_tools(options: Options, specs: &[&'static CaseSpec]) -> Result<(), String> {
    let corpus_path = options
        .corpus
        .as_deref()
        .ok_or("--corpus is required with --profile real-tools")?;
    // Validate the corpus before resolving executables or touching a report.
    let corpus = load_corpus(corpus_path)?;

    let (oracle, rust_bin) = resolve_tools(&options)?;
    let availability = real_tool_availability();
    let cwd = corpus
        .root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| corpus.root.clone());
    let environment = Environment::for_real_host();
    let normalizer = Normalizer::new(NormalizationContext::new(Path::new("/nonexistent"), &cwd));
    let prepared = prepare_real_cases(specs, &corpus, &availability);
    let report = execute_prepared(
        prepared,
        &oracle,
        &rust_bin,
        &cwd,
        &environment,
        &normalizer,
        ANALYSIS_REPORT_KIND,
        Some(REAL_TOOLS_PROFILE),
        ANALYSIS_APPROVED_CASES,
    )?;

    finish_analysis(options.report.as_deref(), report, REAL_TOOLS_PROFILE)
}

fn finish_analysis(
    report_path: Option<&str>,
    report: ParityReport,
    profile: &'static str,
) -> Result<(), String> {
    let rendered = render_report(&report)?;
    write_report(report_path, &rendered)?;
    if !report_passed(&report) {
        return Err(format!(
            "analysis parity ({profile}) failed: {} of {} case(s) did not pass",
            report.summary.failed + report.summary.missing,
            report.summary.total
        ));
    }
    Ok(())
}

fn resolve_tools(options: &Options) -> Result<(Oracle, PathBuf), String> {
    let python_project = options
        .python_project
        .as_ref()
        .ok_or("--python-project is required unless --list is used")?;
    let rust_bin = options
        .rust_bin
        .as_ref()
        .ok_or("--rust-bin is required unless --list is used")?;
    let python_project = require_directory(python_project, "--python-project")?;
    let rust_bin = require_executable(rust_bin, "--rust-bin")?;
    let oracle = resolve_oracle(&python_project)?;
    Ok((oracle, rust_bin))
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut group: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut python_project: Option<PathBuf> = None;
    let mut rust_bin: Option<PathBuf> = None;
    let mut cases = Vec::new();
    let mut corpus: Option<PathBuf> = None;
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
            "--profile" => {
                profile = Some(require_value(args, &mut index, "--profile")?);
            }
            _ if arg.starts_with("--profile=") => {
                profile = Some(non_empty(&arg["--profile=".len()..], "--profile")?);
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
            "--corpus" => {
                corpus = Some(PathBuf::from(require_value(args, &mut index, "--corpus")?));
            }
            _ if arg.starts_with("--corpus=") => {
                corpus = Some(PathBuf::from(non_empty(
                    &arg["--corpus=".len()..],
                    "--corpus",
                )?));
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

    let group = group.ok_or(format!(
        "--group is required (available: '{CORE_GROUP}', '{ANALYSIS_GROUP}')"
    ))?;
    let profile = profile.unwrap_or_else(|| SYNTHETIC_PROFILE.to_string());
    match profile.as_str() {
        SYNTHETIC_PROFILE | REAL_TOOLS_PROFILE => {}
        other => {
            return Err(format!(
                "unsupported parity profile '{other}' (available: '{SYNTHETIC_PROFILE}', '{REAL_TOOLS_PROFILE}')"
            ))
        }
    }
    Ok(Some(Options {
        group,
        profile,
        python_project,
        rust_bin,
        cases,
        corpus,
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

fn select_core_cases(requested: &[String]) -> Result<Vec<&'static CaseSpec>, String> {
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

fn select_analysis_specs(requested: &[String]) -> Result<Vec<&'static CaseSpec>, String> {
    let mut wanted = BTreeSet::new();
    for id in requested {
        if analysis_case(id).is_none() {
            return Err(format!(
                "unknown analysis parity case '{id}' (run 'parity --group analysis --list')"
            ));
        }
        wanted.insert(id.as_str());
    }
    Ok(ANALYSIS_CASES
        .iter()
        .map(|case| &case.spec)
        .filter(|spec| wanted.is_empty() || wanted.contains(spec.id))
        .collect())
}

fn print_core_list(cases: &[&CaseSpec]) {
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

fn print_analysis_list(cases: &[&CaseSpec]) {
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
        Self::for_sandbox_with_tools(sandbox, project, fake_sdk, None)
    }

    fn for_sandbox_with_tools(
        sandbox: &Path,
        project: &Path,
        fake_sdk: &Path,
        fake_bin: Option<&Path>,
    ) -> Self {
        let home = sandbox.join("home");
        let temp = sandbox.join("tmp");
        let mut path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
        if let Some(fake_bin) = fake_bin {
            path = format!("{}:{path}", fake_bin.to_string_lossy());
        }
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

    /// Real-tools parity must see the host's actual pinned tool cache, SDK and
    /// Java installation, so it preserves the host inputs instead of pointing
    /// HOME/ANDROID_HOME at the sandbox.
    fn for_real_host() -> Self {
        let variables = std::env::vars()
            .filter(|(name, _)| {
                matches!(
                    name.as_str(),
                    "PATH"
                        | "HOME"
                        | "USERPROFILE"
                        | "HOMEDRIVE"
                        | "HOMEPATH"
                        | "ANDROID_HOME"
                        | "ANDROID_SDK_ROOT"
                        | "JAVA_HOME"
                        | "APK_LAB_TOOLS_LOCK"
                        | "APK_LAB_REPO_ROOT"
                )
            })
            .chain(
                [
                    ("TZ", "UTC"),
                    ("PYTHONIOENCODING", "utf-8"),
                    ("PYTHONDONTWRITEBYTECODE", "1"),
                    ("PYTHONHASHSEED", "0"),
                    ("PYTHONUTF8", "1"),
                ]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string())),
            )
            .collect();
        Self { variables }
    }
}

/// One selected case with its concrete argv. `spec` supplies the comparison
/// contract; `argv` may come from the committed table (synthetic) or from a
/// validated corpus (real tools). `missing` short-circuits execution and
/// produces a `missing` result with an exact reason.
#[derive(Debug)]
struct PreparedCase {
    id: String,
    argv: Vec<String>,
    spec: CaseSpec,
    missing: Option<String>,
}

impl PreparedCase {
    fn new(spec: &CaseSpec, argv: Vec<String>) -> Self {
        Self {
            id: spec.id.to_string(),
            argv,
            spec: *spec,
            missing: None,
        }
    }

    fn missing(spec: &CaseSpec, argv: Vec<String>, reason: String) -> Self {
        Self {
            id: spec.id.to_string(),
            argv,
            spec: *spec,
            missing: Some(reason),
        }
    }
}

fn execute_core(
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
    let prepared: Vec<PreparedCase> = cases
        .iter()
        .map(|case| {
            PreparedCase::new(
                case,
                case.argv.iter().map(|value| value.to_string()).collect(),
            )
        })
        .collect();
    execute_prepared(
        prepared,
        oracle,
        rust_bin,
        &run_root,
        &environment,
        &normalizer,
        REPORT_KIND,
        None,
        APPROVED_CASES,
    )
}

fn execute_analysis_synthetic(
    cases: &[&CaseSpec],
    oracle: &Oracle,
    rust_bin: &Path,
    sandbox: &Path,
) -> Result<ParityReport, String> {
    let run_root = sandbox.join("run");
    let fake_sdk = sandbox.join("fake-sdk");
    let fake_bin = sandbox.join("fake-bin");
    let home = sandbox.join("home");
    create_analysis_fixtures(
        &run_root,
        &fake_sdk,
        &home,
        &fake_bin,
        python_project_path(oracle),
    )?;
    for directory in [&home, &sandbox.join("tmp"), &fake_bin] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    let environment = Environment::for_sandbox_with_tools(
        sandbox,
        python_project_path(oracle),
        &fake_sdk,
        Some(&fake_bin),
    );
    let normalizer = Normalizer::new(NormalizationContext::new(sandbox, &run_root));
    let prepared: Vec<PreparedCase> = cases
        .iter()
        .map(|case| {
            PreparedCase::new(
                case,
                case.argv.iter().map(|value| value.to_string()).collect(),
            )
        })
        .collect();
    execute_prepared(
        prepared,
        oracle,
        rust_bin,
        &run_root,
        &environment,
        &normalizer,
        ANALYSIS_REPORT_KIND,
        Some(SYNTHETIC_PROFILE),
        ANALYSIS_APPROVED_CASES,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_prepared(
    cases: Vec<PreparedCase>,
    oracle: &Oracle,
    rust_bin: &Path,
    cwd: &Path,
    environment: &Environment,
    normalizer: &Normalizer,
    kind: &'static str,
    profile: Option<&'static str>,
    approvals: &'static [(&'static str, &'static str)],
) -> Result<ParityReport, String> {
    let mut results = Vec::with_capacity(cases.len());
    for case in &cases {
        if let Some(reason) = &case.missing {
            results.push(missing_result_with_argv(case, reason.clone()));
            continue;
        }
        let python = match run_process(oracle_command(oracle, &case.argv), cwd, environment) {
            Ok(result) => result,
            Err(error) => {
                results.push(missing_result_with_argv(case, format!("python: {error}")));
                continue;
            }
        };
        let rust = match run_binary(rust_bin, &case.argv, cwd, environment) {
            Ok(result) => result,
            Err(error) => {
                results.push(missing_result_with_exit_and_argv(
                    case,
                    &python,
                    format!("rust: {error}"),
                ));
                continue;
            }
        };
        let differences = compare_case(&case.spec, &python, &rust, &normalizer.at(&case.spec));
        let approval = approved_reason_in(approvals, &case.id);
        let approved = !differences.is_empty() && approval.is_some();
        let status = if differences.is_empty() || approved {
            CaseStatus::Passed
        } else {
            CaseStatus::Failed
        };
        results.push(CaseResult {
            id: case.id.clone(),
            argv: case.argv.clone(),
            status,
            approved,
            approval: approval.map(str::to_string),
            exit: ExitPair {
                python: python.exit,
                rust: rust.exit,
            },
            differences,
        });
    }

    Ok(build_report_for(results, kind, profile))
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

fn oracle_command(oracle: &Oracle, argv: &[String]) -> (PathBuf, Vec<String>) {
    match oracle {
        Oracle::Direct(entrypoint) => (entrypoint.clone(), argv.to_vec()),
        Oracle::Uv { uv, project } => {
            let mut args = vec![
                "run".to_string(),
                "--project".to_string(),
                project.to_string_lossy().into_owned(),
                "apk-lab".to_string(),
            ];
            args.extend(argv.iter().cloned());
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

fn missing_result_with_argv(case: &PreparedCase, difference: String) -> CaseResult {
    missing_result_with_exit_and_argv(case, &ProcessResult::missing(), difference)
}

fn missing_result_with_exit_and_argv(
    case: &PreparedCase,
    python: &ProcessResult,
    difference: String,
) -> CaseResult {
    CaseResult {
        id: case.id.clone(),
        argv: case.argv.clone(),
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
// Real-tools corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusDocument {
    kind: String,
    version: u32,
    #[serde(default)]
    artifacts: BTreeMap<String, String>,
    cases: Vec<CorpusCaseDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusCaseDocument {
    id: String,
    argv: Vec<String>,
}

#[derive(Debug, Clone)]
struct Corpus {
    /// Canonical path of the corpus document itself.
    root: PathBuf,
    cases: BTreeMap<String, Vec<String>>,
}

fn load_corpus(path: &Path) -> Result<Corpus, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read corpus '{}': {error}", path.display()))?;
    let document: CorpusDocument = serde_json::from_str(&text)
        .map_err(|error| format!("invalid corpus '{}': {error}", path.display()))?;
    if document.kind != CORPUS_KIND {
        return Err(format!(
            "invalid corpus '{}': kind must be '{CORPUS_KIND}'",
            path.display()
        ));
    }
    if document.version != CORPUS_VERSION {
        return Err(format!(
            "invalid corpus '{}': unsupported version {} (expected {CORPUS_VERSION})",
            path.display(),
            document.version
        ));
    }
    let root = fs::canonicalize(path)
        .map_err(|error| format!("could not resolve corpus '{}': {error}", path.display()))?;
    let base = root.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut artifacts = BTreeMap::new();
    for (name, relative) in document.artifacts {
        if name.is_empty() {
            return Err(format!(
                "invalid corpus '{}': artifact names must be non-empty",
                path.display()
            ));
        }
        if relative.is_empty() {
            return Err(format!(
                "invalid corpus '{}': artifact '{name}' has an empty path",
                path.display()
            ));
        }
        let candidate = if Path::new(&relative).is_absolute() {
            PathBuf::from(&relative)
        } else {
            base.join(&relative)
        };
        let resolved = fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "invalid corpus '{}': artifact '{name}' cannot be resolved ('{relative}'): {error}",
                path.display()
            )
        })?;
        if !resolved.is_file() {
            return Err(format!(
                "invalid corpus '{}': artifact '{name}' is not a file: {}",
                path.display(),
                resolved.display()
            ));
        }
        artifacts.insert(name, resolved);
    }

    if document.cases.is_empty() {
        return Err(format!(
            "invalid corpus '{}': cases must contain at least one entry",
            path.display()
        ));
    }
    let mut cases = BTreeMap::new();
    for case in document.cases {
        if analysis_case(&case.id).is_none() {
            return Err(format!(
                "invalid corpus '{}': unknown analysis case '{}' (run 'parity --group analysis --list')",
                path.display(),
                case.id
            ));
        }
        if case.argv.is_empty() {
            return Err(format!(
                "invalid corpus '{}': case '{}' has an empty argv",
                path.display(),
                case.id
            ));
        }
        let mut argv = Vec::with_capacity(case.argv.len());
        for token in case.argv {
            if let Some(name) = token.strip_prefix('$') {
                let Some(artifact) = artifacts.get(name) else {
                    return Err(format!(
                        "invalid corpus '{}': case '{}' references unknown artifact '${name}'",
                        path.display(),
                        case.id
                    ));
                };
                argv.push(artifact.to_string_lossy().into_owned());
            } else {
                argv.push(token);
            }
        }
        if cases.insert(case.id.clone(), argv).is_some() {
            return Err(format!(
                "invalid corpus '{}': duplicate case '{}'",
                path.display(),
                case.id
            ));
        }
    }

    Ok(Corpus { root, cases })
}

#[derive(Debug, Clone, Copy, Default)]
struct RealToolAvailability {
    aapt2: bool,
    dexdump: bool,
    baksmali: bool,
    smali: bool,
    jadx: bool,
    apktool: bool,
}

impl RealToolAvailability {
    fn has(self, prerequisite: Prerequisite) -> bool {
        match prerequisite {
            Prerequisite::Aapt2 => self.aapt2,
            Prerequisite::Dexdump => self.dexdump,
            Prerequisite::Baksmali => self.baksmali,
            Prerequisite::Smali => self.smali,
            Prerequisite::Jadx => self.jadx,
            Prerequisite::Apktool => self.apktool,
        }
    }
}

fn real_tool_availability() -> RealToolAvailability {
    let repo_root = crate::tools::default_repo_root();
    let env = crate::tools::ToolEnvironment::from_process(&repo_root);
    let manager = crate::tools::ToolManager::with_defaults();
    let installed = |name: &str| {
        manager
            .as_ref()
            .is_ok_and(|manager| manager.is_tool_installed(name))
    };
    RealToolAvailability {
        aapt2: crate::tools::find_build_tools_bin(&env, "aapt2").is_ok(),
        dexdump: crate::tools::find_build_tools_bin(&env, "dexdump").is_ok(),
        baksmali: installed("baksmali"),
        smali: installed("smali"),
        jadx: installed("jadx"),
        apktool: installed("apktool"),
    }
}

fn missing_prerequisites(
    required: &[Prerequisite],
    availability: RealToolAvailability,
) -> Option<String> {
    let missing: Vec<&str> = required
        .iter()
        .filter(|prerequisite| !availability.has(**prerequisite))
        .map(|prerequisite| prerequisite.name())
        .collect();
    (!missing.is_empty()).then(|| format!("missing pinned tool(s): {}", missing.join(", ")))
}

fn prepare_real_cases(
    specs: &[&'static CaseSpec],
    corpus: &Corpus,
    availability: &RealToolAvailability,
) -> Vec<PreparedCase> {
    specs
        .iter()
        .map(|spec| {
            let Some(argv) = corpus.cases.get(spec.id) else {
                return PreparedCase::missing(
                    spec,
                    spec.argv.iter().map(|value| value.to_string()).collect(),
                    format!("corpus has no case '{}'", spec.id),
                );
            };
            let requires = analysis_case(spec.id)
                .map(|case| case.requires)
                .unwrap_or(&[]);
            if let Some(reason) = missing_prerequisites(requires, *availability) {
                return PreparedCase::missing(spec, argv.clone(), reason);
            }
            PreparedCase::new(spec, argv.clone())
        })
        .collect()
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
// Analysis fixtures and fake pinned tools
// ---------------------------------------------------------------------------

const ANALYSIS_SDK_VERSION: &str = "build-tools/36.0.0";
const ANALYSIS_CACHE: &str = ".cache/aidans-patches/apk-lab/tools";

const ANALYSIS_BADGING: &str = "\
package: name='com.example.analysis' versionCode='100' versionName='1.0' compileSdkVersion='34'
sdkVersion:'21'
targetSdkVersion:'34'
uses-permission: name='android.permission.INTERNET'
";

const ANALYSIS_XMLTREE: &str = r#"N: android=http://schemas.android.com/apk/res/android (line=2)
  E: manifest (line=2)
    E: application (line=10)
      E: activity (line=12)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.MainActivity" (Raw: "com.example.MainActivity")
        A: http://schemas.android.com/apk/res/android:exported(0x01010010)=true
        E: intent-filter (line=15)
          E: action (line=16)
            A: http://schemas.android.com/apk/res/android:name(0x01010003)="android.intent.action.MAIN" (Raw: "android.intent.action.MAIN")
      E: service (line=20)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.example.SyncService"
        A: http://schemas.android.com/apk/res/android:permission(0x01010006)="com.example.PERMISSION"
"#;

const ANALYSIS_RESOURCES: &str = r#"Package name=com.example.analysis id=7f
  type string id=01 entryCount=3
    resource 0x7f010001 string/app_name
      () "Parity App"
    resource 0x7f010002 string/app_alias
      () "@string/app_name"
    resource 0x7f010003 string/duplicate_value
      () "same"
    resource 0x7f010003 string/duplicate_value
      () "same"
  type drawable id=02 entryCount=1
    resource 0x7f020001 drawable/icon
      () (file) res/drawable/icon.png
"#;

const ANALYSIS_AAPT2: &str = r#"#!/bin/sh
set -eu
dir="$(dirname "$0")"
case "${1:-} ${2:-}" in
  "dump badging") exec cat "$dir/badging.txt" ;;
  "dump xmltree") exec cat "$dir/xmltree.txt" ;;
  "dump resources") exec cat "$dir/resources.txt" ;;
  *)
    echo "fake aapt2 does not support: $*" >&2
    exit 64
    ;;
esac
"#;

const FAKE_JAVA: &str = r#"#!/bin/sh
if [ "${1:-}" != "-jar" ]; then
  echo "fake java only supports -jar" >&2
  exit 64
fi
shift
jar="$1"
shift
exec "$jar" "$@"
"#;

const FAKE_JADX: &str = r#"#!/bin/sh
set -eu
out=""
single_out=""
artifact=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -d) out="$2"; shift 2 ;;
    --single-class-output) single_out="$2"; shift 2 ;;
    --single-class|--include-pkg|--output-format) shift 2 ;;
    --no-res|--show-bad-code) shift ;;
    *) artifact="$1"; shift ;;
  esac
done
if [ -n "$single_out" ]; then
  file="$single_out"
  mkdir -p "$(dirname "$file")"
else
  file="$out/sources/com/example/Foo.java"
  mkdir -p "$(dirname "$file")"
fi
if grep -aq "PARITY_NEW" "$artifact"; then
  base=1000
else
  base=0
fi
{
  echo "package com.example;"
  echo
  echo "public class Foo {"
  echo "    public static int answer() {"
  i=0
  while [ "$i" -lt 300 ]; do
    echo "        int value$i = $((base + i));"
    i=$((i + 1))
  done
  echo "        return value299;"
  echo "    }"
  echo "}"
} > "$file"
"#;

const FAKE_BAKSMALI: &str = r#"#!/bin/sh
set -eu
if [ "${1:-}" != "d" ]; then
  echo "fake baksmali only supports d" >&2
  exit 64
fi
shift
dex="$1"
shift
out=""
descriptor=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --classes) descriptor="$2"; shift 2 ;;
    --code-offsets) shift ;;
    -o) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ -z "$out" ]; then
  echo "fake baksmali requires -o" >&2
  exit 64
fi
if [ -n "$descriptor" ]; then
  rel="$(printf '%s' "$descriptor" | sed 's/^L//; s/;$//')"
  file="$out/$rel.smali"
  mkdir -p "$(dirname "$file")"
  {
    printf '.class public %s\n' "$descriptor"
    printf '.super Ljava/lang/Object;\n\n'
    printf '.method public static answer()I\n'
    printf '    .registers 2\n'
    printf '    const/4 v0, 0x2a\n'
    printf '    return v0\n'
    printf '.end method\n'
  } > "$file"
else
  mkdir -p "$out"
  printf '.class public Lcom/example/All;\n' > "$out/All.smali"
fi
"#;

const FAKE_SMALI: &str = r#"#!/bin/sh
if [ "${1:-}" = "assemble" ]; then
  exit 0
fi
exit 1
"#;

const FAKE_APKTOOL: &str = r#"#!/bin/sh
set -eu
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ -n "$out" ]; then
  mkdir -p "$out"
  printf 'decoded\n' > "$out/apktool.yml"
fi
"#;

const FAKE_MORPHE: &str = "#!/bin/sh\nexit 0\n";

fn create_analysis_fixtures(
    run_root: &Path,
    fake_sdk: &Path,
    home: &Path,
    fake_bin: &Path,
    project: &Path,
) -> Result<(), String> {
    let fixtures = run_root.join("fixtures");
    let build_tools = fake_sdk.join(ANALYSIS_SDK_VERSION);
    for directory in [&fixtures, &build_tools, home, fake_bin] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }

    for (name, content) in [
        ("badging.txt", ANALYSIS_BADGING),
        ("xmltree.txt", ANALYSIS_XMLTREE),
        ("resources.txt", ANALYSIS_RESOURCES),
    ] {
        fs::write(build_tools.join(name), content)
            .map_err(|error| format!("could not write fake {name}: {error}"))?;
    }
    let aapt2 = build_tools.join("aapt2");
    fs::write(&aapt2, ANALYSIS_AAPT2)
        .map_err(|error| format!("could not write fake aapt2: {error}"))?;
    make_executable(&aapt2)?;

    let dexdump = build_tools.join("dexdump");
    fs::write(&dexdump, FAKE_DEXDUMP)
        .map_err(|error| format!("could not write fake dexdump: {error}"))?;
    make_executable(&dexdump)?;

    let apksigner = build_tools.join("apksigner");
    fs::write(&apksigner, "#!/bin/sh\nexit 1\n")
        .map_err(|error| format!("could not write fake apksigner: {error}"))?;
    make_executable(&apksigner)?;

    install_analysis_tools(home, fake_bin, project)?;

    let manifest = b"<manifest package=\"com.example.analysis\"/>" as &[u8];
    let metadata = il2cpp_metadata_v29();
    let gameobject = unity_gameobject("Button_HelpCenter", 1);
    let analysis_dex = dex_with_strings(&["PARITY_CLASS", "Grüße", "Привет"], 1);
    let analysis_entries: [(&str, &[u8]); 9] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", analysis_dex.as_slice()),
        ("resources.arsc", b"fake_arsc" as &[u8]),
        ("res/values/strings.xml", b"<resources/>" as &[u8]),
        ("res/drawable/icon.png", b"icon" as &[u8]),
        ("assets/note.txt", b"parity fixture" as &[u8]),
        ("lib/arm64-v8a/libnative.so", b"native" as &[u8]),
        (
            "assets/bin/Data/Managed/Metadata/global-metadata.dat",
            metadata.as_slice(),
        ),
        (
            "assets/bin/Data/sharedassets0.assets",
            gameobject.as_slice(),
        ),
    ];
    write_apk(&fixtures.join("analysis.apk"), &analysis_entries)?;

    let old_dex = dex_with_strings(&["PARITY_CLASS", "PARITY_OLD"], 1);
    let new_dex = dex_with_strings(&["PARITY_CLASS", "PARITY_NEW"], 1);
    let old_entries: [(&str, &[u8]); 4] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", old_dex.as_slice()),
        ("lib/arm64-v8a/libnative.so", b"native" as &[u8]),
        ("assets/note.txt", b"parity fixture" as &[u8]),
    ];
    let new_entries: [(&str, &[u8]); 4] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", new_dex.as_slice()),
        ("lib/arm64-v8a/libnative.so", b"native" as &[u8]),
        ("assets/note.txt", b"parity fixture" as &[u8]),
    ];
    write_apk(&fixtures.join("analysis-old.apk"), &old_entries)?;
    write_apk(&fixtures.join("analysis-new.apk"), &new_entries)?;

    let ambiguous_dex = dex_with_strings(&["PARITY_CLASS", "Grüße"], 1);
    let ambiguous_entries: [(&str, &[u8]); 3] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", ambiguous_dex.as_slice()),
        ("classes2.dex", ambiguous_dex.as_slice()),
    ];
    write_apk(&fixtures.join("ambiguous.apk"), &ambiguous_entries)?;

    write_apk(
        &fixtures.join("malformed.apk"),
        &[
            ("AndroidManifest.xml", manifest),
            ("classes.dex", b"dex\n035\0" as &[u8]),
        ],
    )?;
    write_apk(
        &fixtures.join("unity-unsupported.apk"),
        &[
            ("AndroidManifest.xml", manifest),
            ("assets/bin/Data/sharedassets9.assets", b"short" as &[u8]),
        ],
    )?;
    let unsupported_metadata = il2cpp_metadata_with_version(99);
    write_apk(
        &fixtures.join("il2cpp-unsupported.apk"),
        &[
            ("AndroidManifest.xml", manifest),
            (
                "assets/bin/Data/Managed/Metadata/global-metadata.dat",
                unsupported_metadata.as_slice(),
            ),
        ],
    )?;
    Ok(())
}

const FAKE_DEXDUMP: &str = r#"#!/bin/sh
if grep -aq "PARITY_CLASS" "$1" || grep -aq "PARITY_OLD" "$1" || grep -aq "PARITY_NEW" "$1"; then
  echo "Class descriptor  : 'Lcom/example/Foo;'"
fi
exit 0
"#;

fn install_analysis_tools(home: &Path, fake_bin: &Path, project: &Path) -> Result<(), String> {
    let lock_path = project.join("tools.lock.json");
    let text = fs::read_to_string(&lock_path).map_err(|error| {
        format!(
            "could not read pinned tool lock '{}': {error}",
            lock_path.display()
        )
    })?;
    let lock: Value = serde_json::from_str(&text).map_err(|error| {
        format!(
            "invalid pinned tool lock '{}': {error}",
            lock_path.display()
        )
    })?;
    let tools = lock
        .get("tools")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("pinned tool lock '{}' has no tools", lock_path.display()))?;

    for (name, script) in [
        ("morphe", FAKE_MORPHE),
        ("jadx", FAKE_JADX),
        ("apktool", FAKE_APKTOOL),
        ("baksmali", FAKE_BAKSMALI),
        ("smali", FAKE_SMALI),
    ] {
        let spec = tools
            .get(name)
            .ok_or_else(|| format!("pinned tool lock is missing '{name}'"))?;
        let version = spec
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("pinned tool '{name}' has no version"))?;
        let sha256 = spec
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("pinned tool '{name}' has no sha256"))?;
        let executable = spec
            .get("executablePath")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("pinned tool '{name}' has no executablePath"))?;
        let path = home
            .join(ANALYSIS_CACHE)
            .join(name)
            .join(version)
            .join(sha256)
            .join(executable);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        fs::write(&path, script)
            .map_err(|error| format!("could not write fake {name}: {error}"))?;
        make_executable(&path)?;
    }

    let java = fake_bin.join("java");
    fs::write(&java, FAKE_JAVA).map_err(|error| format!("could not write fake java: {error}"))?;
    make_executable(&java)
}

fn dex_with_strings(strings: &[&str], class_count: u32) -> Vec<u8> {
    let ids_offset = 0x70usize;
    let data_offset = ids_offset + strings.len() * 4;
    let mut blob = Vec::new();
    let mut offsets = Vec::new();
    for value in strings {
        offsets.push(data_offset + blob.len());
        blob.push(value.len() as u8);
        blob.extend_from_slice(value.as_bytes());
        blob.push(0);
    }
    let mut data = vec![0u8; data_offset];
    data[0..8].copy_from_slice(b"dex\n035\0");
    for (index, offset) in offsets.iter().enumerate() {
        let position = ids_offset + index * 4;
        data[position..position + 4].copy_from_slice(&(*offset as u32).to_le_bytes());
    }
    data.extend_from_slice(&blob);
    data[0x38..0x3C].copy_from_slice(&(strings.len() as u32).to_le_bytes());
    data[0x3C..0x40].copy_from_slice(&(ids_offset as u32).to_le_bytes());
    data[0x58..0x5C].copy_from_slice(&0u32.to_le_bytes());
    data[0x60..0x64].copy_from_slice(&class_count.to_le_bytes());
    let size = data.len() as u32;
    data[0x20..0x24].copy_from_slice(&size.to_le_bytes());
    data
}

fn unity_gameobject(name: &str, is_active: u8) -> Vec<u8> {
    let mut data = vec![0u8; 32];
    let name_bytes = name.as_bytes();
    data.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(name_bytes);
    data.extend(std::iter::repeat_n(0u8, (4 - (name_bytes.len() % 4)) % 4));
    data.extend_from_slice(&0u16.to_le_bytes());
    data.push(is_active);
    data.extend(std::iter::repeat_n(0u8, 16));
    data
}

fn il2cpp_metadata_v29() -> Vec<u8> {
    il2cpp_metadata_with_version(29)
}

fn il2cpp_metadata_with_version(version: u32) -> Vec<u8> {
    let string_data: &[u8] =
        b"\x00TestNamespace\x00BlackjackApplication\x00OpenShop\x00OtherMethod\x00";
    let string_offset = 256usize;
    let string_size = string_data.len();
    let mut methods_data = Vec::new();
    // v29 method record: name_idx, decl_type, return_type, param_start,
    // gen_container, token, flags, iflags, slot (u16), param_count (u16).
    methods_data.extend_from_slice(&36i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&(-1i32).to_le_bytes());
    methods_data.extend_from_slice(&0u32.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&1u16.to_le_bytes());
    methods_data.extend_from_slice(&45i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&0i32.to_le_bytes());
    methods_data.extend_from_slice(&(-1i32).to_le_bytes());
    methods_data.extend_from_slice(&0u32.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    methods_data.extend_from_slice(&0u16.to_le_bytes());
    let methods_offset = string_offset + string_size + 4;
    let types_offset = methods_offset + methods_data.len() + 4;
    let mut type0 = vec![0u8; 88];
    type0[0..4].copy_from_slice(&15i32.to_le_bytes());
    type0[4..8].copy_from_slice(&1i32.to_le_bytes());
    type0[36..40].copy_from_slice(&0i32.to_le_bytes());
    type0[64..66].copy_from_slice(&2u16.to_le_bytes());
    let total = types_offset + type0.len() + 16;
    let mut data = vec![0u8; total];
    data[0..4].copy_from_slice(&0xFAB1_1BAFu32.to_le_bytes());
    data[4..8].copy_from_slice(&version.to_le_bytes());
    data[24..28].copy_from_slice(&(string_offset as u32).to_le_bytes());
    data[28..32].copy_from_slice(&(string_size as u32).to_le_bytes());
    data[48..52].copy_from_slice(&(methods_offset as u32).to_le_bytes());
    data[52..56].copy_from_slice(&(methods_data.len() as u32).to_le_bytes());
    data[160..164].copy_from_slice(&(types_offset as u32).to_le_bytes());
    data[164..168].copy_from_slice(&(type0.len() as u32).to_le_bytes());
    data[string_offset..string_offset + string_size].copy_from_slice(string_data);
    data[methods_offset..methods_offset + methods_data.len()].copy_from_slice(&methods_data);
    data[types_offset..types_offset + type0.len()].copy_from_slice(&type0);
    data
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
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<&'static str>,
    cases: Vec<CaseResult>,
    summary: Summary,
}

#[cfg(test)]
fn build_report(cases: Vec<CaseResult>) -> ParityReport {
    build_report_for(cases, REPORT_KIND, None)
}

fn build_report_for(
    cases: Vec<CaseResult>,
    kind: &'static str,
    profile: Option<&'static str>,
) -> ParityReport {
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
        kind,
        profile,
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
         \x20 apk-lab-migrate parity --group analysis [--profile synthetic] \\\n\
         \x20                     --python-project DIR --rust-bin PATH \\\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group analysis --profile real-tools \\\n\
         \x20                     --corpus FILE --python-project DIR --rust-bin PATH \\\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group analysis --list [--case ID ...]\n\
         \n\
         Runs the independent Python oracle and the native binary with identical\n\
         argv, cwd and environment, then compares exit status and output. JSON\n\
         output is compared structurally after removing only the volatile fields\n\
         declared by the committed case table; human output uses the same\n\
         normalization set.\n\
         \n\
         The analysis group defaults to --profile synthetic: it creates tiny\n\
         deterministic fixtures and installs fake pinned aapt2, dexdump, baksmali,\n\
         smali, JADX and Apktool tool identities from the project lock file. No\n\
         network, device, Python production path or real tool cache is used.\n\
         \n\
         --profile real-tools requires --corpus and the actual pinned tools plus a\n\
         real artifact for every corpus case. A missing tool, artifact or case is\n\
         reported as `missing` and exits nonzero; nothing is skipped green. The\n\
         corpus is a JSON document, validated before any report is written:\n\
         \n\
         \x20 {{\n\
         \x20   \"kind\": \"apk-lab-analysis-corpus\",\n\
         \x20   \"version\": 1,\n\
         \x20   \"artifacts\": {{\"base\": \"relative/or/absolute.apk\"}},\n\
         \x20   \"cases\": [\n\
         \x20     {{\"id\": \"manifest-json\", \"argv\": [\"manifest\", \"$base\", \"--json\"]}}\n\
         \x20   ]\n\
         \x20 }}\n\
         \n\
         Every case id must name a case from `--list`; duplicate or unknown ids,\n\
         unknown $artifact placeholders, missing artifacts, an unknown kind or\n\
         version, and an empty cases list are rejected. Artifact paths are resolved\n\
         relative to the corpus file; `$name` tokens in argv are replaced by the\n\
         canonical artifact path before execution.\n\
         \n\
         Options:\n\
         \x20 --group NAME          Parity group (available: core, analysis)\n\
         \x20 --profile NAME        Analysis profile (synthetic, real-tools; default synthetic)\n\
         \x20 --corpus FILE         Validated real-tools corpus manifest (required for real-tools)\n\
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
        let error = select_core_cases(&["not-a-real-case".to_string()])
            .expect_err("unknown case must be rejected");
        assert!(error.contains("unknown core parity case"), "{error}");
    }

    #[test]
    fn analysis_group_selection_is_explicit_and_stable() {
        let selected = select_analysis_specs(&[]).expect("all analysis cases");
        assert_eq!(selected.len(), ANALYSIS_CASES.len());
        assert_eq!(selected[0].id, "asm-encode");
        assert_eq!(selected.last().map(|spec| spec.id), Some("analyze-apktool"));
        let filtered =
            select_analysis_specs(&["manifest-json".to_string(), "asm-encode".to_string()])
                .expect("filtered");
        assert_eq!(
            filtered.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            vec!["asm-encode", "manifest-json"]
        );
        let error = select_analysis_specs(&["not-an-analysis-case".to_string()])
            .expect_err("unknown analysis case must be rejected");
        assert!(error.contains("unknown analysis parity case"), "{error}");

        let core = select_core_cases(&[]).expect("all core cases");
        assert_eq!(core.len(), CORE_CASES.len());
    }

    #[test]
    fn analysis_case_absent_from_corpus_is_missing() {
        let spec = select_analysis_specs(&["manifest-json".to_string()])
            .expect("spec")
            .remove(0);
        let corpus = Corpus {
            root: PathBuf::from("/tmp/corpus.json"),
            cases: BTreeMap::new(),
        };
        let availability = RealToolAvailability {
            aapt2: true,
            ..RealToolAvailability::default()
        };
        let prepared = prepare_real_cases(&[spec], &corpus, &availability);
        assert_eq!(prepared.len(), 1);
        assert_eq!(
            prepared[0].missing.as_deref(),
            Some("corpus has no case 'manifest-json'")
        );
        let result = missing_result_with_argv(&prepared[0], prepared[0].missing.clone().unwrap());
        assert_eq!(result.status, CaseStatus::Missing);
        let report = build_report_for(vec![result], ANALYSIS_REPORT_KIND, Some(REAL_TOOLS_PROFILE));
        assert!(!report_passed(&report));
        assert_eq!(report.summary.missing, 1);
    }

    #[test]
    fn missing_real_tools_prerequisites_are_missing_not_skipped() {
        let availability = RealToolAvailability::default();
        let reason =
            missing_prerequisites(&[Prerequisite::Aapt2, Prerequisite::Dexdump], availability)
                .expect("both missing");
        assert_eq!(reason, "missing pinned tool(s): aapt2, dexdump");
        assert!(missing_prerequisites(
            &[Prerequisite::Aapt2],
            RealToolAvailability {
                aapt2: true,
                ..RealToolAvailability::default()
            }
        )
        .is_none());
    }

    #[test]
    fn analysis_exit_json_and_text_mismatches_fail() {
        let context = NormalizationContext::for_test("/tmp/parity-case");
        let json_spec = select_analysis_specs(&["manifest-json".to_string()])
            .expect("spec")
            .remove(0);
        let normalizer = Normalizer::new(context.clone()).at(json_spec);
        let differences = compare_case(
            json_spec,
            &result(0, r#"[{"type":"activity"}]"#, ""),
            &result(0, r#"[{"type":"service"}]"#, ""),
            &normalizer,
        );
        assert!(
            differences
                .iter()
                .any(|line| line.contains("/0/type: python")),
            "{differences:?}"
        );

        let text_spec = select_analysis_specs(&["res-scalar".to_string()])
            .expect("spec")
            .remove(0);
        let normalizer = Normalizer::new(context).at(text_spec);
        let differences = compare_case(
            text_spec,
            &result(0, "Found 1 matching resources:\n", ""),
            &result(0, "Found 2 matching resources:\n", ""),
            &normalizer,
        );
        assert!(
            differences
                .iter()
                .any(|line| line.contains("stdout differs")),
            "{differences:?}"
        );
    }

    #[test]
    fn analysis_report_is_deterministic_and_names_profile() {
        let cases = vec![CaseResult {
            id: "asm-encode".to_string(),
            argv: vec!["asm".to_string(), "ret".to_string()],
            status: CaseStatus::Passed,
            approved: false,
            approval: None,
            exit: ExitPair { python: 0, rust: 0 },
            differences: Vec::new(),
        }];
        let report = build_report_for(cases, ANALYSIS_REPORT_KIND, Some(SYNTHETIC_PROFILE));
        assert_eq!(report.summary.total, 1);
        assert_eq!(report.summary.passed, 1);
        let first = render_report(&report).expect("render");
        let second = render_report(&report).expect("render twice");
        assert_eq!(first, second);
        let parsed: Value = serde_json::from_str(&first).expect("json");
        assert_eq!(parsed["kind"], "apk-lab-analysis-parity");
        assert_eq!(parsed["profile"], "synthetic");
        assert_eq!(
            parsed["summary"],
            serde_json::json!({
                "total": 1,
                "passed": 1,
                "failed": 0,
                "missing": 0,
                "approved": 0
            })
        );
    }

    #[test]
    fn invalid_corpus_is_refused_before_reporting() {
        let directory = tempfile::tempdir().expect("tempdir");
        let corpus = directory.path().join("corpus.json");
        let report = directory.path().join("report.json");
        fs::write(&corpus, "{ not json").expect("write corpus");
        let error = run(&[
            "--group".to_string(),
            "analysis".to_string(),
            "--profile".to_string(),
            "real-tools".to_string(),
            "--corpus".to_string(),
            corpus.to_string_lossy().into_owned(),
            "--python-project".to_string(),
            ".".to_string(),
            "--rust-bin".to_string(),
            "/bin/sh".to_string(),
            "--report".to_string(),
            report.to_string_lossy().into_owned(),
        ])
        .expect_err("invalid corpus must be rejected");
        assert!(error.contains("invalid corpus"), "{error}");
        assert!(!report.exists(), "no report may be written");
    }

    #[test]
    fn corpus_validation_rejects_unknown_case_and_placeholder() {
        let directory = tempfile::tempdir().expect("tempdir");
        let artifact = directory.path().join("base.apk");
        fs::write(&artifact, b"apk").expect("artifact");

        let unknown_case = directory.path().join("unknown-case.json");
        fs::write(
            &unknown_case,
            serde_json::to_vec(&serde_json::json!({
                "kind": CORPUS_KIND,
                "version": CORPUS_VERSION,
                "artifacts": {"base": "base.apk"},
                "cases": [{"id": "not-a-case", "argv": ["asm", "ret"]}]
            }))
            .expect("json"),
        )
        .expect("write");
        let error = load_corpus(&unknown_case).expect_err("unknown case");
        assert!(error.contains("unknown analysis case"), "{error}");

        let unknown_artifact = directory.path().join("unknown-artifact.json");
        fs::write(
            &unknown_artifact,
            serde_json::to_vec(&serde_json::json!({
                "kind": CORPUS_KIND,
                "version": CORPUS_VERSION,
                "artifacts": {"base": "base.apk"},
                "cases": [{"id": "manifest-json", "argv": ["manifest", "$other", "--json"]}]
            }))
            .expect("json"),
        )
        .expect("write");
        let error = load_corpus(&unknown_artifact).expect_err("unknown artifact");
        assert!(error.contains("unknown artifact '$other'"), "{error}");

        let valid = directory.path().join("valid.json");
        fs::write(
            &valid,
            serde_json::to_vec(&serde_json::json!({
                "kind": CORPUS_KIND,
                "version": CORPUS_VERSION,
                "artifacts": {"base": "base.apk"},
                "cases": [{"id": "manifest-json", "argv": ["manifest", "$base", "--json"]}]
            }))
            .expect("json"),
        )
        .expect("write");
        let corpus = load_corpus(&valid).expect("valid corpus");
        let resolved = &corpus.cases["manifest-json"];
        assert_eq!(resolved[0], "manifest");
        assert_eq!(
            resolved[1],
            fs::canonicalize(&artifact)
                .expect("canonical")
                .to_string_lossy()
        );
    }

    #[test]
    fn only_the_approved_analysis_difference_is_allowlisted() {
        assert_eq!(ANALYSIS_APPROVED_CASES.len(), 1);
        let reason = approved_reason_in(ANALYSIS_APPROVED_CASES, "res-no-json")
            .expect("res --json rejection is approved");
        assert!(
            reason.contains("both reject --json with exit 2"),
            "{reason}"
        );
        for spec in ANALYSIS_CASES {
            if spec.spec.id == "res-no-json" {
                continue;
            }
            assert!(
                approved_reason_in(ANALYSIS_APPROVED_CASES, spec.spec.id).is_none(),
                "unexpected analysis approval for {}",
                spec.spec.id
            );
        }
        let spec = select_analysis_specs(&["res-no-json".to_string()])
            .expect("spec")
            .remove(0);
        let prepared = PreparedCase::new(spec, vec!["res".into(), "--json".into()]);
        let context = NormalizationContext::for_test("/tmp/parity-case");
        let normalizer = Normalizer::new(context).at(spec);
        let python = result(
            2,
            "",
            "usage: apk-lab ...\napk-lab: error: unrecognized arguments: --json\n",
        );
        let rust = result(
            2,
            "",
            "Usage: apk-lab res ...\napk-lab: unrecognized option '--json'\n",
        );
        let differences = compare_case(spec, &python, &rust, &normalizer);
        assert!(!differences.is_empty());
        let approval = approved_reason_in(ANALYSIS_APPROVED_CASES, &prepared.id);
        assert!(approval.is_some());
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
        let selected = select_core_cases(&[]).expect("all cases");
        assert_eq!(selected.len(), CORE_CASES.len());
        assert_eq!(selected[0].id, "help");
        assert_eq!(selected[0].stream.as_str(), "stdout");
        assert_eq!(case("inspect-missing-artifact").stream, StreamMode::Stderr);
        assert_eq!(
            select_core_cases(&["version".to_string(), "help".to_string()])
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
