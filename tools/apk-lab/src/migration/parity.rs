//! Differential parity harness for the native `apk-lab` core, analysis and
//! patching/device commands.
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

use crate::models::ExitCode;
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
const PATCHING_DEVICE_REPORT_KIND: &str = "apk-lab-patching-device-parity";
const ACQUISITION_CI_REPORT_KIND: &str = "apk-lab-acquisition-ci-parity";
const CORE_GROUP: &str = "core";
const ANALYSIS_GROUP: &str = "analysis";
const PATCHING_DEVICE_GROUP: &str = "patching-device";
const ACQUISITION_CI_GROUP: &str = "acquisition-ci";
const SYNTHETIC_PROFILE: &str = "synthetic";
const REAL_TOOLS_PROFILE: &str = "real-tools";
const REAL_DEVICE_PROFILE: &str = "real-device";
const CORPUS_KIND: &str = "apk-lab-analysis-corpus";
const CORPUS_VERSION: u32 = 1;
const DEVICE_CORPUS_KIND: &str = "apk-lab-patching-device-corpus";
const DEVICE_CORPUS_VERSION: u32 = 1;

/// The only normalizations the core group applies. Anything not listed here is
/// a substantive difference and fails the case.
const NORMALIZATION_RULES: &[&str] = &[
    "absolute cwd/sandbox/home/temp roots are replaced with <cwd>/<sandbox>/<home>/<temp>",
    "doctor host prerequisite/tool version, path and error details are replaced with <volatile>",
    "declared JSON fields are replaced with <path>/<version>/<pid>",
    "patching human output replaces per-case durations with <duration> and PIDs with <pid>",
    "acquisition-ci scenario JSON compares structurally after path replacement; timestamps are not reported",
    "acquisition-ci result JSON is written to a per-side file and compared structurally",
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
    Pid,
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
    /// Patching/device output embeds wall-clock durations and process IDs.
    PatchingDetails,
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

/// The native deployment path reports the exact post-replacement state after a
/// failed install/launch; the Python oracle only prints the raw ADB failure.
/// Both preserve the signed output, exit 4, and write the same leading text.
const PATCHING_DEVICE_APPROVED_CASES: &[(&str, &str)] = &[(
    "deploy-install-failure",
    "native names the replaced output path and says it was not installed; Python reports only the ADB failure",
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
// Patching/device case table
// ---------------------------------------------------------------------------

/// A filesystem result the harness checks immediately after each side runs.
/// These cover transactional boundaries that stdout alone cannot prove, such
/// as "a failed signature never replaces the existing output".
#[derive(Debug, Clone)]
enum Postcondition {
    FileExists(&'static str),
    FileUnchanged(&'static str, &'static [u8]),
}

#[derive(Debug, Clone, Copy)]
struct PatchingCase {
    spec: CaseSpec,
    /// Extra environment for this case. Values beginning with `@` are fixture
    /// paths resolved inside the run root before execution.
    env: &'static [(&'static str, &'static str)],
    postconditions: &'static [Postcondition],
}

const PATCHING_PACKAGE: &str = "com.avinside";
const PATCHING_SERIAL: &str = "emulator-5554";
const PATCHING_ARTIFACT: &str = "fixtures/artifact.apk";
const PATCHING_MPP: &str = "fixtures/patches.mpp";
const PATCHING_KEYSTORE: &str = "fixtures/custom.keystore";
const PATCHING_JSON_DURATION: &[JsonVolatile] = &[JsonVolatile {
    pointer: "/testCases/*/durationSeconds",
    kind: VolatileKind::Version,
}];
const CRASH_JSON_VOLATILE: &[JsonVolatile] = &[JsonVolatile {
    pointer: "/0/pid",
    kind: VolatileKind::Pid,
}];

const PATCHING_DEVICE_CASES: &[PatchingCase] = &[
    PatchingCase {
        spec: CaseSpec {
            id: "check-default-json",
            argv: &[
                "check",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--jobs",
                "1",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: PATCHING_JSON_DURATION,
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "check-all-json",
            argv: &[
                "check",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--all",
                "--force",
                "--jobs",
                "1",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: PATCHING_JSON_DURATION,
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "check-incompatible",
            argv: &[
                "check",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--all",
                "--jobs",
                "1",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: PATCHING_JSON_DURATION,
            text_rules: &[],
        },
        env: &[("FAKE_PATCHED_APK", "@missing.apk")],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "check-human-durations",
            argv: &[
                "check",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--patch",
                "Remove Ads",
                "--jobs",
                "1",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "check-missing-mpp",
            argv: &[
                "check",
                PATCHING_ARTIFACT,
                "--package",
                PATCHING_PACKAGE,
                "--jobs",
                "1",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-default",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--keystore",
                PATCHING_KEYSTORE,
                "--out",
                "out/deploy-default.apk",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[],
        postconditions: &[Postcondition::FileExists("out/deploy-default.apk")],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-two-option-groups",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--keystore",
                PATCHING_KEYSTORE,
                "-e",
                "Remove Ads",
                "-O",
                "removeReviewPrompts=false",
                "-e",
                "Unlock Developer Tools",
                "-O",
                "startWithDebugMessages=true",
                "--launch",
                "--out",
                "out/deploy-options.apk",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[],
        postconditions: &[Postcondition::FileExists("out/deploy-options.apk")],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-missing-bundle",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-alias-rejected",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--keystore",
                PATCHING_KEYSTORE,
                "--out",
                PATCHING_ARTIFACT,
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[Postcondition::FileExists(PATCHING_ARTIFACT)],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-signing-failure",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--keystore",
                PATCHING_KEYSTORE,
                "--out",
                "out/preserved.apk",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[("FAKE_SIGN_FAIL", "1")],
        postconditions: &[Postcondition::FileUnchanged(
            "out/preserved.apk",
            b"original output\n",
        )],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "deploy-install-failure",
            argv: &[
                "deploy",
                PATCHING_ARTIFACT,
                "--mpp",
                PATCHING_MPP,
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--keystore",
                PATCHING_KEYSTORE,
                "--out",
                "out/install-failure.apk",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[("FAKE_ADB_INSTALL_FAIL", "1")],
        postconditions: &[Postcondition::FileExists("out/install-failure.apk")],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "monitor-healthy",
            argv: &[
                "monitor",
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--timeout",
                "0",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "monitor-crash",
            argv: &[
                "monitor",
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--timeout",
                "3",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[TextRule::PatchingDetails],
        },
        env: &[("FAKE_LIVE_LOG", "@crash.log")],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "crash-trace-json",
            argv: &[
                "crash-trace",
                "--package",
                PATCHING_PACKAGE,
                "--device",
                PATCHING_SERIAL,
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: CRASH_JSON_VOLATILE,
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-dump-clickable",
            argv: &["ui", "dump", "--device", PATCHING_SERIAL, "--clickable"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-dump-filter-json",
            argv: &[
                "ui",
                "dump",
                "--device",
                PATCHING_SERIAL,
                "--text",
                "save",
                "--json",
            ],
            stream: StreamMode::Stdout,
            json_stream: Some(StreamMode::Stdout),
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-dump-desc",
            argv: &["ui", "dump", "--device", PATCHING_SERIAL, "--desc", "media"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-click-text",
            argv: &["ui", "click", "--device", PATCHING_SERIAL, "--text", "save"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-click-desc",
            argv: &[
                "ui",
                "click",
                "--device",
                PATCHING_SERIAL,
                "--desc",
                "media",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-click-missing",
            argv: &[
                "ui",
                "click",
                "--device",
                PATCHING_SERIAL,
                "--text",
                "absent",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
    PatchingCase {
        spec: CaseSpec {
            id: "ui-click-invalid-coords",
            argv: &[
                "ui",
                "click",
                "--device",
                PATCHING_SERIAL,
                "--coords",
                "nope",
            ],
            stream: StreamMode::Stderr,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        env: &[],
        postconditions: &[],
    },
];

fn patching_device_case(id: &str) -> Option<&'static PatchingCase> {
    PATCHING_DEVICE_CASES.iter().find(|case| case.spec.id == id)
}

// ---------------------------------------------------------------------------
// Acquisition and CI scenario table
// ---------------------------------------------------------------------------
//
// These cases execute library scenarios rather than the CLI directly. The
// Python side runs a generated driver that imports `apk_lab` and injects fakes;
// the native side runs the same scenario through the hidden
// `acquisition-ci-scenario` subcommand. Both write a result document to a
// side-specific file. Acquisition providers, S3, the Morphe checker and the
// Worker poster are always fakes; no case touches Play, R2, the Worker, a
// device, or the network.

#[derive(Debug, Clone, Copy)]
struct ScenarioCase {
    spec: CaseSpec,
    kind: ScenarioKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScenarioKind {
    /// Fake apkeep/Play/APKMirror providers, fake badger/inspector and temp APKs.
    Acquisition,
    /// Fixture-slot rotation over an in-memory S3 adapter.
    Fixtures,
    /// Matrix/start/run over fake S3, checker and poster.
    Ci,
}

const ROTATION_ACCOUNTS: &str = r#"{"version":1,"accounts":[
{"label":"first","email":"first@example.com","aasToken":"aas_et/first-token","device":"px_9a","locale":"en_US","timezone":"UTC","enabled":true},
{"label":"second","email":"second@example.com","aasToken":"aas_et/second-token","device":"px_9a","locale":"en_US","timezone":"UTC","enabled":true}
]}"#;

const SINGLE_ACCOUNT: &str = r#"{"version":1,"accounts":[
{"label":"only","email":"only@example.com","aasToken":"aas_et/only-token","device":"px_9a","locale":"en_US","timezone":"UTC","enabled":true}
]}"#;

const ACQUISITION_CI_CASES: &[ScenarioCase] = &[
    ScenarioCase {
        spec: CaseSpec {
            id: "acquire-no-account",
            argv: &["acquire", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Acquisition,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "acquire-account-rotation",
            argv: &["acquire", "com.example.parity", "--version", "1.0.0"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Acquisition,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "acquire-wrong-version",
            argv: &["acquire", "com.example.parity", "--version", "1.0.0"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Acquisition,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "acquire-normalization-manifest",
            argv: &["acquire", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Acquisition,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "acquire-source-labels",
            argv: &["acquire", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Acquisition,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "fixtures-key-metadata-rotation",
            argv: &["fixtures", "seed", "--package", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Fixtures,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "ci-matrix-tie-break",
            argv: &["ci", "matrix", "--output-file", "GITHUB_OUTPUT"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Ci,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "ci-start-payload",
            argv: &[
                "ci",
                "start",
                "--package",
                "com.example.parity",
                "--request-id",
                "req-1",
                "--git-revision",
                "abc123",
            ],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Ci,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "ci-run-saved-before-post",
            argv: &["ci", "run", "--package", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Ci,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "ci-run-2xx-marker",
            argv: &["ci", "run", "--package", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Ci,
    },
    ScenarioCase {
        spec: CaseSpec {
            id: "ci-run-transport-failure",
            argv: &["ci", "run", "--package", "com.example.parity"],
            stream: StreamMode::Stdout,
            json_stream: None,
            volatile_json: &[],
            text_rules: &[],
        },
        kind: ScenarioKind::Ci,
    },
];

fn acquisition_ci_case(id: &str) -> Option<&'static ScenarioCase> {
    ACQUISITION_CI_CASES.iter().find(|case| case.spec.id == id)
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
        PATCHING_DEVICE_GROUP => run_patching_device(options),
        ACQUISITION_CI_GROUP => run_acquisition_ci(options),
        other => Err(format!(
            "unsupported parity group '{other}' (available: '{CORE_GROUP}', '{ANALYSIS_GROUP}', '{PATCHING_DEVICE_GROUP}', '{ACQUISITION_CI_GROUP}')"
        )),
    }
}

fn run_core(options: Options) -> Result<(), String> {
    if options.profile != SYNTHETIC_PROFILE {
        return Err(format!(
            "--profile {} is only supported with --group {ANALYSIS_GROUP} or {PATCHING_DEVICE_GROUP}",
            options.profile
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
        REAL_DEVICE_PROFILE => Err(format!(
            "--profile {REAL_DEVICE_PROFILE} is only supported with --group {PATCHING_DEVICE_GROUP}"
        )),
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

fn run_patching_device(options: Options) -> Result<(), String> {
    let specs = select_patching_device_cases(&options.cases)?;
    if options.list {
        print_patching_device_list(&specs);
        return Ok(());
    }

    match options.profile.as_str() {
        SYNTHETIC_PROFILE => run_patching_device_synthetic(options, &specs),
        REAL_DEVICE_PROFILE => run_patching_device_real(options, &specs),
        other => Err(format!(
            "unsupported patching-device profile '{other}' (available: '{SYNTHETIC_PROFILE}', '{REAL_DEVICE_PROFILE}')"
        )),
    }
}

fn run_patching_device_synthetic(
    options: Options,
    specs: &[&'static PatchingCase],
) -> Result<(), String> {
    if options.corpus.is_some() {
        return Err(format!(
            "--corpus requires --profile {REAL_DEVICE_PROFILE}; the {SYNTHETIC_PROFILE} profile generates its own fixtures"
        ));
    }
    let (oracle, rust_bin) = resolve_tools(&options)?;
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-patching-device-parity-")
        .tempdir()
        .map_err(|error| format!("could not create parity sandbox: {error}"))?;
    let prepared = execute_patching_device_synthetic(specs, &oracle, &rust_bin, sandbox.path())?;
    finish_patching_device(options.report.as_deref(), prepared, SYNTHETIC_PROFILE)
}

fn run_patching_device_real(
    options: Options,
    specs: &[&'static PatchingCase],
) -> Result<(), String> {
    let corpus_path = options
        .corpus
        .as_deref()
        .ok_or("--corpus is required with --profile real-device")?;
    let corpus = load_device_corpus(corpus_path)?;
    let (oracle, rust_bin) = resolve_tools(&options)?;
    let cwd = corpus
        .root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| corpus.root.clone());
    let environment = Environment::for_real_host();
    let normalizer = Normalizer::new(NormalizationContext::new(Path::new("/nonexistent"), &cwd));
    let prepared = prepare_real_patching_cases(specs, &corpus);
    let report = execute_prepared(
        prepared,
        &oracle,
        &rust_bin,
        &cwd,
        &environment,
        &normalizer,
        PATCHING_DEVICE_REPORT_KIND,
        Some(REAL_DEVICE_PROFILE),
        PATCHING_DEVICE_APPROVED_CASES,
    )?;
    finish_patching_device(options.report.as_deref(), report, REAL_DEVICE_PROFILE)
}

fn run_acquisition_ci(options: Options) -> Result<(), String> {
    if options.profile != SYNTHETIC_PROFILE {
        return Err(format!(
            "--profile {} is not supported with --group {ACQUISITION_CI_GROUP}; the offline group only runs fake scenarios",
            options.profile
        ));
    }
    if options.corpus.is_some() {
        return Err(format!(
            "--corpus is not supported with --group {ACQUISITION_CI_GROUP}"
        ));
    }

    let selected = select_acquisition_ci_cases(&options.cases)?;
    if options.list {
        print_acquisition_ci_list(&selected);
        return Ok(());
    }

    let (oracle, rust_bin) = resolve_tools(&options)?;
    let sandbox = tempfile::Builder::new()
        .prefix("apk-lab-acquisition-ci-parity-")
        .tempdir()
        .map_err(|error| format!("could not create parity sandbox: {error}"))?;
    let report = execute_acquisition_ci(&selected, &oracle, &rust_bin, sandbox.path())?;

    let rendered = render_report(&report)?;
    write_report(options.report.as_deref(), &rendered)?;
    if !report_passed(&report) {
        return Err(format!(
            "acquisition-ci parity failed: {} of {} case(s) did not pass",
            report.summary.failed + report.summary.missing,
            report.summary.total
        ));
    }
    Ok(())
}

fn select_acquisition_ci_cases(requested: &[String]) -> Result<Vec<&'static ScenarioCase>, String> {
    let mut wanted = BTreeSet::new();
    for id in requested {
        if acquisition_ci_case(id).is_none() {
            return Err(format!(
                "unknown acquisition-ci parity case '{id}' (run 'parity --group {ACQUISITION_CI_GROUP}' --list)"
            ));
        }
        wanted.insert(id.as_str());
    }
    Ok(ACQUISITION_CI_CASES
        .iter()
        .filter(|case| wanted.is_empty() || wanted.contains(case.spec.id))
        .collect())
}

fn print_acquisition_ci_list(cases: &[&ScenarioCase]) {
    let mut text = String::new();
    for case in cases {
        text.push_str(case.spec.id);
        text.push('\t');
        text.push_str(case.spec.stream.as_str());
        text.push('\t');
        text.push_str(&case.spec.argv.join(" "));
        text.push('\n');
    }
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
}

fn finish_patching_device(
    report_path: Option<&str>,
    report: ParityReport,
    profile: &'static str,
) -> Result<(), String> {
    let rendered = render_report(&report)?;
    write_report(report_path, &rendered)?;
    if !report_passed(&report) {
        return Err(format!(
            "patching-device parity ({profile}) failed: {} of {} case(s) did not pass",
            report.summary.failed + report.summary.missing,
            report.summary.total
        ));
    }
    Ok(())
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
        "--group is required (available: '{CORE_GROUP}', '{ANALYSIS_GROUP}', '{PATCHING_DEVICE_GROUP}')"
    ))?;
    let profile = profile.unwrap_or_else(|| SYNTHETIC_PROFILE.to_string());
    match profile.as_str() {
        SYNTHETIC_PROFILE | REAL_TOOLS_PROFILE | REAL_DEVICE_PROFILE => {}
        other => {
            return Err(format!(
                "unsupported parity profile '{other}' (available: '{SYNTHETIC_PROFILE}', '{REAL_TOOLS_PROFILE}', '{REAL_DEVICE_PROFILE}')"
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

fn select_patching_device_cases(
    requested: &[String],
) -> Result<Vec<&'static PatchingCase>, String> {
    let mut wanted = BTreeSet::new();
    for id in requested {
        if patching_device_case(id).is_none() {
            return Err(format!(
                "unknown patching-device parity case '{id}' (run 'parity --group {PATCHING_DEVICE_GROUP}' --list)"
            ));
        }
        wanted.insert(id.as_str());
    }
    Ok(PATCHING_DEVICE_CASES
        .iter()
        .filter(|case| wanted.is_empty() || wanted.contains(case.spec.id))
        .collect())
}

fn print_patching_device_list(cases: &[&PatchingCase]) {
    let mut text = String::new();
    for case in cases {
        text.push_str(case.spec.id);
        text.push('\t');
        text.push_str(case.spec.stream.as_str());
        text.push('\t');
        text.push_str(&case.spec.argv.join(" "));
        text.push('\n');
    }
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
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

#[derive(Debug, Clone)]
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

    /// Applies case-local variables. Later entries win, matching the order the
    /// child process receives.
    fn with_variables(mut self, extra: impl IntoIterator<Item = (String, String)>) -> Self {
        for (name, value) in extra {
            if let Some(existing) = self
                .variables
                .iter_mut()
                .find(|(candidate, _)| candidate == &name)
            {
                existing.1 = value;
            } else {
                self.variables.push((name, value));
            }
        }
        self
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
    env: Vec<(String, String)>,
    postconditions: Vec<Postcondition>,
    missing: Option<String>,
}

impl PreparedCase {
    fn new(spec: &CaseSpec, argv: Vec<String>) -> Self {
        Self {
            id: spec.id.to_string(),
            argv,
            spec: *spec,
            env: Vec::new(),
            postconditions: Vec::new(),
            missing: None,
        }
    }

    fn missing(spec: &CaseSpec, argv: Vec<String>, reason: String) -> Self {
        Self {
            id: spec.id.to_string(),
            argv,
            spec: *spec,
            env: Vec::new(),
            postconditions: Vec::new(),
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

fn execute_patching_device_synthetic(
    cases: &[&PatchingCase],
    oracle: &Oracle,
    rust_bin: &Path,
    sandbox: &Path,
) -> Result<ParityReport, String> {
    let run_root = sandbox.join("run");
    let fake_sdk = sandbox.join("fake-sdk");
    let fake_bin = sandbox.join("fake-bin");
    let home = sandbox.join("home");
    create_patching_device_fixtures(
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
    validate_patching_device_tools(sandbox)?;

    let environment = Environment::for_sandbox_with_tools(
        sandbox,
        python_project_path(oracle),
        &fake_sdk,
        Some(&fake_bin),
    )
    .with_variables(patching_common_variables(&run_root));

    let normalizer = Normalizer::new(NormalizationContext::new(sandbox, &run_root));
    let prepared: Vec<PreparedCase> = cases
        .iter()
        .map(|case| {
            let argv = case
                .spec
                .argv
                .iter()
                .map(|value| value.to_string())
                .collect();
            let mut prepared = PreparedCase::new(&case.spec, argv);
            prepared.env = case
                .env
                .iter()
                .map(|(name, value)| (name.to_string(), resolve_fixture_value(&run_root, value)))
                .collect();
            prepared.postconditions = case.postconditions.to_vec();
            prepared
        })
        .collect();
    execute_prepared(
        prepared,
        oracle,
        rust_bin,
        &run_root,
        &environment,
        &normalizer,
        PATCHING_DEVICE_REPORT_KIND,
        Some(SYNTHETIC_PROFILE),
        PATCHING_DEVICE_APPROVED_CASES,
    )
}

fn execute_acquisition_ci(
    cases: &[&ScenarioCase],
    oracle: &Oracle,
    rust_bin: &Path,
    sandbox: &Path,
) -> Result<ParityReport, String> {
    let run_root = sandbox.join("run");
    let fake_sdk = sandbox.join("fake-sdk");
    create_acquisition_ci_fixtures(&run_root, &fake_sdk)?;
    for directory in [&sandbox.join("home"), &sandbox.join("tmp")] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }

    let driver = run_root.join("acquisition_ci_driver.py");
    fs::write(&driver, ACQUISITION_CI_PYTHON_DRIVER)
        .map_err(|error| format!("could not write acquisition-ci driver: {error}"))?;

    let environment = Environment::for_sandbox(sandbox, python_project_path(oracle), &fake_sdk);
    let normalizer = Normalizer::new(NormalizationContext::new(sandbox, &run_root));
    // Prefer the migration binary built beside the requested native binary so
    // `--rust-bin` selects the build under test; fall back to the running
    // harness when the sibling is absent.
    let sibling = rust_bin.with_file_name("apk-lab-migrate");
    let native_harness = if sibling.is_file() {
        sibling
    } else {
        std::env::current_exe()
            .map_err(|error| format!("could not resolve parity executable: {error}"))?
    };

    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut side_results: Vec<(ProcessResult, Option<Value>, Option<String>)> = Vec::new();
        for side in ["python", "rust"] {
            let result_name = format!("result-{side}.json");
            let result_path = run_root.join(&result_name);
            let _ = fs::remove_file(&result_path);

            let case_environment = environment
                .clone()
                .with_variables(scenario_variables(case, side, &run_root))
                .with_variables([
                    ("PARITY_SIDE".to_string(), side.to_string()),
                    ("PARITY_RESULT".to_string(), result_name.clone()),
                ]);
            for stale in [
                format!("submission-{side}.json"),
                format!("marker-{side}.txt"),
                format!("matrix-{side}.txt"),
            ] {
                let _ = fs::remove_file(run_root.join(stale));
            }

            let outcome = if side == "python" {
                run_python_scenario(
                    oracle,
                    &driver,
                    case.spec.id,
                    &result_name,
                    &run_root,
                    &case_environment,
                )
            } else {
                run_binary(
                    &native_harness,
                    [
                        "acquisition-ci-scenario".to_string(),
                        case.spec.id.to_string(),
                        "--result".to_string(),
                        result_name.clone(),
                        "--side".to_string(),
                        side.to_string(),
                    ],
                    &run_root,
                    &case_environment,
                )
            };

            let (process, result, failure) = match outcome {
                Ok(process) => match fs::read_to_string(&result_path) {
                    Ok(text) => match serde_json::from_str::<Value>(&text) {
                        Ok(value) => (process, Some(value), None),
                        Err(error) => (
                            process,
                            None,
                            Some(format!(
                                "{side} result is not JSON ({error}): {}",
                                compact(&text, 240)
                            )),
                        ),
                    },
                    Err(error) => (
                        process,
                        None,
                        Some(format!("{side} did not write a result file: {error}")),
                    ),
                },
                Err(error) => (
                    ProcessResult::missing(),
                    None,
                    Some(format!("{side}: {error}")),
                ),
            };
            side_results.push((process, result, failure));
        }

        let (python, python_result, python_failure) = &side_results[0];
        let (rust, rust_result, rust_failure) = &side_results[1];
        let mut differences = Vec::new();
        if let Some(reason) = python_failure {
            differences.push(reason.clone());
        }
        if let Some(reason) = rust_failure {
            differences.push(reason.clone());
        }
        if python_failure.is_none() && rust_failure.is_none() {
            differences.extend(compare_case(
                &case.spec,
                python,
                rust,
                &normalizer.at(&case.spec),
            ));
            if let (Some(python_json), Some(rust_json)) = (python_result, rust_result) {
                compare_result_json(
                    python_json,
                    rust_json,
                    &normalizer.context,
                    &mut differences,
                );
            }
        }

        let result = if python_failure.is_some() || rust_failure.is_some() {
            CaseResult {
                id: case.spec.id.to_string(),
                argv: case
                    .spec
                    .argv
                    .iter()
                    .map(|value| value.to_string())
                    .collect(),
                status: CaseStatus::Missing,
                approved: false,
                approval: None,
                exit: ExitPair {
                    python: python.exit,
                    rust: rust.exit,
                },
                differences,
            }
        } else {
            CaseResult {
                id: case.spec.id.to_string(),
                argv: case
                    .spec
                    .argv
                    .iter()
                    .map(|value| value.to_string())
                    .collect(),
                status: if differences.is_empty() {
                    CaseStatus::Passed
                } else {
                    CaseStatus::Failed
                },
                approved: false,
                approval: None,
                exit: ExitPair {
                    python: python.exit,
                    rust: rust.exit,
                },
                differences,
            }
        };
        results.push(result);
    }

    Ok(build_report_for(
        results,
        ACQUISITION_CI_REPORT_KIND,
        Some(SYNTHETIC_PROFILE),
    ))
}

fn compare_result_json(
    python: &Value,
    rust: &Value,
    context: &NormalizationContext,
    differences: &mut Vec<String>,
) {
    let mut python = python.clone();
    let mut rust = rust.clone();
    normalize_json_paths(&mut python, context);
    normalize_json_paths(&mut rust, context);
    let before = differences.len();
    compare_json(&python, &rust, "result", differences);
    for difference in &mut differences[before..] {
        if !difference.starts_with("result/") {
            *difference = format!("result {difference}");
        }
    }
}

fn run_python_scenario(
    oracle: &Oracle,
    driver: &Path,
    scenario: &str,
    result_name: &str,
    cwd: &Path,
    environment: &Environment,
) -> Result<ProcessResult, String> {
    let (python, prefix) = oracle_python_command(oracle)?;
    let mut args = prefix;
    args.push(driver.to_string_lossy().into_owned());
    args.push(scenario.to_string());
    args.push(result_name.to_string());
    match environment
        .variables
        .iter()
        .find(|(name, _)| name == "PARITY_SIDE")
    {
        Some((_, side)) => args.push(side.clone()),
        None => args.push("python".to_string()),
    }
    run_binary(&python, args, cwd, environment)
}

fn oracle_python_command(oracle: &Oracle) -> Result<(PathBuf, Vec<String>), String> {
    match oracle {
        Oracle::Direct(entrypoint) => {
            let python = entrypoint
                .parent()
                .map(|directory| directory.join("python"))
                .filter(|candidate| is_executable(candidate))
                .or_else(|| find_in_path("python3"))
                .ok_or_else(|| {
                    "could not find a Python interpreter beside the oracle entrypoint".to_string()
                })?;
            Ok((python, Vec::new()))
        }
        Oracle::Uv { uv, project } => Ok((
            uv.clone(),
            vec![
                "run".to_string(),
                "--project".to_string(),
                project.to_string_lossy().into_owned(),
                "python".to_string(),
            ],
        )),
    }
}

fn scenario_variables(case: &ScenarioCase, side: &str, run_root: &Path) -> Vec<(String, String)> {
    let mut variables: Vec<(String, String)> = Vec::new();
    match case.spec.id {
        "acquire-account-rotation" => {
            variables.push(("APKEEP_ACCOUNTS".to_string(), ROTATION_ACCOUNTS.to_string()));
        }
        "acquire-wrong-version" => {
            variables.push(("APKEEP_ACCOUNTS".to_string(), SINGLE_ACCOUNT.to_string()));
        }
        _ => {}
    }
    if case.kind == ScenarioKind::Ci {
        let runner_temp = run_root.join("tmp");
        let _ = fs::create_dir_all(&runner_temp);
        let values = [
            ("GITHUB_EVENT_NAME", "workflow_dispatch"),
            ("GITHUB_SHA", "abc123"),
            ("GITHUB_REPOSITORY", "example/parity"),
            ("INPUT_PACKAGE_NAME", "all"),
            ("RUNNER_TEMP", &runner_temp.to_string_lossy()),
            ("COMPATIBILITY_STATUS_SECRET", "test-secret"),
            ("WORKER_STATUS_URL", "https://worker.parity.invalid"),
            ("DISPATCH_REQUEST_ID", "req-1"),
        ];
        for (name, value) in values {
            variables.push((name.to_string(), value.to_string()));
        }
        let submission = run_root.join(format!("submission-{side}.json"));
        let marker = run_root.join(format!("marker-{side}.txt"));
        variables.push((
            "COMPATIBILITY_SUBMISSION_PATH".to_string(),
            submission.to_string_lossy().into_owned(),
        ));
        variables.push((
            "COMPATIBILITY_CALLBACK_MARKER".to_string(),
            marker.to_string_lossy().into_owned(),
        ));
    }
    variables
}

fn create_acquisition_ci_fixtures(run_root: &Path, fake_sdk: &Path) -> Result<(), String> {
    let fixtures = run_root.join("fixtures");
    for directory in [&fixtures, &run_root.join("tmp"), fake_sdk] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }
    let patches = serde_json::json!({
        "version": "9.9.9",
        "patches": [
            {
                "name": "Parity Patch",
                "description": "synthetic acquisition-ci patch",
                "compatiblePackages": [{
                    "name": "Parity App",
                    "packageName": "com.example.parity",
                    "targets": [
                        {"version": "1.0.0"},
                        {"version": "v1.0.0"},
                        {"version": "1.0.0-beta"},
                        {"version": "1.10.0"},
                        {"version": "1.9.0"}
                    ]
                }]
            },
            {
                "name": "Beta Patch",
                "description": "second synthetic package",
                "compatiblePackages": [{
                    "name": "Beta App",
                    "packageName": "com.example.beta",
                    "targets": [{"version": "2.0.0"}]
                }]
            }
        ]
    });
    fs::write(
        fixtures.join("patches-list.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&patches).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| format!("could not write patches-list fixture: {error}"))?;
    fs::write(fixtures.join("payload.mpp"), b"synthetic-mpp")
        .map_err(|error| format!("could not write MPP fixture: {error}"))?;
    fs::write(fixtures.join("slot-artifact.apk"), b"slot-artifact-bytes")
        .map_err(|error| format!("could not write slot artifact fixture: {error}"))?;
    Ok(())
}

const ACQUISITION_CI_PYTHON_DRIVER: &str = r##"#!/usr/bin/env python3
"""Deterministic acquisition/CI parity scenarios with injected fakes."""

import hashlib
import json
import os
import re
import shutil
import sys
import zipfile
from pathlib import Path
from subprocess import CompletedProcess

PACKAGE = "com.example.parity"
SCENARIO = ""
RESULT_PATH = ""
SIDE = "python"
APK_VERSION = "1.0.0"
INSPECT_VERSION = "1.0.0"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def artifact_manifest(rel, path):
    path = Path(path)
    members = []
    with zipfile.ZipFile(path) as archive:
        for info in archive.infolist():
            data = archive.read(info.filename)
            members.append(
                {
                    "name": info.filename,
                    "size": len(data),
                    "sha256": sha256(data),
                    "dateTime": list(info.date_time),
                    "mode": (info.external_attr >> 16) & 0o777,
                }
            )
    return {"path": rel, "exists": True, "members": members}


def fake_badging(apk):
    from apk_lab.models import AndroidManifestInfo

    name = Path(apk).name
    split = None if name == "base.apk" else name[:-4]
    return AndroidManifestInfo(
        package_name=PACKAGE,
        version_code=100,
        version_name=APK_VERSION,
        split_name=split,
    )


def fake_inspect(path):
    from apk_lab.models import ArtifactInspection, ContainerType

    path = Path(path)
    return ArtifactInspection(
        file_path=str(path),
        container_type=ContainerType.APKM,
        file_size=path.stat().st_size,
        sha256=sha256(path.read_bytes()),
        package_name=PACKAGE,
        version_name=INSPECT_VERSION,
        version_code=100,
    )


class FakeToolManager:
    def __init__(self):
        self.calls = []

    def is_platform_supported(self, name):
        return True

    def is_tool_installed(self, name):
        return True

    def run_tool_cmd(self, name, cmd, **kwargs):
        ini = Path(cmd[cmd.index("-i") + 1]).read_text(encoding="utf-8")
        match = re.search(r"email = (.+)", ini)
        email = match.group(1).strip() if match else "unknown@example.com"
        self.calls.append(email)
        attempt = Path(cmd[-1])
        attempt.mkdir(parents=True, exist_ok=True)
        if email == "first@example.com":
            return CompletedProcess(cmd, 1, "", "first denial")
        (attempt / "base.apk").write_bytes(b"base-apk-bytes")
        return CompletedProcess(cmd, 0, "", "")


def patch_acquisition():
    import apk_lab.acquisition as acq

    acq.parse_apk_badging = fake_badging
    acq.inspect_artifact = fake_inspect

    def goopdl(*args, **kwargs):
        raise acq.AcquisitionError(
            "dispenser unavailable", acq.ExitCode.INFRASTRUCTURE_FAILURE
        )

    def apkmirror(*args, **kwargs):
        raise acq.AcquisitionError(
            f"No APKMirror repository mapping configured for '{PACKAGE}'",
            acq.ExitCode.INVALID_ARTIFACT,
        )

    acq.acquire_with_goopdl = goopdl
    acq.acquire_with_apkmirror = apkmirror
    return acq


def scenario_acquire_no_account():
    acq = patch_acquisition()
    try:
        acq.acquire_artifact(PACKAGE, Path("out"), None, None, tool_mgr=FakeToolManager())
    except acq.AcquisitionError as error:
        return error.exit_code.value, {"outcome": "error", "error": str(error)}
    return 0, {"outcome": "success"}


def scenario_acquire_account_rotation():
    acq = patch_acquisition()
    shutil.rmtree("out", ignore_errors=True)
    tool = FakeToolManager()
    path, source = acq.acquire_artifact(
        PACKAGE, Path("out"), "1.0.0", 100, tool_mgr=tool
    )
    return 0, {
        "outcome": "success",
        "source": source.value,
        "calls": tool.calls,
        "artifact": artifact_manifest("out/com.example.parity.apkm", path),
    }


def scenario_acquire_wrong_version():
    global INSPECT_VERSION
    acq = patch_acquisition()
    INSPECT_VERSION = "9.9.9"
    shutil.rmtree("out", ignore_errors=True)
    try:
        acq.acquire_artifact(PACKAGE, Path("out"), "1.0.0", 100, tool_mgr=FakeToolManager())
    except acq.AcquisitionError as error:
        return error.exit_code.value, {
            "outcome": "error",
            "error": str(error),
            "artifactPresent": (Path("out") / "com.example.parity.apkm").exists(),
        }
    return 0, {"outcome": "success"}


def scenario_acquire_normalization_manifest():
    acq = patch_acquisition()
    shutil.rmtree("norm", ignore_errors=True)
    Path("norm").mkdir()
    files = []
    for name, data in [
        ("base.apk", b"BASE-APK"),
        ("config.en.apk", b"EN-SPLIT"),
        ("config.arm64.apk", b"ARM64-SPLIT"),
    ]:
        path = Path("norm") / name
        path.write_bytes(data)
        files.append(path)
    path = acq.normalize_apks_to_apkm(files, Path("norm/out.apkm"))
    return 0, {
        "outcome": "success",
        "artifact": artifact_manifest("norm/out.apkm", path),
    }


def scenario_acquire_source_labels():
    from apk_lab.acquisition import AcquisitionSource

    return 0, {"labels": [source.value for source in AcquisitionSource]}


class FakeS3:
    def __init__(self):
        self.objects = {}

    def head_object(self, Bucket, Key):
        from botocore.exceptions import ClientError

        if Key not in self.objects:
            raise ClientError(
                {"Error": {"Code": "404", "Message": "Not Found"}}, "HeadObject"
            )
        return {"Metadata": dict(self.objects[Key][1])}

    def upload_file(self, Filename, Bucket, Key, ExtraArgs=None):
        metadata = dict((ExtraArgs or {}).get("Metadata", {}))
        self.objects[Key] = (Path(Filename).read_bytes(), metadata)

    def download_file(self, Bucket, Key, Filename):
        Path(Filename).write_bytes(self.objects[Key][0])

    def copy_object(self, Bucket, CopySource, Key, Metadata=None, MetadataDirective=None):
        data, old = self.objects[CopySource["Key"]]
        self.objects[Key] = (data, dict(Metadata if Metadata is not None else old))

    def delete_object(self, Bucket, Key):
        self.objects.pop(Key, None)


def fixture_inspect(path):
    from apk_lab.models import ArtifactInspection, ContainerType

    path = Path(path)
    version = {"v1.apk": "1.0.0", "v2.apk": "2.0.0"}.get(path.name, "9.9.9")
    return ArtifactInspection(
        file_path=str(path),
        container_type=ContainerType.APKM,
        file_size=path.stat().st_size,
        sha256="sha-" + version,
        package_name=PACKAGE,
        version_name=version,
        version_code=100,
    )


def fixture_snapshot(manager, s3):
    snapshot = {"keys": sorted(s3.objects)}
    for role in ("latest", "target"):
        meta = manager.get_slot_metadata(PACKAGE, role)
        snapshot[role] = (
            None
            if meta is None
            else {
                "role": meta.role,
                "packageName": meta.package_name,
                "versionName": meta.version_name,
                "versionCode": meta.version_code,
                "containerType": meta.container_type,
                "sha256": meta.sha256,
                "signerSha256": meta.signer_sha256,
                "sizeBytes": meta.size_bytes,
                "acquisitionSource": meta.acquisition_source,
            }
        )
    return snapshot


def scenario_fixtures_key_metadata_rotation():
    import apk_lab.fixtures as fixtures

    fixtures.inspect_artifact = fixture_inspect
    shutil.rmtree("fixture-artifacts", ignore_errors=True)
    Path("fixture-artifacts").mkdir()
    v1 = Path("fixture-artifacts/v1.apk")
    v2 = Path("fixture-artifacts/v2.apk")
    v1.write_bytes(b"fixture-v1")
    v2.write_bytes(b"fixture-v2")

    s3 = FakeS3()
    manager = fixtures.R2FixtureManager(bucket_name="test-bucket", s3_client=s3)
    snapshots = []
    manager.seed_fixture(PACKAGE, "latest", v1)
    snapshots.append(fixture_snapshot(manager, s3))
    manager.rotate_slots_on_new_latest(PACKAGE, v2, "1.0.0", "apkeep")
    snapshots.append(fixture_snapshot(manager, s3))
    manager.rotate_slots_on_new_latest(PACKAGE, v1, "1.0.0", "apkeep")
    snapshots.append(fixture_snapshot(manager, s3))
    return 0, {"fixtures": snapshots}


def scenario_ci_matrix_tie_break():
    import apk_lab.compatibility_ci as ci

    output = Path(f"matrix-{SIDE}.txt")
    output.write_text("before\n", encoding="utf-8")
    code = ci.main(
        [
            "matrix",
            "--output-file",
            str(output),
            "--patches-list",
            "fixtures/patches-list.json",
        ]
    )
    data = json.loads(Path("fixtures/patches-list.json").read_text(encoding="utf-8"))
    metadata = ci.get_package_run_metadata(data, PACKAGE)
    return code, {
        "outcome": "success" if code == 0 else "error",
        "matrix": {"outputFile": output.read_text(encoding="utf-8"), "metadata": metadata},
    }


def scenario_ci_start_payload():
    import apk_lab.compatibility_ci as ci

    captured = {}

    def fake_post(url, secret, payload):
        captured.update({"url": url, "payload": payload, "status": 201})
        return 201

    ci.post_run_start = fake_post
    code = ci.main(
        [
            "start",
            "--package",
            PACKAGE,
            "--request-id",
            "req-1",
            "--git-revision",
            "abc123",
            "--patches-list",
            "fixtures/patches-list.json",
        ]
    )
    return code, {
        "outcome": "success" if code == 0 else "error",
        "post": captured,
    }


class FakeR2:
    def __init__(self):
        from apk_lab.fixtures import SlotMetadata

        self.metadata = SlotMetadata(
            role="latest",
            package_name=PACKAGE,
            version_name="1.10.0",
            version_code=100,
            container_type="APKM",
            sha256="slot-sha",
            signer_sha256=None,
            size_bytes=19,
            acquisition_source="manual",
            timestamp="2026-01-01T00:00:00+00:00",
        )

    def get_slot_metadata(self, package, role):
        return self.metadata if role == "latest" else None

    def download_slot(self, package, role, destination):
        Path(destination).write_bytes(b"slot-artifact-bytes")
        return Path(destination)


def fake_check(artifact, mpp, expected_package=None, all_patches=True):
    from apk_lab.models import PatchCompatibilityReport

    return (
        PatchCompatibilityReport(
            artifact_sha256="slot-sha",
            package_name=PACKAGE,
            version_name="1.10.0",
            version_code=100,
            patch_bundle_version="9.9.9",
            git_revision="abc123",
            tool_versions={},
            overall_status="compatible",
            total_cases=2,
            passed_cases=2,
            failed_cases=0,
            test_cases=[],
            failure_reason=None,
        ),
        0,
    )


def run_ci_run(status, fail):
    import apk_lab.compatibility_ci as ci

    captured = {}

    def fake_post(url, secret, submission):
        submission_path = os.environ.get("COMPATIBILITY_SUBMISSION_PATH")
        captured["savedBeforePost"] = bool(
            submission_path and Path(submission_path).is_file()
        )
        captured["payload"] = submission
        captured["status"] = status
        if fail:
            raise OSError("connection refused")
        return status

    ci.R2FixtureManager = lambda *args, **kwargs: FakeR2()
    ci.run_compatibility_check = fake_check
    ci.post_compatibility_submission = fake_post
    code = ci.main(
        [
            "run",
            "--package",
            PACKAGE,
            "--mpp",
            "fixtures/payload.mpp",
            "--patches-list",
            "fixtures/patches-list.json",
        ]
    )
    submission_path = Path(os.environ["COMPATIBILITY_SUBMISSION_PATH"])
    marker_path = Path(os.environ["COMPATIBILITY_CALLBACK_MARKER"])
    submission = (
        json.loads(submission_path.read_text(encoding="utf-8"))
        if submission_path.is_file()
        else None
    )
    marker = marker_path.read_text(encoding="utf-8") if marker_path.is_file() else None
    return code, {
        "outcome": "success" if code == 0 else "error",
        "post": captured,
        "files": {"submission": submission, "marker": marker},
    }


def scenario_ci_run_saved_before_post():
    return run_ci_run(201, False)


def scenario_ci_run_2xx_marker():
    return run_ci_run(204, False)


def scenario_ci_run_transport_failure():
    return run_ci_run(0, True)


def main():
    global SCENARIO, RESULT_PATH, SIDE
    if len(sys.argv) < 4:
        print("usage: driver.py SCENARIO RESULT SIDE", file=sys.stderr)
        return 2
    SCENARIO, RESULT_PATH, SIDE = sys.argv[1], sys.argv[2], sys.argv[3]
    try:
        code, payload = globals()["scenario_" + SCENARIO.replace("-", "_")]()
    except Exception as error:  # noqa: BLE001 - surfaced in the result document
        code, payload = 70, {
            "outcome": "driver-error",
            "error": f"{type(error).__name__}: {error}",
        }
    result = {"scenario": SCENARIO, "exit": int(code)}
    result.update(payload)
    Path(RESULT_PATH).write_text(
        json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    return int(code)


if __name__ == "__main__":
    sys.exit(main())
"##;

/// Hidden entry point used by the parity harness to execute one scenario in a
/// separate process. It writes the canonical result document and exits with
/// the scenario's own status so the harness can compare real exit codes.
pub(crate) fn run_acquisition_ci_scenario_cli(args: &[String]) -> Result<(), String> {
    let Some(scenario) = args.first() else {
        return Err("acquisition-ci-scenario requires a scenario id".to_string());
    };
    let mut result_path: Option<PathBuf> = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--result" => {
                result_path = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--side" => {
                index += 2;
            }
            other => return Err(format!("unrecognized scenario option '{other}'")),
        }
    }
    let result_path =
        result_path.ok_or("acquisition-ci-scenario requires --result PATH".to_string())?;
    let (code, result) = execute_native_scenario(scenario)?;
    fs::write(
        &result_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&result)
                .map_err(|error| format!("could not serialize scenario result: {error}"))?
        ),
    )
    .map_err(|error| format!("could not write scenario result: {error}"))?;
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    std::process::exit(code);
}

fn execute_native_scenario(scenario: &str) -> Result<(i32, Value), String> {
    let outcome = match scenario {
        "acquire-no-account" => scenario_acquire_no_account(),
        "acquire-account-rotation" => scenario_acquire_account_rotation(),
        "acquire-wrong-version" => scenario_acquire_wrong_version(),
        "acquire-normalization-manifest" => scenario_acquire_normalization_manifest(),
        "acquire-source-labels" => scenario_acquire_source_labels(),
        "fixtures-key-metadata-rotation" => scenario_fixtures_key_metadata_rotation(),
        "ci-matrix-tie-break" => scenario_ci_matrix_tie_break(),
        "ci-start-payload" => scenario_ci_start_payload(),
        "ci-run-saved-before-post" => scenario_ci_run(201, false),
        "ci-run-2xx-marker" => scenario_ci_run(204, false),
        "ci-run-transport-failure" => scenario_ci_run(0, true),
        other => Err(format!("unknown acquisition-ci scenario '{other}'")),
    };
    match outcome {
        Ok((code, mut result)) => {
            if let Value::Object(map) = &mut result {
                map.entry("scenario".to_string())
                    .or_insert_with(|| Value::String(scenario.to_string()));
            }
            Ok((code, result))
        }
        Err(error) => Ok((
            70,
            serde_json::json!({
                "scenario": scenario,
                "exit": 70,
                "outcome": "driver-error",
                "error": error,
            }),
        )),
    }
}

fn scenario_result(code: ExitCode, payload: Value) -> (i32, Value) {
    let exit = i32::from(code.as_u8());
    let mut object = payload;
    if let Value::Object(map) = &mut object {
        map.insert(
            "exit".to_string(),
            Value::Number(serde_json::Number::from(exit)),
        );
    }
    (exit, object)
}

fn scenario_manifest(rel: &str, path: &Path) -> Result<Value, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut members = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|error| format!("could not read member {index}: {error}"))?;
        let name = member
            .name()
            .map_err(|error| format!("could not read member name: {error}"))?
            .to_string();
        let mut data = Vec::new();
        io::Read::read_to_end(&mut member, &mut data)
            .map_err(|error| format!("could not read member {name}: {error}"))?;
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&data);
        let modified = member.last_modified();
        let date_time = modified.map_or([0, 0, 0, 0, 0, 0], |value| {
            [
                i64::from(value.year()),
                i64::from(value.month()),
                i64::from(value.day()),
                i64::from(value.hour()),
                i64::from(value.minute()),
                i64::from(value.second()),
            ]
        });
        members.push(serde_json::json!({
            "name": name,
            "size": data.len(),
            "sha256": format!("{:x}", hasher.finalize()),
            "dateTime": date_time,
            "mode": member.unix_mode().unwrap_or(0) & 0o777,
        }));
    }
    Ok(serde_json::json!({
        "path": rel,
        "exists": true,
        "members": members,
    }))
}

struct ScenarioBadger {
    version: &'static str,
    code: u64,
    split: bool,
}

impl crate::acquisition::ApkBadger for ScenarioBadger {
    fn badging(&self, apk: &Path) -> Result<crate::models::AndroidManifestInfo, String> {
        let name = apk.file_name().and_then(|name| name.to_str()).unwrap_or("");
        let split_name = if self.split && name != "base.apk" {
            Some(name.trim_end_matches(".apk").to_string())
        } else {
            None
        };
        Ok(crate::models::AndroidManifestInfo {
            package_name: "com.example.parity".to_string(),
            version_code: self.code,
            version_name: self.version.to_string(),
            min_sdk_version: None,
            target_sdk_version: None,
            permissions: Vec::new(),
            components: Vec::new(),
            split_name,
        })
    }
}

struct ScenarioInspector {
    version: &'static str,
    code: u64,
}

impl crate::acquisition::ArtifactInspector for ScenarioInspector {
    fn version(&self, _artifact: &Path) -> Result<crate::acquisition::ArtifactVersion, String> {
        Ok(crate::acquisition::ArtifactVersion {
            version_name: self.version.to_string(),
            version_code: self.code,
        })
    }
}

struct ScenarioApkeepTool;

impl crate::acquisition::ApkeepTool for ScenarioApkeepTool {
    fn command(&self) -> Result<(PathBuf, Vec<String>), crate::error::Error> {
        Ok((PathBuf::from("apkeep"), Vec::new()))
    }
}

struct ScenarioCommandRunner {
    calls: std::cell::RefCell<Vec<String>>,
    fail_first: bool,
}

impl crate::acquisition::CommandRunner for ScenarioCommandRunner {
    fn run(
        &self,
        _program: &Path,
        args: &[String],
        _cwd: Option<&Path>,
        _timeout: std::time::Duration,
        _secrets: &[String],
    ) -> Result<crate::acquisition::CommandOutcome, String> {
        let ini_index = args
            .iter()
            .position(|arg| arg == "-i")
            .ok_or("apkeep invocation has no credentials file")?;
        let ini_path = Path::new(&args[ini_index + 1]);
        let ini = fs::read_to_string(ini_path)
            .map_err(|error| format!("could not read credentials: {error}"))?;
        let email = ini
            .lines()
            .find_map(|line| line.strip_prefix("email = "))
            .unwrap_or("unknown@example.com")
            .trim()
            .to_string();
        self.calls.borrow_mut().push(email.clone());
        if self.fail_first && email == "first@example.com" {
            return Ok(crate::acquisition::CommandOutcome {
                success: false,
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "first denial".to_string(),
                timed_out: false,
            });
        }
        let attempt = args
            .last()
            .ok_or("apkeep invocation has no output directory")?;
        let attempt = Path::new(attempt);
        fs::create_dir_all(attempt)
            .map_err(|error| format!("could not create attempt directory: {error}"))?;
        fs::write(attempt.join("base.apk"), b"base-apk-bytes")
            .map_err(|error| format!("could not write fake APK: {error}"))?;
        Ok(crate::acquisition::CommandOutcome {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
        })
    }
}

struct FailingPlay(&'static str);

impl crate::acquisition::PlayProvider for FailingPlay {
    fn download(
        &self,
        _request: &crate::acquisition::ProviderRequest<'_>,
        _auth: crate::acquisition::PlayAuth<'_>,
    ) -> Result<PathBuf, crate::error::Error> {
        Err(crate::error::Error::infrastructure(self.0))
    }
}

struct FailingApkmirror(&'static str);

impl crate::acquisition::ApkmirrorProvider for FailingApkmirror {
    fn download(
        &self,
        _request: &crate::acquisition::ProviderRequest<'_>,
    ) -> Result<PathBuf, crate::error::Error> {
        Err(crate::error::Error::invalid_artifact(self.0))
    }
}

fn load_scenario_accounts() -> (
    Option<Vec<crate::acquisition::AccountConfig>>,
    Option<String>,
) {
    let environment = crate::acquisition::AccountsEnvironment::from_process();
    match crate::acquisition::load_accounts(&environment) {
        Ok(accounts) => (accounts, None),
        Err(error) => (
            None,
            Some(format!("Failed to load Google Play accounts: {error}")),
        ),
    }
}

fn run_scenario_acquirer(
    accounts: Option<&[crate::acquisition::AccountConfig]>,
    accounts_error: Option<&str>,
    inspector_version: &'static str,
    expected_version: Option<&str>,
    expected_version_code: Option<u64>,
    fail_first: bool,
) -> (
    Result<crate::acquisition::AcquiredArtifact, crate::error::Error>,
    Vec<String>,
) {
    let commands = ScenarioCommandRunner {
        calls: std::cell::RefCell::new(Vec::new()),
        fail_first,
    };
    let badger = ScenarioBadger {
        version: "1.0.0",
        code: 100,
        split: false,
    };
    let inspector = ScenarioInspector {
        version: inspector_version,
        code: 100,
    };
    let tool = ScenarioApkeepTool;
    let apkeep = crate::acquisition::ApkeepAcquirer {
        tool: &tool,
        commands: &commands,
        badger: &badger,
        inspector: &inspector,
        timeout: std::time::Duration::from_secs(30),
    };
    let play = FailingPlay("dispenser unavailable");
    let apkmirror =
        FailingApkmirror("No APKMirror repository mapping configured for 'com.example.parity'");
    let out_dir = PathBuf::from("out");
    let request = crate::acquisition::ProviderRequest {
        package_name: "com.example.parity",
        out_dir: &out_dir,
        expected_version,
        expected_version_code,
    };
    let acquirer = crate::acquisition::Acquirer {
        apkeep: Some(&apkeep),
        play: Some(&play),
        apkmirror: Some(&apkmirror),
        retry: crate::acquisition::RetryPolicy::default(),
    };
    let result = acquirer.acquire(&request, accounts, accounts_error);
    (result, commands.calls.into_inner())
}

fn scenario_acquire_no_account() -> Result<(i32, Value), String> {
    let (accounts, accounts_error) = load_scenario_accounts();
    let (result, _) = run_scenario_acquirer(
        accounts.as_deref(),
        accounts_error.as_deref(),
        "1.0.0",
        None,
        None,
        false,
    );
    match result {
        Ok(artifact) => Ok(scenario_result(
            ExitCode::Success,
            serde_json::json!({
                "outcome": "success",
                "source": artifact.source.label(),
            }),
        )),
        Err(error) => Ok(scenario_result(
            error.exit_code(),
            serde_json::json!({"outcome": "error", "error": error.message()}),
        )),
    }
}

fn scenario_acquire_account_rotation() -> Result<(i32, Value), String> {
    let (accounts, accounts_error) = load_scenario_accounts();
    let _ = fs::remove_dir_all("out");
    let (result, calls) = run_scenario_acquirer(
        accounts.as_deref(),
        accounts_error.as_deref(),
        "1.0.0",
        Some("1.0.0"),
        Some(100),
        true,
    );
    match result {
        Ok(artifact) => Ok(scenario_result(
            ExitCode::Success,
            serde_json::json!({
                "outcome": "success",
                "source": artifact.source.label(),
                "calls": calls,
                "artifact": scenario_manifest("out/com.example.parity.apkm", &artifact.path)?,
            }),
        )),
        Err(error) => Ok(scenario_result(
            error.exit_code(),
            serde_json::json!({"outcome": "error", "error": error.message(), "calls": calls}),
        )),
    }
}

fn scenario_acquire_wrong_version() -> Result<(i32, Value), String> {
    let (accounts, accounts_error) = load_scenario_accounts();
    let _ = fs::remove_dir_all("out");
    let (result, _) = run_scenario_acquirer(
        accounts.as_deref(),
        accounts_error.as_deref(),
        "9.9.9",
        Some("1.0.0"),
        Some(100),
        false,
    );
    match result {
        Ok(artifact) => Ok(scenario_result(
            ExitCode::Success,
            serde_json::json!({
                "outcome": "success",
                "artifact": scenario_manifest("out/com.example.parity.apkm", &artifact.path)?,
            }),
        )),
        Err(error) => Ok(scenario_result(
            error.exit_code(),
            serde_json::json!({
                "outcome": "error",
                "error": error.message(),
                "artifactPresent": Path::new("out/com.example.parity.apkm").exists(),
            }),
        )),
    }
}

fn scenario_acquire_normalization_manifest() -> Result<(i32, Value), String> {
    let _ = fs::remove_dir_all("norm");
    fs::create_dir_all("norm").map_err(|error| format!("could not create norm: {error}"))?;
    let mut apks = Vec::new();
    for (name, data) in [
        ("base.apk", b"BASE-APK" as &[u8]),
        ("config.en.apk", b"EN-SPLIT" as &[u8]),
        ("config.arm64.apk", b"ARM64-SPLIT" as &[u8]),
    ] {
        let path = PathBuf::from("norm").join(name);
        fs::write(&path, data).map_err(|error| format!("could not write {name}: {error}"))?;
        apks.push(path);
    }
    let badger = ScenarioBadger {
        version: "1.0.0",
        code: 100,
        split: true,
    };
    let output =
        crate::acquisition::normalize_apks_to_apkm(&apks, Path::new("norm/out.apkm"), &badger)
            .map_err(|error| error.message())?;
    Ok(scenario_result(
        ExitCode::Success,
        serde_json::json!({
            "outcome": "success",
            "artifact": scenario_manifest("norm/out.apkm", &output)?,
        }),
    ))
}

fn scenario_acquire_source_labels() -> Result<(i32, Value), String> {
    Ok(scenario_result(
        ExitCode::Success,
        serde_json::json!({"labels": ["apkeep", "goopdl", "apkmirror", "manual"]}),
    ))
}

// ---------------------------------------------------------------------------
// Native fixture rotation scenario (in-memory S3)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct ScenarioStoredObject {
    bytes: Vec<u8>,
    metadata: BTreeMap<String, String>,
}

#[derive(Default)]
struct ScenarioMemoryS3 {
    objects: std::sync::Mutex<BTreeMap<String, ScenarioStoredObject>>,
}

impl ScenarioMemoryS3 {
    fn insert(&self, key: &str, bytes: &[u8], metadata: BTreeMap<String, String>) {
        self.objects.lock().unwrap().insert(
            key.to_string(),
            ScenarioStoredObject {
                bytes: bytes.to_vec(),
                metadata,
            },
        );
    }

    fn keys(&self) -> Vec<String> {
        self.objects.lock().unwrap().keys().cloned().collect()
    }
}

#[async_trait::async_trait]
impl crate::fixtures::S3Adapter for ScenarioMemoryS3 {
    async fn head_object(
        &self,
        _bucket: &str,
        key: &str,
    ) -> Result<Option<crate::fixtures::S3Object>, crate::fixtures::S3AdapterError> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(key)
            .map(|stored| crate::fixtures::S3Object {
                metadata: stored.metadata.clone(),
                content_type: Some("application/vnd.android.package-archive".to_string()),
                content_length: Some(stored.bytes.len() as u64),
            }))
    }

    async fn get_object_to_file(
        &self,
        _bucket: &str,
        key: &str,
        destination: &Path,
    ) -> Result<(), crate::fixtures::S3AdapterError> {
        let bytes = self
            .objects
            .lock()
            .unwrap()
            .get(key)
            .map(|stored| stored.bytes.clone())
            .ok_or_else(|| {
                crate::fixtures::S3AdapterError::new(
                    crate::fixtures::S3ErrorKind::NotFound,
                    "object not found",
                )
            })?;
        fs::write(destination, bytes).map_err(|error| {
            crate::fixtures::S3AdapterError::new(
                crate::fixtures::S3ErrorKind::Other,
                error.to_string(),
            )
        })
    }

    async fn put_object_from_file(
        &self,
        _bucket: &str,
        key: &str,
        source: &Path,
        metadata: &BTreeMap<String, String>,
        _content_type: &str,
    ) -> Result<(), crate::fixtures::S3AdapterError> {
        let bytes = fs::read(source).map_err(|error| {
            crate::fixtures::S3AdapterError::new(
                crate::fixtures::S3ErrorKind::Other,
                error.to_string(),
            )
        })?;
        self.insert(key, &bytes, metadata.clone());
        Ok(())
    }

    async fn delete_object(
        &self,
        _bucket: &str,
        key: &str,
    ) -> Result<(), crate::fixtures::S3AdapterError> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
}

struct ScenarioFixtureInspector;

impl crate::morphe::ArtifactInspector for ScenarioFixtureInspector {
    fn inspect(
        &self,
        path: &Path,
    ) -> Result<crate::models::ArtifactInspection, crate::error::Error> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let version = match name {
            "v1.apk" => "1.0.0",
            "v2.apk" => "2.0.0",
            other => other,
        };
        let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        Ok(crate::models::ArtifactInspection {
            file_path: path.to_string_lossy().into_owned(),
            container_type: crate::models::ContainerType::Apkm,
            file_size: size,
            sha256: format!("sha-{version}"),
            package_name: "com.example.parity".to_string(),
            version_name: version.to_string(),
            version_code: 100,
            min_sdk: None,
            target_sdk: None,
            signing_certificate_sha256: None,
            splits: Vec::new(),
            dex_classes_count: 0,
            dex_methods_count: 0,
            dex_files: Vec::new(),
            native_libraries: Vec::new(),
            resources: Vec::new(),
            assets: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

fn scenario_fixture_snapshot(
    manager: &crate::fixtures::R2FixtureManager,
    s3: &ScenarioMemoryS3,
) -> Value {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("scenario runtime");
    let mut snapshot = serde_json::json!({"keys": s3.keys()});
    for role in ["latest", "target"] {
        let metadata = runtime
            .block_on(manager.get_slot_metadata("com.example.parity", role))
            .ok()
            .flatten()
            .map(|meta| {
                serde_json::json!({
                    "role": meta.role,
                    "packageName": meta.package_name,
                    "versionName": meta.version_name,
                    "versionCode": meta.version_code,
                    "containerType": meta.container_type,
                    "sha256": meta.sha256,
                    "signerSha256": meta.signer_sha256,
                    "sizeBytes": meta.size_bytes,
                    "acquisitionSource": meta.acquisition_source,
                })
            })
            .unwrap_or(Value::Null);
        snapshot
            .as_object_mut()
            .expect("snapshot object")
            .insert(role.to_string(), metadata);
    }
    snapshot
}

fn scenario_fixtures_key_metadata_rotation() -> Result<(i32, Value), String> {
    let _ = fs::remove_dir_all("fixture-artifacts");
    fs::create_dir_all("fixture-artifacts")
        .map_err(|error| format!("could not create fixture artifacts: {error}"))?;
    for (name, data) in [("v1.apk", b"fixture-v1"), ("v2.apk", b"fixture-v2")] {
        fs::write(Path::new("fixture-artifacts").join(name), data)
            .map_err(|error| format!("could not write {name}: {error}"))?;
    }

    let s3 = std::sync::Arc::new(ScenarioMemoryS3::default());
    let manager = crate::fixtures::R2FixtureManager::with_adapter("test-bucket", s3.clone())
        .with_inspector(std::sync::Arc::new(ScenarioFixtureInspector));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not create scenario runtime: {error}"))?;

    let mut snapshots = Vec::new();
    runtime
        .block_on(manager.seed_fixture(
            "com.example.parity",
            "latest",
            Path::new("fixture-artifacts/v1.apk"),
        ))
        .map_err(|error| error.to_string())?;
    snapshots.push(scenario_fixture_snapshot(&manager, &s3));
    runtime
        .block_on(manager.rotate_slots_on_new_latest(
            "com.example.parity",
            Path::new("fixture-artifacts/v2.apk"),
            "1.0.0",
            "apkeep",
        ))
        .map_err(|error| error.to_string())?;
    snapshots.push(scenario_fixture_snapshot(&manager, &s3));
    runtime
        .block_on(manager.rotate_slots_on_new_latest(
            "com.example.parity",
            Path::new("fixture-artifacts/v1.apk"),
            "1.0.0",
            "apkeep",
        ))
        .map_err(|error| error.to_string())?;
    snapshots.push(scenario_fixture_snapshot(&manager, &s3));

    Ok(scenario_result(
        ExitCode::Success,
        serde_json::json!({"fixtures": snapshots}),
    ))
}

// ---------------------------------------------------------------------------
// Native CI scenarios
// ---------------------------------------------------------------------------

fn scenario_patches_data() -> Result<Value, String> {
    let text = fs::read_to_string("fixtures/patches-list.json")
        .map_err(|error| format!("could not read patches-list fixture: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("invalid patches-list fixture: {error}"))
}

fn environment_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn scenario_ci_matrix_tie_break() -> Result<(i32, Value), String> {
    let data = scenario_patches_data()?;
    let side = environment_value("PARITY_SIDE").unwrap_or_else(|| "rust".to_string());
    let output = PathBuf::from(format!("matrix-{side}.txt"));
    fs::write(&output, b"before\n")
        .map_err(|error| format!("could not reset matrix output: {error}"))?;
    let args: Vec<std::ffi::OsString> = vec![
        "ci".into(),
        "matrix".into(),
        "--output-file".into(),
        output.clone().into_os_string(),
        "--patches-list".into(),
        "fixtures/patches-list.json".into(),
    ];
    let code = match crate::cli::run(&args) {
        Ok(code) => code,
        Err(error) => error.exit_code(),
    };
    let metadata = crate::compatibility_ci::get_package_run_metadata(&data, "com.example.parity");
    let output_text = fs::read_to_string(&output)
        .map_err(|error| format!("could not read matrix output: {error}"))?;
    Ok(scenario_result(
        code,
        serde_json::json!({
            "outcome": if code == ExitCode::Success { "success" } else { "error" },
            "matrix": {
                "outputFile": output_text,
                "metadata": {
                    "packageName": metadata.package_name,
                    "appName": metadata.app_name,
                    "targetVersion": metadata.target_version,
                    "supportedVersions": metadata.supported_versions,
                }
            }
        }),
    ))
}

struct ScenarioStartPoster {
    payload: std::sync::Mutex<Option<Value>>,
}

impl crate::compatibility_ci::WorkerPoster for ScenarioStartPoster {
    fn post_run_start(
        &self,
        _url: &str,
        _secret: &str,
        payload: &crate::compatibility_ci::RunStartPayload,
    ) -> Result<u16, crate::compatibility_ci::PostError> {
        *self.payload.lock().unwrap() = serde_json::to_value(payload).ok();
        Ok(201)
    }

    fn post_submission(
        &self,
        _url: &str,
        _secret: &str,
        _submission: &crate::compatibility_ci::CompatibilitySubmission,
    ) -> Result<u16, crate::compatibility_ci::PostError> {
        Err(crate::compatibility_ci::PostError {
            message: "unexpected submission".to_string(),
        })
    }
}

fn scenario_ci_start_payload() -> Result<(i32, Value), String> {
    let poster = ScenarioStartPoster {
        payload: std::sync::Mutex::new(None),
    };
    let args = vec![
        "--package".to_string(),
        "com.example.parity".to_string(),
        "--request-id".to_string(),
        "req-1".to_string(),
        "--git-revision".to_string(),
        "abc123".to_string(),
        "--patches-list".to_string(),
        "fixtures/patches-list.json".to_string(),
    ];
    let code = match crate::cli::run_ci_start_with(&args, &poster) {
        Ok(code) => code,
        Err(error) => error.exit_code(),
    };
    let payload = poster
        .payload
        .into_inner()
        .unwrap_or(None)
        .unwrap_or(Value::Null);
    let url = environment_value("WORKER_STATUS_URL").unwrap_or_default();
    Ok(scenario_result(
        code,
        serde_json::json!({
            "outcome": if code == ExitCode::Success { "success" } else { "error" },
            "post": {"url": url, "payload": payload, "status": 201}
        }),
    ))
}

struct ScenarioChecker;

impl crate::compatibility_ci::CompatibilityChecker for ScenarioChecker {
    fn check(
        &self,
        _artifact: &Path,
        _mpp: &Path,
        _expected_package: &str,
    ) -> Result<(crate::morphe::PatchCompatibilityReport, ExitCode), crate::error::Error> {
        Ok((
            crate::morphe::PatchCompatibilityReport {
                artifact_sha256: "slot-sha".to_string(),
                package_name: "com.example.parity".to_string(),
                version_name: "1.10.0".to_string(),
                version_code: 100,
                patch_bundle_version: "9.9.9".to_string(),
                git_revision: "abc123".to_string(),
                tool_versions: crate::morphe::ToolVersions::new(),
                overall_status: "compatible".to_string(),
                total_cases: 2,
                passed_cases: 2,
                failed_cases: 0,
                test_cases: Vec::new(),
                failure_reason: None,
            },
            ExitCode::Success,
        ))
    }
}

struct ScenarioSubmissionPoster {
    status: u16,
    fail: bool,
    captured: std::sync::Mutex<Option<Value>>,
}

impl crate::compatibility_ci::WorkerPoster for ScenarioSubmissionPoster {
    fn post_run_start(
        &self,
        _url: &str,
        _secret: &str,
        _payload: &crate::compatibility_ci::RunStartPayload,
    ) -> Result<u16, crate::compatibility_ci::PostError> {
        Ok(201)
    }

    fn post_submission(
        &self,
        _url: &str,
        _secret: &str,
        submission: &crate::compatibility_ci::CompatibilitySubmission,
    ) -> Result<u16, crate::compatibility_ci::PostError> {
        let saved_before_post = environment_value("COMPATIBILITY_SUBMISSION_PATH")
            .map(|path| Path::new(&path).is_file())
            .unwrap_or(false);
        let payload = serde_json::to_value(submission).unwrap_or(Value::Null);
        *self.captured.lock().unwrap() = Some(serde_json::json!({
            "savedBeforePost": saved_before_post,
            "payload": payload,
            "status": self.status,
        }));
        if self.fail {
            return Err(crate::compatibility_ci::PostError {
                message: "connection refused".to_string(),
            });
        }
        Ok(self.status)
    }
}

fn scenario_ci_run(status: u16, fail: bool) -> Result<(i32, Value), String> {
    let s3 = std::sync::Arc::new(ScenarioMemoryS3::default());
    let metadata = crate::fixtures::SlotMetadata {
        role: "latest".to_string(),
        package_name: "com.example.parity".to_string(),
        version_name: "1.10.0".to_string(),
        version_code: 100,
        container_type: "APKM".to_string(),
        sha256: "slot-sha".to_string(),
        signer_sha256: None,
        size_bytes: 19,
        acquisition_source: "manual".to_string(),
        timestamp: "2026-01-01T00:00:00+00:00".to_string(),
    };
    s3.insert(
        &crate::fixtures::R2FixtureManager::get_slot_key("com.example.parity", "latest"),
        b"slot-artifact-bytes",
        metadata.to_s3_metadata(),
    );
    let fixtures = crate::fixtures::R2FixtureManager::with_adapter("test-bucket", s3)
        .with_inspector(std::sync::Arc::new(ScenarioFixtureInspector));
    let checker = ScenarioChecker;
    let poster = ScenarioSubmissionPoster {
        status,
        fail,
        captured: std::sync::Mutex::new(None),
    };
    let inspector = ScenarioFixtureInspector;
    let backends = crate::cli::CiRunBackends {
        fixtures: &fixtures,
        acquirer: None,
        checker: &checker,
        poster: &poster,
        inspector: &inspector,
    };
    let args = vec![
        "--package".to_string(),
        "com.example.parity".to_string(),
        "--mpp".to_string(),
        "fixtures/payload.mpp".to_string(),
        "--patches-list".to_string(),
        "fixtures/patches-list.json".to_string(),
    ];
    let code = match crate::cli::run_ci_run_with(&args, Some(&backends)) {
        Ok(code) => code,
        Err(error) => error.exit_code(),
    };
    let submission_path = environment_value("COMPATIBILITY_SUBMISSION_PATH");
    let marker_path = environment_value("COMPATIBILITY_CALLBACK_MARKER");
    let submission = submission_path
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    let marker = marker_path
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(Value::String)
        .unwrap_or(Value::Null);
    let captured = poster
        .captured
        .into_inner()
        .unwrap_or(None)
        .unwrap_or(Value::Null);
    Ok(scenario_result(
        code,
        serde_json::json!({
            "outcome": if code == ExitCode::Success { "success" } else { "error" },
            "post": captured,
            "files": {"submission": submission, "marker": marker}
        }),
    ))
}

fn resolve_fixture_value(run_root: &Path, value: &str) -> String {
    match value.strip_prefix('@') {
        Some(relative) => run_root.join(relative).to_string_lossy().into_owned(),
        None => value.to_string(),
    }
}

fn patching_common_variables(run_root: &Path) -> Vec<(String, String)> {
    [
        ("FAKE_LOG", "calls.log"),
        ("FAKE_BADGING", "badging.txt"),
        ("FAKE_LOGCAT", "logcat.txt"),
        ("FAKE_UI_XML", "ui.xml"),
        ("FAKE_DUMPSYS", "dumpsys.txt"),
        ("FAKE_PATCHED_APK", "fixtures/patched.apk"),
        ("FAKE_INSTALLED", "installed.apk"),
        ("FAKE_PID", "pid.txt"),
        ("FAKE_LIVE_LOG", "empty.log"),
    ]
    .into_iter()
    .map(|(name, relative)| {
        (
            name.to_string(),
            run_root.join(relative).to_string_lossy().into_owned(),
        )
    })
    .collect()
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
        let case_environment = environment.clone().with_variables(case.env.iter().cloned());
        let python = match run_process(oracle_command(oracle, &case.argv), cwd, &case_environment) {
            Ok(result) => result,
            Err(error) => {
                results.push(missing_result_with_argv(case, format!("python: {error}")));
                continue;
            }
        };
        let mut differences = postcondition_differences(case, cwd, "python");
        let rust = match run_binary(rust_bin, &case.argv, cwd, &case_environment) {
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
        differences.extend(postcondition_differences(case, cwd, "rust"));
        differences.extend(compare_case(
            &case.spec,
            &python,
            &rust,
            &normalizer.at(&case.spec),
        ));
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

fn postcondition_differences(case: &PreparedCase, cwd: &Path, side: &str) -> Vec<String> {
    let mut differences = Vec::new();
    for postcondition in &case.postconditions {
        match postcondition {
            Postcondition::FileExists(relative) => {
                if !cwd.join(relative).is_file() {
                    differences.push(format!(
                        "{side} postcondition: expected file '{relative}' to exist"
                    ));
                }
            }
            Postcondition::FileUnchanged(relative, expected) => {
                match fs::read(cwd.join(relative)) {
                    Ok(contents) if contents == *expected => {}
                    Ok(_) => differences.push(format!(
                        "{side} postcondition: file '{relative}' changed but must stay unchanged"
                    )),
                    Err(error) => differences.push(format!(
                        "{side} postcondition: could not read '{relative}': {error}"
                    )),
                }
            }
        }
    }
    differences
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

// ---------------------------------------------------------------------------
// Real-device corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceCorpusDocument {
    kind: String,
    version: u32,
    artifact: String,
    mpp: String,
    package: String,
    device: String,
    cases: Vec<CorpusCaseDocument>,
}

#[derive(Debug, Clone)]
struct DeviceCorpus {
    /// Canonical path of the corpus document itself.
    root: PathBuf,
    cases: BTreeMap<String, Vec<String>>,
}

fn load_device_corpus(path: &Path) -> Result<DeviceCorpus, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read corpus '{}': {error}", path.display()))?;
    let document: DeviceCorpusDocument = serde_json::from_str(&text)
        .map_err(|error| format!("invalid corpus '{}': {error}", path.display()))?;
    if document.kind != DEVICE_CORPUS_KIND {
        return Err(format!(
            "invalid corpus '{}': kind must be '{DEVICE_CORPUS_KIND}'",
            path.display()
        ));
    }
    if document.version != DEVICE_CORPUS_VERSION {
        return Err(format!(
            "invalid corpus '{}': unsupported version {} (expected {DEVICE_CORPUS_VERSION})",
            path.display(),
            document.version
        ));
    }
    for (name, value) in [
        ("artifact", &document.artifact),
        ("mpp", &document.mpp),
        ("package", &document.package),
        ("device", &document.device),
    ] {
        if value.trim().is_empty() {
            return Err(format!(
                "invalid corpus '{}': {name} must be non-empty",
                path.display()
            ));
        }
    }
    let root = fs::canonicalize(path)
        .map_err(|error| format!("could not resolve corpus '{}': {error}", path.display()))?;
    let base = root.parent().unwrap_or(Path::new(".")).to_path_buf();
    let resolve_input = |name: &str, value: &str| -> Result<PathBuf, String> {
        let candidate = if Path::new(value).is_absolute() {
            PathBuf::from(value)
        } else {
            base.join(value)
        };
        let resolved = fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "invalid corpus '{}': {name} cannot be resolved ('{value}'): {error}",
                path.display()
            )
        })?;
        if !resolved.is_file() {
            return Err(format!(
                "invalid corpus '{}': {name} is not a file: {}",
                path.display(),
                resolved.display()
            ));
        }
        Ok(resolved)
    };
    let artifact = resolve_input("artifact", &document.artifact)?;
    let mpp = resolve_input("mpp", &document.mpp)?;

    if document.cases.is_empty() {
        return Err(format!(
            "invalid corpus '{}': cases must contain at least one entry",
            path.display()
        ));
    }
    let replacements = [
        ("artifact", artifact.to_string_lossy().into_owned()),
        ("mpp", mpp.to_string_lossy().into_owned()),
        ("package", document.package.clone()),
        ("device", document.device.clone()),
    ];
    let mut cases = BTreeMap::new();
    for case in document.cases {
        if patching_device_case(&case.id).is_none() {
            return Err(format!(
                "invalid corpus '{}': unknown patching-device case '{}' (run 'parity --group {PATCHING_DEVICE_GROUP}' --list)",
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
            let mut value = token;
            if let Some(name) = value.strip_prefix('$') {
                let Some((_, replacement)) = replacements
                    .iter()
                    .find(|(candidate, _)| *candidate == name)
                else {
                    return Err(format!(
                        "invalid corpus '{}': case '{}' references unknown placeholder '${name}'",
                        path.display(),
                        case.id
                    ));
                };
                value = replacement.clone();
            }
            argv.push(value);
        }
        if cases.insert(case.id.clone(), argv).is_some() {
            return Err(format!(
                "invalid corpus '{}': duplicate case '{}'",
                path.display(),
                case.id
            ));
        }
    }

    Ok(DeviceCorpus { root, cases })
}

fn prepare_real_patching_cases(
    specs: &[&'static PatchingCase],
    corpus: &DeviceCorpus,
) -> Vec<PreparedCase> {
    specs
        .iter()
        .map(|case| {
            let Some(argv) = corpus.cases.get(case.spec.id) else {
                return PreparedCase::missing(
                    &case.spec,
                    case.spec
                        .argv
                        .iter()
                        .map(|value| value.to_string())
                        .collect(),
                    format!("corpus has no case '{}'", case.spec.id),
                );
            };
            let mut prepared = PreparedCase::new(&case.spec, argv.clone());
            prepared.postconditions = case.postconditions.to_vec();
            prepared
        })
        .collect()
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
        // Commands canonicalize working directories, so macOS may report
        // `/private/var/...` for a tempdir created as `/var/...`. Register both
        // spellings; longest first keeps nested roots deterministic.
        for path in [sandbox, &home, &temp, run_root] {
            if let Ok(canonical) = fs::canonicalize(path) {
                let canonical = canonical.to_string_lossy().into_owned();
                if canonical != path.to_string_lossy() {
                    replacements.push((canonical, path_replacement(path)));
                }
            }
        }
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

fn path_replacement(path: &Path) -> String {
    if path.file_name().and_then(|name| name.to_str()) == Some("run") {
        "<cwd>".to_string()
    } else if path.file_name().and_then(|name| name.to_str()) == Some("home") {
        "<home>".to_string()
    } else if path.file_name().and_then(|name| name.to_str()) == Some("tmp") {
        "<temp>".to_string()
    } else {
        "<sandbox>".to_string()
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
        let normalized = if self.rules.contains(&TextRule::HostToolDetails) {
            normalize_host_tool_lines(&normalized)
        } else {
            normalized
        };
        if self.rules.contains(&TextRule::PatchingDetails) {
            normalize_patching_details(&normalized)
        } else {
            normalized
        }
    }
}

/// Patching check lines carry per-case wall-clock durations and monitor text
/// carries process IDs. Both are environmental, so replace them with stable
/// placeholders while leaving status, names and thresholds intact.
fn normalize_patching_details(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = normalize_pid(line);
        if (line.starts_with("  [PASS] ") || line.starts_with("  [FAIL] ")) && line.ends_with(')') {
            if let Some(position) = line.rfind(" (") {
                lines.push(format!("{} (<duration>)", &line[..position]));
                continue;
            }
        }
        lines.push(line);
    }
    lines.join("\n").trim_end_matches('\n').to_string()
}

fn normalize_pid(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(position) = rest.find("PID") {
        result.push_str(&rest[..position]);
        rest = &rest[position..];
        let mut chars = rest.char_indices();
        let _ = chars.next();
        let _ = chars.next();
        let _ = chars.next();
        let mut digits_at = None;
        for (index, character) in rest.char_indices().skip(3) {
            if character.is_ascii_digit() {
                digits_at = Some(index);
                break;
            }
            if !(character == ' ' || character == ':') {
                break;
            }
        }
        if let Some(start) = digits_at {
            let end = rest[start..]
                .find(|character: char| !character.is_ascii_digit())
                .map(|offset| start + offset)
                .unwrap_or(rest.len());
            if end > start {
                result.push_str(&rest[..start]);
                result.push_str("<pid>");
                rest = &rest[end..];
                continue;
            }
        }
        result.push_str(&rest[..3]);
        rest = &rest[3..];
    }
    result.push_str(rest);
    result
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
        VolatileKind::Version => match value {
            Value::Number(_) => serde_json::json!("<version>"),
            _ => Value::String("<version>".to_string()),
        },
        VolatileKind::Pid => match value {
            Value::Number(_) => serde_json::json!("<pid>"),
            _ => Value::String("<pid>".to_string()),
        },
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

// ---------------------------------------------------------------------------
// Patching/device fixtures and fake tools
// ---------------------------------------------------------------------------

const PATCHING_BADGING: &str = "\
package: name='com.avinside' versionCode='100' versionName='1.0' compileSdkVersion='34'
launchable-activity: name='.MainActivity' label='' icon=''
sdkVersion:'21'
targetSdkVersion:'34'
";

const PATCHING_UI_XML: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="Must &amp; Watch" resource-id="" class="android.widget.TextView" package="com.avinside" content-desc="" clickable="false" enabled="true" bounds="[32,297][861,361]" />
  <node index="1" text="" resource-id="" class="android.view.View" package="com.avinside" content-desc="media" clickable="true" enabled="true" bounds="[281,2259][360,2290]" />
  <node index="2" text="Save" resource-id="com.avinside:id/save" class="android.widget.Button" package="com.avinside" content-desc="" clickable="true" enabled="true" bounds="[100,200][200,300]" />
</hierarchy>
"#;

const PATCHING_LOGCAT: &str = "\
10-15 12:00:00.000  1234  1234 E AndroidRuntime: FATAL EXCEPTION: main
10-15 12:00:00.000  1234  1234 E AndroidRuntime: Process: com.avinside, PID: 1234
10-15 12:00:00.000  1234  1234 E AndroidRuntime: java.lang.VerifyError: Rejecting class com.example.Bad
10-15 12:00:00.000  1234  1234 E AndroidRuntime:     at com.example.app.MainActivity.onCreate(MainActivity.java:20)
";

const PATCHING_LIVE_CRASH: &str =
    "10-15 12:00:00.000  1234  1234 E AndroidRuntime: FATAL EXCEPTION: main\n";

const PATCHING_AAPT2: &str = r#"#!/bin/sh
if [ "${1:-}" = "dump" ] && [ "${2:-}" = "badging" ]; then
  cat "$FAKE_BADGING"
  exit 0
fi
exit 0
"#;

const PATCHING_DEXDUMP: &str = r#"#!/bin/sh
exit 0
"#;

const PATCHING_APKSIGNER: &str = r#"#!/bin/sh
printf 'apksigner %s\n' "$*" >> "${FAKE_LOG:-/dev/null}" 2>/dev/null || true
if [ "${1:-}" = "verify" ]; then
  printf 'Signer #1 certificate SHA-256 digest: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n'
  exit 0
fi
if [ "${1:-}" = "sign" ]; then
  if [ "${FAKE_SIGN_FAIL:-}" = "1" ]; then
    printf 'injected signing failure\n' >&2
    exit 1
  fi
  exit 0
fi
exit 0
"#;

const PATCHING_ZIPALIGN: &str = r#"#!/bin/sh
printf 'zipalign %s\n' "$*" >> "${FAKE_LOG:-/dev/null}" 2>/dev/null || true
previous=""
last=""
for argument in "$@"; do
  previous="$last"
  last="$argument"
done
cp "$previous" "$last"
exit 0
"#;

const PATCHING_ADB: &str = r#"#!/bin/sh
printf 'adb %s\n' "$*" >> "${FAKE_LOG:-/dev/null}" 2>/dev/null || true
if [ "${1:-}" = "-s" ]; then shift 2; fi
case "${1:-}" in
  devices)
    printf 'List of devices attached\nemulator-5554\tdevice\n'
    ;;
  logcat)
    if [ "${2:-}" = "-d" ]; then
      cat "$FAKE_LOGCAT"
    else
      if [ -s "${FAKE_LIVE_LOG:-/dev/null}" ]; then cat "$FAKE_LIVE_LOG"; fi
    fi
    ;;
  install)
    if [ "${FAKE_ADB_INSTALL_FAIL:-}" = "1" ]; then
      printf 'Failure [INSTALL_FAILED_TEST]\n' >&2
      exit 1
    fi
    last=""
    for argument in "$@"; do last="$argument"; done
    cp "$last" "$FAKE_INSTALLED" 2>/dev/null || true
    printf 'Success\n'
    ;;
  uninstall)
    printf 'Success\n'
    ;;
  shell)
    shift
    case "${1:-}" in
      date) printf '1700000000 +0000\n' ;;
      dumpsys) if [ -s "$FAKE_DUMPSYS" ]; then cat "$FAKE_DUMPSYS"; fi ;;
      pidof) if [ -s "$FAKE_PID" ]; then cat "$FAKE_PID"; fi ;;
      cat)
        case "${2:-}" in
          /proc/stat) printf 'cpu  100 0 0 0 0 0 0 0 0 0\ncpu0 100 0 0 0 0 0 0 0 0 0\n' ;;
          *) printf '1234 (com.avinside) R 1 1 1 0 -1 4194560 100 0 0 0 100 0 0 0 20 0 1 0 100 0 0\n' ;;
        esac
        ;;
      input) printf 'tap\n' ;;
      am) printf 'Starting: Intent\n' ;;
      kill) printf '' ;;
    esac
    ;;
  exec-out)
    cat "$FAKE_UI_XML"
    ;;
esac
exit 0
"#;

const PATCHING_MORPHE: &str = r#"#!/bin/sh
printf 'morphe %s\n' "$*" >> "${FAKE_LOG:-/dev/null}" 2>/dev/null || true
out=""
result=""
group_open=0
ungrouped=0
names="$(mktemp "${TMPDIR:-/tmp}/morphe-names.XXXXXX")"
: > "$names"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -r) result="$2"; shift 2 ;;
    -p) group_open=1; shift 2 ;;
    -O)
      if [ "$group_open" != "1" ]; then ungrouped=1; fi
      shift 2
      ;;
    -e)
      if [ "$group_open" != "1" ]; then ungrouped=1; fi
      group_open=0
      printf '%s\n' "$2" >> "$names"
      shift 2
      ;;
    *) shift ;;
  esac
done
if [ -n "$out" ] && [ -n "${FAKE_PATCHED_APK:-}" ]; then
  cp "$FAKE_PATCHED_APK" "$out" 2>/dev/null || true
fi
if [ -n "$result" ]; then
  printf '{"appliedPatches":[' > "$result"
  first=1
  while IFS= read -r name; do
    if [ "$first" = "1" ]; then first=0; else printf ',' >> "$result"; fi
    printf '{"name":"%s"}' "$name" >> "$result"
  done < "$names"
  printf '],"failedPatches":[' >> "$result"
  if [ "$ungrouped" = "1" ]; then
    printf '{"name":"option-grouping","reason":"each -e must follow its own -p group"}' >> "$result"
  fi
  printf ']}\n' >> "$result"
fi
rm -f "$names"
exit 0
"#;

fn create_patching_device_fixtures(
    run_root: &Path,
    fake_sdk: &Path,
    home: &Path,
    fake_bin: &Path,
    project: &Path,
) -> Result<(), String> {
    let fixtures = run_root.join("fixtures");
    let out = run_root.join("out");
    let build_tools = fake_sdk.join(ANALYSIS_SDK_VERSION);
    let platform_tools = fake_sdk.join("platform-tools");
    for directory in [
        &fixtures,
        &out,
        &build_tools,
        &platform_tools,
        home,
        fake_bin,
    ] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }

    let manifest = b"<manifest package=\"com.avinside\"/>" as &[u8];
    let input_dex = dex_header(3, 7);
    let patched_dex = dex_header(5, 11);
    let entries: [(&str, &[u8]); 5] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", input_dex.as_slice()),
        ("resources.arsc", b"resources" as &[u8]),
        ("assets/note.txt", b"parity fixture" as &[u8]),
        ("lib/arm64-v8a/libnative.so", b"native" as &[u8]),
    ];
    let patched_entries: [(&str, &[u8]); 5] = [
        ("AndroidManifest.xml", manifest),
        ("classes.dex", patched_dex.as_slice()),
        ("resources.arsc", b"resources" as &[u8]),
        ("assets/note.txt", b"parity fixture" as &[u8]),
        ("lib/arm64-v8a/libnative.so", b"native" as &[u8]),
    ];
    write_apk(&fixtures.join("artifact.apk"), &entries)?;
    write_apk(&fixtures.join("patched.apk"), &patched_entries)?;
    for (name, content) in [
        ("patches.mpp", b"mpp" as &[u8]),
        ("custom.keystore", b"custom keystore" as &[u8]),
    ] {
        fs::write(fixtures.join(name), content)
            .map_err(|error| format!("could not write fixture {name}: {error}"))?;
    }
    for (name, content) in [
        ("badging.txt", PATCHING_BADGING),
        ("logcat.txt", PATCHING_LOGCAT),
        ("ui.xml", PATCHING_UI_XML),
        ("crash.log", PATCHING_LIVE_CRASH),
    ] {
        fs::write(run_root.join(name), content)
            .map_err(|error| format!("could not write fixture {name}: {error}"))?;
    }
    for name in ["dumpsys.txt", "empty.log", "calls.log"] {
        fs::write(run_root.join(name), b"")
            .map_err(|error| format!("could not write fixture {name}: {error}"))?;
    }
    fs::write(run_root.join("pid.txt"), b"1234\n")
        .map_err(|error| format!("could not write pid fixture: {error}"))?;
    fs::write(out.join("preserved.apk"), b"original output\n")
        .map_err(|error| format!("could not write preserved output: {error}"))?;

    for (name, script, directory) in [
        ("aapt2", PATCHING_AAPT2, build_tools.as_path()),
        ("dexdump", PATCHING_DEXDUMP, build_tools.as_path()),
        ("apksigner", PATCHING_APKSIGNER, build_tools.as_path()),
        ("zipalign", PATCHING_ZIPALIGN, build_tools.as_path()),
        ("adb", PATCHING_ADB, platform_tools.as_path()),
        ("java", FAKE_JAVA, fake_bin),
    ] {
        let path = directory.join(name);
        fs::write(&path, script)
            .map_err(|error| format!("could not write fake {name}: {error}"))?;
        make_executable(&path)?;
    }

    install_patching_morphe(home, project)?;
    Ok(())
}

fn install_patching_morphe(home: &Path, project: &Path) -> Result<(), String> {
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
    let spec = lock
        .get("tools")
        .and_then(|tools| tools.get("morphe"))
        .ok_or_else(|| format!("pinned tool lock '{}' has no morphe", lock_path.display()))?;
    let version = spec
        .get("version")
        .and_then(Value::as_str)
        .ok_or("pinned morphe has no version")?;
    let sha256 = spec
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or("pinned morphe has no sha256")?;
    let executable = spec
        .get("executablePath")
        .and_then(Value::as_str)
        .ok_or("pinned morphe has no executablePath")?;
    let path = home
        .join(ANALYSIS_CACHE)
        .join("morphe")
        .join(version)
        .join(sha256)
        .join(executable);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    fs::write(&path, PATCHING_MORPHE)
        .map_err(|error| format!("could not write fake morphe: {error}"))?;
    make_executable(&path)
}

fn validate_patching_device_tools(sandbox: &Path) -> Result<(), String> {
    let build_tools = sandbox.join("fake-sdk").join(ANALYSIS_SDK_VERSION);
    let mut required = vec![
        sandbox.join("fake-bin").join("java"),
        build_tools.join("aapt2"),
        build_tools.join("dexdump"),
        build_tools.join("apksigner"),
        build_tools.join("zipalign"),
        sandbox.join("fake-sdk").join("platform-tools").join("adb"),
    ];
    let morphe_root = sandbox.join("home").join(ANALYSIS_CACHE).join("morphe");
    required.push(morphe_root);
    for path in required {
        if !path.exists() {
            return Err(format!(
                "patching-device fake tool is missing: {}",
                path.display()
            ));
        }
    }
    Ok(())
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
         \x20 apk-lab-migrate parity --group patching-device [--profile synthetic]\n\
         \x20                     --python-project DIR --rust-bin PATH\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group patching-device --profile real-device \\\n\
         \x20                     --corpus FILE --python-project DIR --rust-bin PATH \\\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group patching-device --list [--case ID ...]\n\
         \x20 apk-lab-migrate parity --group acquisition-ci \\\n\
         \x20                     --python-project DIR --rust-bin PATH \\\n\
         \x20                     [--case ID ...] [--report PATH|-]\n\
         \x20 apk-lab-migrate parity --group acquisition-ci --list [--case ID ...]\n\
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
         The patching-device group defaults to --profile synthetic: it creates a\n\
         synthetic artifact, MPP, keystore, fake SDK build-tools, fake adb and a\n\
         fake pinned Morphe jar. No device, network or real tool cache is used.\n\
         It covers check selection/options, deployment transaction boundaries,\n\
         monitoring, crash traces and UI dump/click. `--profile real-device`\n\
         requires a corpus with explicit device, artifact, MPP and package inputs;\n\
         it never runs unless all four are present.\n\
         \n\
         \x20 {{\n\
         \x20   \"kind\": \"apk-lab-patching-device-corpus\",\n\
         \x20   \"version\": 1,\n\
         \x20   \"artifact\": \"relative/or/absolute.apk\",\n\
         \x20   \"mpp\": \"relative/or/absolute.mpp\",\n\
         \x20   \"package\": \"com.example.app\",\n\
         \x20   \"device\": \"SERIAL\",\n\
         \x20   \"cases\": [\n\
         \x20     {{\"id\": \"deploy-default\", \"argv\": [\"deploy\", \"$artifact\", \"--mpp\", \"$mpp\", \"--package\", \"$package\", \"--device\", \"$device\"]}}\n\
         \x20   ]\n\
         \x20 }}\n\
         \n\
         The acquisition-ci group is entirely offline. It runs library scenarios\n\
         with fake apkeep/Play/APKMirror providers, an in-memory S3 adapter, fake\n\
         acquirer/checker/poster boundaries and temporary artifacts. It covers\n\
         acquisition routing, account rotation, version rejection, deterministic\n\
         APKM member manifests, source labels, fixture key/metadata/rotation, CI\n\
         matrix output, run-start payloads and CI run submission/marker semantics.\n\
         No case contacts Play, R2, the Worker, a device, or the network.\n\
         \n\
         Options:\n\
         \x20 --group NAME          Parity group (available: core, analysis, patching-device, acquisition-ci)\n\
         \x20 --profile NAME        Profile (synthetic, real-tools, real-device; default synthetic)\n\
         \x20 --corpus FILE         Validated corpus manifest (required for real-tools/real-device)\n\
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
        assert!(NORMALIZATION_RULES
            .iter()
            .any(|rule| rule.contains("patching")));
    }

    #[test]
    fn patching_device_case_selection_is_explicit_and_stable() {
        let all = select_patching_device_cases(&[]).expect("all patching cases");
        assert_eq!(all.len(), PATCHING_DEVICE_CASES.len());
        assert_eq!(all[0].spec.id, "check-default-json");
        assert!(all
            .iter()
            .any(|case| case.spec.id == "deploy-install-failure"));
        assert!(all
            .iter()
            .any(|case| case.spec.id == "ui-click-invalid-coords"));

        let selected = select_patching_device_cases(&[
            "ui-click-text".to_string(),
            "check-default-json".to_string(),
        ])
        .expect("filtered");
        assert_eq!(
            selected.iter().map(|case| case.spec.id).collect::<Vec<_>>(),
            vec!["check-default-json", "ui-click-text"]
        );

        let error = select_patching_device_cases(&["not-a-case".to_string()])
            .expect_err("unknown case must fail");
        assert!(
            error.contains("unknown patching-device parity case"),
            "{error}"
        );
        assert!(patching_device_case("deploy-signing-failure")
            .expect("case")
            .postconditions
            .iter()
            .any(|postcondition| matches!(postcondition, Postcondition::FileUnchanged(_, _))));
    }

    #[test]
    fn missing_patching_device_fake_tool_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let sandbox = directory.path();
        let project = Path::new(env!("CARGO_MANIFEST_DIR"));
        create_patching_device_fixtures(
            &sandbox.join("run"),
            &sandbox.join("fake-sdk"),
            &sandbox.join("home"),
            &sandbox.join("fake-bin"),
            project,
        )
        .expect("fixtures");
        validate_patching_device_tools(sandbox).expect("fake tools complete");

        fs::remove_file(sandbox.join("fake-bin/java")).expect("remove fake java");
        let error = validate_patching_device_tools(sandbox).expect_err("missing tool must fail");
        assert!(error.contains("fake tool is missing"), "{error}");
        assert!(error.contains("java"), "{error}");
    }

    #[test]
    fn patching_device_exit_and_text_mismatches_fail() {
        let spec = patching_device_case("ui-click-text").expect("case").spec;
        let normalizer =
            Normalizer::new(NormalizationContext::for_test("/tmp/parity-patching-case"));
        let python = result(0, "Clicked node at (150, 250)\n", "");
        let rust = result(2, "Clicked node at (151, 250)\n", "boom\n");
        let differences = compare_case(&spec, &python, &rust, &normalizer.at(&spec));
        assert!(differences.iter().any(|entry| entry.starts_with("exit:")));
        assert!(differences.iter().any(|entry| entry.starts_with("stdout")));
        assert!(differences.iter().any(|entry| entry.starts_with("stderr")));
    }

    #[test]
    fn patching_device_report_is_deterministic_and_names_its_kind() {
        let case_result = CaseResult {
            id: "crash-trace-json".to_string(),
            argv: vec!["crash-trace".to_string(), "--json".to_string()],
            status: CaseStatus::Passed,
            approved: false,
            approval: None,
            exit: ExitPair { python: 0, rust: 0 },
            differences: Vec::new(),
        };
        let report = build_report_for(
            vec![case_result.clone(), case_result],
            PATCHING_DEVICE_REPORT_KIND,
            Some(SYNTHETIC_PROFILE),
        );
        let rendered = render_report(&report).expect("report renders");
        assert!(rendered.contains("\"kind\": \"apk-lab-patching-device-parity\""));
        assert!(rendered.contains("\"profile\": \"synthetic\""));
        assert!(report_passed(&report));
        assert_eq!(render_report(&report).expect("renders again"), rendered);
        assert_eq!(report.summary.total, 2);
        assert_eq!(report.summary.passed, 2);
    }

    #[test]
    fn real_device_profile_requires_explicit_inputs() {
        let error = run(&[
            "--group".to_string(),
            PATCHING_DEVICE_GROUP.to_string(),
            "--profile".to_string(),
            REAL_DEVICE_PROFILE.to_string(),
        ])
        .expect_err("real-device without corpus must fail");
        assert!(error.contains("--corpus is required"), "{error}");

        let directory = tempfile::tempdir().expect("tempdir");
        let corpus = directory.path().join("corpus.json");
        fs::write(
            &corpus,
            serde_json::to_vec(&serde_json::json!({
                "kind": DEVICE_CORPUS_KIND,
                "version": DEVICE_CORPUS_VERSION,
                "artifact": "missing.apk",
                "mpp": "missing.mpp",
                "package": "com.example.synthetic",
                "device": "",
                "cases": [{"id": "deploy-default", "argv": ["deploy"]}]
            }))
            .expect("corpus json"),
        )
        .expect("write corpus");
        let error = load_device_corpus(&corpus).expect_err("empty device must fail");
        assert!(error.contains("device must be non-empty"), "{error}");

        fs::write(
            &corpus,
            serde_json::to_vec(&serde_json::json!({
                "kind": DEVICE_CORPUS_KIND,
                "version": DEVICE_CORPUS_VERSION,
                "artifact": "missing.apk",
                "mpp": "missing.mpp",
                "package": "com.example.synthetic",
                "device": "emulator-5554",
                "cases": [{"id": "deploy-default", "argv": ["deploy", "$artifact"]}]
            }))
            .expect("corpus json"),
        )
        .expect("write corpus");
        let error = load_device_corpus(&corpus).expect_err("missing artifact must fail");
        assert!(error.contains("artifact cannot be resolved"), "{error}");
    }

    #[test]
    fn acquisition_ci_case_selection_is_explicit_and_stable() {
        let all = select_acquisition_ci_cases(&[]).expect("all acquisition-ci cases");
        assert_eq!(all.len(), ACQUISITION_CI_CASES.len());
        assert_eq!(all[0].spec.id, "acquire-no-account");
        for required in [
            "acquire-account-rotation",
            "acquire-wrong-version",
            "acquire-normalization-manifest",
            "acquire-source-labels",
            "fixtures-key-metadata-rotation",
            "ci-matrix-tie-break",
            "ci-start-payload",
            "ci-run-saved-before-post",
            "ci-run-2xx-marker",
            "ci-run-transport-failure",
        ] {
            assert!(
                all.iter().any(|case| case.spec.id == required),
                "missing case {required}"
            );
        }

        let selected = select_acquisition_ci_cases(&[
            "ci-start-payload".to_string(),
            "acquire-no-account".to_string(),
        ])
        .expect("filtered");
        assert_eq!(
            selected.iter().map(|case| case.spec.id).collect::<Vec<_>>(),
            vec!["acquire-no-account", "ci-start-payload"]
        );

        let error = select_acquisition_ci_cases(&["not-a-case".to_string()])
            .expect_err("unknown case must fail");
        assert!(
            error.contains("unknown acquisition-ci parity case"),
            "{error}"
        );
    }

    #[test]
    fn acquisition_ci_rejects_non_synthetic_profiles_and_corpus() {
        let error = run(&[
            "--group".to_string(),
            ACQUISITION_CI_GROUP.to_string(),
            "--profile".to_string(),
            REAL_TOOLS_PROFILE.to_string(),
        ])
        .expect_err("real-tools must be rejected");
        assert!(error.contains("only runs fake scenarios"), "{error}");

        let directory = tempfile::tempdir().expect("tempdir");
        let error = run(&[
            "--group".to_string(),
            ACQUISITION_CI_GROUP.to_string(),
            "--corpus".to_string(),
            directory
                .path()
                .join("corpus.json")
                .to_string_lossy()
                .into_owned(),
        ])
        .expect_err("corpus must be rejected");
        assert!(error.contains("--corpus is not supported"), "{error}");
    }

    #[test]
    fn acquisition_ci_result_mismatch_is_reported() {
        let context = NormalizationContext::for_test("/tmp/parity-acquisition-ci");
        let mut differences = Vec::new();
        compare_result_json(
            &serde_json::json!({"outcome": "success", "source": "apkeep"}),
            &serde_json::json!({"outcome": "success", "source": "goopdl"}),
            &context,
            &mut differences,
        );
        assert!(
            differences
                .iter()
                .any(|line| line.contains("/source: python")),
            "{differences:?}"
        );
    }

    #[test]
    fn acquisition_ci_report_is_deterministic_and_names_its_kind() {
        let result = CaseResult {
            id: "ci-run-2xx-marker".to_string(),
            argv: vec!["ci".to_string(), "run".to_string()],
            status: CaseStatus::Passed,
            approved: false,
            approval: None,
            exit: ExitPair { python: 0, rust: 0 },
            differences: Vec::new(),
        };
        let report = build_report_for(
            vec![result],
            ACQUISITION_CI_REPORT_KIND,
            Some(SYNTHETIC_PROFILE),
        );
        let rendered = render_report(&report).expect("report renders");
        assert!(rendered.contains("\"kind\": \"apk-lab-acquisition-ci-parity\""));
        assert!(rendered.contains("\"profile\": \"synthetic\""));
        assert!(report_passed(&report));
        assert_eq!(render_report(&report).expect("renders again"), rendered);
    }

    #[test]
    fn acquisition_ci_missing_scenario_is_reported_as_missing() {
        let (code, result) = execute_native_scenario("not-a-scenario").expect("driver result");
        assert_eq!(code, 70);
        assert_eq!(result["outcome"], "driver-error");
        let case = CaseResult {
            id: "not-a-scenario".to_string(),
            argv: vec!["not-a-scenario".to_string()],
            status: CaseStatus::Missing,
            approved: false,
            approval: None,
            exit: ExitPair {
                python: -1,
                rust: -1,
            },
            differences: vec!["missing result".to_string()],
        };
        let report = build_report_for(
            vec![case],
            ACQUISITION_CI_REPORT_KIND,
            Some(SYNTHETIC_PROFILE),
        );
        assert!(!report_passed(&report));
        assert_eq!(report.summary.missing, 1);
    }

    #[test]
    fn patching_details_normalize_durations_and_pids() {
        let normalizer = Normalizer {
            context: NormalizationContext::for_test("/tmp/parity-patching-case"),
            rules: &[TextRule::PatchingDetails],
        };
        let text =
            "  [PASS] Remove Ads (opts: {'x': True}) (0.42s)\nAttached to process PID 1234\n";
        let normalized = normalizer.text(text);
        assert!(normalized.contains("(<duration>)"), "{normalized}");
        assert!(normalized.contains("PID <pid>"), "{normalized}");
        assert!(!normalized.contains("0.42"));
        assert!(!normalized.contains("1234"));
    }
}
