//! Icon pixel-compatibility preflight for the asset migration (plan 011).
//!
//! `preflight icons` executes the reusable `asset-prep` icon transform over a
//! bounded manifest of frozen inputs and compares the candidate RGBA pixels
//! against references captured with the pinned Pillow oracle. Comparison is
//! native Rust and byte-exact: there is no tolerance, perceptual metric, or
//! channel rounding. The harness may invoke the isolated, pinned Pillow oracle
//! to decode the candidate PNG independently, so a shared native encode/decode
//! defect cannot hide a pixel difference.
//!
//! `--check` re-executes the transform and re-hashes the source, inputs, and
//! references. It treats the report as untrusted evidence: every recorded
//! value must equal the freshly computed value, and acceptance is derived from
//! the fresh run. Evidence freshness is also bound to the compiled native
//! candidate: the bytes of the asset-prep and harness sources, their Cargo
//! manifests/lockfiles, and the SHA-256 of the executing binary are recorded.
//! A binary whose embedded sources no longer match the checkout is rejected
//! before it can capture or verify a report.
//!
//! `--sanitize --input RAW --out PUBLIC` projects an existing private capture
//! onto a distinct public-summary schema. It preserves measurements, hashes,
//! and case outcomes while removing private paths and diagnostic strings, and
//! refuses to write output that still leaks a private path. The public artifact
//! is explicitly not replayable: only the raw capture plus the private inputs
//! can be re-checked. The production icon script, workflow, and committed
//! icons are never read for modification and never written by this harness.

use asset_prep::icons::{transform_icon, write_icon_if_changed, IconWriteDecision};
use flate2::read::ZlibDecoder;
use png::{BitDepth, ColorType, Compression, Encoder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 2;
const REPORT_KIND: &str = "apk-lab-icon-preflight-report";
const REPORT_MODE: &str = "capture";
const PUBLIC_REPORT_SCHEMA_VERSION: u32 = 1;
const PUBLIC_REPORT_KIND: &str = "apk-lab-icon-preflight-public-summary";
const PUBLIC_REPORT_MODE: &str = "sanitized";
const ICON_SIZE: u32 = 128;
const RGBA_LEN: usize = (ICON_SIZE as usize) * (ICON_SIZE as usize) * 4;
const MAX_DECODED_INPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_FIRST_DIFFERENCES: usize = 16;
const MAX_ERROR_CHARS: usize = 512;
const ORACLE_ENV: &str = "APK_LAB_ICON_ORACLE_PYTHON";

const REQUIREMENT_KEYS: &[&str] = &[
    "pythonImplementation",
    "pythonVersion",
    "pillowVersion",
    "platform",
    "machine",
    "architecture",
];

/// Bytes of the native candidate’s own source and dependency manifests as they
/// existed when this binary was compiled.
///
/// `include_bytes!` embeds the raw bytes; the fingerprint is computed over
/// those bytes at run time. Hashing a raw included source (rather than writing
/// a precomputed digest into the source) keeps the evidence non-recursive.
struct CompiledSource {
    /// Repo-relative path used for reporting and on-disk comparison.
    path: &'static str,
    bytes: &'static [u8],
}

const COMPILED_SOURCES: &[CompiledSource] = &[
    CompiledSource {
        path: "tools/asset-prep/src/lib.rs",
        bytes: include_bytes!("../../../asset-prep/src/lib.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/icons.rs",
        bytes: include_bytes!("../../../asset-prep/src/icons.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/main.rs",
        bytes: include_bytes!("../../../asset-prep/src/main.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/cli.rs",
        bytes: include_bytes!("../../../asset-prep/src/cli.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/error.rs",
        bytes: include_bytes!("../../../asset-prep/src/error.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/cache.rs",
        bytes: include_bytes!("../../../asset-prep/src/cache.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/font.rs",
        bytes: include_bytes!("../../../asset-prep/src/font.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/ocr.rs",
        bytes: include_bytes!("../../../asset-prep/src/ocr.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/pdfbox.rs",
        bytes: include_bytes!("../../../asset-prep/src/pdfbox.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/src/icon_sync.rs",
        bytes: include_bytes!("../../../asset-prep/src/icon_sync.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/Cargo.toml",
        bytes: include_bytes!("../../../asset-prep/Cargo.toml"),
    },
    CompiledSource {
        path: "tools/asset-prep/Cargo.lock",
        bytes: include_bytes!("../../../asset-prep/Cargo.lock"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/Cargo.toml",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/Cargo.toml"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/build.rs",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/build.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/CMakeLists.txt",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/CMakeLists.txt"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/cmake/patch_libjpeg.cmake",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/cmake/patch_libjpeg.cmake"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/src/lib.rs",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/src/lib.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/bindings.rs",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/bindings.rs"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/NOTICE.md",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/NOTICE.md"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/LICENSE-MIT",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/LICENSE-MIT"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/LIBJPEG-TURBO-LICENSE.md",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/LIBJPEG-TURBO-LICENSE.md"),
    },
    CompiledSource {
        path: "tools/asset-prep/vendor/turbojpeg-sys/LIBJPEG-TURBO-README.ijg",
        bytes: include_bytes!("../../../asset-prep/vendor/turbojpeg-sys/LIBJPEG-TURBO-README.ijg"),
    },
    CompiledSource {
        path: "tools/apk-lab/src/migration/icons.rs",
        bytes: include_bytes!("icons.rs"),
    },
    CompiledSource {
        path: "tools/apk-lab/src/migration/mod.rs",
        bytes: include_bytes!("mod.rs"),
    },
    CompiledSource {
        path: "tools/apk-lab/Cargo.toml",
        bytes: include_bytes!("../../Cargo.toml"),
    },
    CompiledSource {
        path: "tools/apk-lab/Cargo.lock",
        bytes: include_bytes!("../../Cargo.lock"),
    },
    CompiledSource {
        path: "tools/apk-lab/migration/fixtures/icons/required-corpus.json",
        bytes: include_bytes!("../../migration/fixtures/icons/required-corpus.json"),
    },
];

const SOURCE_FINGERPRINT_DOMAIN: &[u8] = b"apk-lab-icon-candidate-sources-v1\0";
const REQUIRED_CORPUS_SCHEMA_VERSION: u32 = 1;
const REQUIRED_CORPUS_KIND: &str = "apk-lab-icon-required-corpus";
const REQUIRED_CORPUS_BYTES: &[u8] =
    include_bytes!("../../migration/fixtures/icons/required-corpus.json");

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    required_coverage: Vec<String>,
    reference: ReferenceSpec,
    cases: Vec<CaseSpec>,
    /// Optional capture extension: decoded-before-resize RGBA for decoder
    /// isolation. Present in both frozen manifests; absent entries are not
    /// treated as evidence.
    #[serde(default)]
    decoded_inputs: BTreeMap<String, DecodedInputSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DecodedInputSpec {
    path: String,
    /// SHA-256 of the original (decompressed) RGBA bytes, regardless of how
    /// the fixture stores them on disk.
    sha256: String,
    width: u32,
    height: u32,
    mode: String,
    /// Storage encoding of the referenced fixture. Omitted entries remain the
    /// historical raw RGBA format.
    #[serde(default)]
    encoding: DecodedInputEncoding,
    /// SHA-256 of the compressed file bytes. Required when `encoding` is
    /// `zlib`, because the stored hash is verified before inflating.
    #[serde(default)]
    stored_sha256: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DecodedInputEncoding {
    #[default]
    Raw,
    Zlib,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReferenceSpec {
    python_version: String,
    pillow_version: String,
    source_sha256: String,
    source_path: String,
    environment: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaseSpec {
    id: String,
    coverage: Vec<String>,
    visibility: Visibility,
    input_path: String,
    input_sha256: String,
    expected_outcome: ExpectedOutcome,
    reference_rgba_path: Option<String>,
    reference_rgba_sha256: Option<String>,
    reference_png_path: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExpectedOutcome {
    Success,
    Error,
}

/// Compiled acceptance floor for plan 011. This file is independent of any
/// caller-supplied manifest: a fresh capture of a reduced manifest cannot
/// narrow the proof.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RequiredCorpus {
    schema_version: u32,
    kind: String,
    note: String,
    required_coverage: Vec<String>,
    cases: Vec<RequiredCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RequiredCase {
    id: String,
    visibility: Visibility,
    expected_outcome: ExpectedOutcome,
    coverage: Vec<String>,
    input_sha256: String,
    reference_rgba_sha256: Option<String>,
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    kind: String,
    generated_at: String,
    mode: String,
    manifest_path: String,
    manifest_sha256: String,
    candidate: CandidateReport,
    requirements: RequirementsReport,
    reference: ReferenceReport,
    required_coverage: Vec<String>,
    oracle: OracleReport,
    summary: Summary,
    cases: Vec<CaseReport>,
    blockers: Vec<Blocker>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CandidateReport {
    source_fingerprint: String,
    binary_sha256: String,
    binary_file_name: String,
    /// Executed target OS (`std::env::consts::OS`), independent of the pinned
    /// Python reference environment. No hostname or `uname` output is recorded.
    os: String,
    /// Executed target architecture (`std::env::consts::ARCH`).
    arch: String,
    sources: Vec<SourceFileReport>,
}

/// Audit of the caller manifest against the compiled acceptance floor. These
/// expectations are deliberately separate from the candidate case outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RequirementsReport {
    corpus_sha256: String,
    required_coverage: Vec<String>,
    required_cases: usize,
    missing_cases: Vec<String>,
    altered_cases: Vec<String>,
    missing_coverage: Vec<String>,
    satisfied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourceFileReport {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReferenceReport {
    python_version: String,
    pillow_version: String,
    source_sha256: String,
    source_path: String,
    source_verified: bool,
    environment: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OracleReport {
    required_python_version: String,
    required_pillow_version: String,
    decoder: String,
    available: bool,
    usable: bool,
    python_version: Option<String>,
    pillow_version: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Summary {
    total: usize,
    matched: usize,
    mismatched: usize,
    success_matched: usize,
    error_matched: usize,
    private_cases: usize,
    private_successes: usize,
    covered_coverage: Vec<String>,
    missing_coverage: Vec<String>,
    acceptance_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaseReport {
    id: String,
    coverage: Vec<String>,
    visibility: Visibility,
    input_path: String,
    input_sha256: String,
    expected_outcome: ExpectedOutcome,
    reference_rgba_path: Option<String>,
    reference_rgba_sha256: Option<String>,
    reference_png_path: Option<String>,
    reference_png_sha256: Option<String>,
    actual_outcome: String,
    width: Option<u32>,
    height: Option<u32>,
    candidate_rgba_sha256: Option<String>,
    candidate_png_sha256: Option<String>,
    candidate_png_size: Option<u64>,
    input_decoded_rgba_sha256: Option<String>,
    input_decoded_width: Option<u32>,
    input_decoded_height: Option<u32>,
    input_decoded_mode: Option<String>,
    oracle_decoded_rgba_sha256: Option<String>,
    encoder_roundtrip_matched: Option<bool>,
    reference_png_decoded_sha256: Option<String>,
    reference_png_matched: Option<bool>,
    matched: bool,
    differing_bytes: Option<u64>,
    differing_channels: Option<ChannelCounts>,
    first_differences: Vec<Difference>,
    error: Option<String>,
    write: Option<WriteEvidence>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ChannelCounts {
    r: u64,
    g: u64,
    b: u64,
    a: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Difference {
    byte_index: usize,
    pixel_index: usize,
    x: u32,
    y: u32,
    channel: String,
    reference: u8,
    candidate: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WriteEvidence {
    first: String,
    second: String,
    candidate_sha256: String,
    preserved: bool,
    alternate_encoding_sha256: String,
    alternate_encoding_preserved: bool,
    alternate_mtime_preserved: bool,
    alternate_encoding_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Blocker {
    kind: String,
    detail: String,
}

/// Public projection of a private capture. It deliberately has a different
/// `kind`, is marked non-replayable, and omits the manifest path and all
/// private path/error text while retaining measured evidence.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicReport {
    schema_version: u32,
    kind: &'static str,
    replayable: bool,
    check_supported: bool,
    notice: &'static str,
    generated_at: String,
    mode: &'static str,
    raw_report_sha256: String,
    manifest_sha256: String,
    candidate: CandidateReport,
    requirements: RequirementsReport,
    reference: ReferenceReport,
    required_coverage: Vec<String>,
    oracle: OracleReport,
    summary: Summary,
    cases: Vec<CaseReport>,
    blockers: Vec<Blocker>,
}

const PUBLIC_NOTICE: &str = "Sanitized public summary of a private icon preflight capture. \
Measured hashes, dimensions, outcomes, and diff counts are preserved; private paths and \
diagnostic/reason strings are removed. This artifact is not replayable and --check does not \
accept it; verifying the proof requires the raw capture and the private input files.";

struct DecodedEvidence {
    sha256: String,
    width: u32,
    height: u32,
    mode: String,
}

#[derive(Debug, Default)]
struct DiffStats {
    differing_bytes: u64,
    channels: ChannelCounts,
    first_differences: Vec<Difference>,
}

impl DiffStats {
    fn is_empty(&self) -> bool {
        self.differing_bytes == 0
    }
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

enum Invocation {
    Help,
    Capture {
        manifest: PathBuf,
        out: PathBuf,
    },
    Check {
        report: PathBuf,
        manifest: Option<PathBuf>,
    },
    Sanitize {
        input: PathBuf,
        out: PathBuf,
    },
}

/// Entry point for `apk-lab-migrate preflight icons ...`.
pub fn run(args: &[String]) -> Result<(), String> {
    match parse_args(args)? {
        Invocation::Help => {
            print_help();
            Ok(())
        }
        Invocation::Capture { manifest, out } => capture_command(&manifest, &out),
        Invocation::Check { report, manifest } => check_command(&report, manifest.as_deref()),
        Invocation::Sanitize { input, out } => sanitize_command(&input, &out),
    }
}

fn print_help() {
    println!(
        "Usage:\n\
         \x20 apk-lab-migrate preflight icons --manifest PATH --out PATH\n\
         \x20 apk-lab-migrate preflight icons --check --report PATH [--manifest PATH]\n\
         \x20 apk-lab-migrate preflight icons --sanitize --input RAW --out PUBLIC\n\
         \n\
         Executes the reusable asset-prep icon transform over a frozen manifest and\n\
         compares every candidate RGBA byte against the independently captured Pillow\n\
         reference. Success requires exactly 128x128 RGBA output, zero differing\n\
         bytes, and no acceptance blockers (source/oracle/private coverage/candidate\n\
         freshness); there is no visual tolerance.\n\
         \n\
         --manifest PATH   icon pixel manifest (reference provenance + frozen cases)\n\
         --out PATH        write the capture report ('-' for stdout)\n\
         --check           re-execute and verify an existing capture report\n\
         --report PATH     capture report to verify (required with --check)\n\
         --sanitize        publish a sanitized, non-replayable summary of RAW\n\
         --input PATH      raw capture report to sanitize (required with --sanitize)\n\
         \n\
         The only optional --manifest with --check overrides the path recorded in the\n\
         report. The pinned Pillow oracle is located from APK_LAB_ICON_ORACLE_PYTHON or\n\
         the isolated migration environment; it is used only to decode candidate and\n\
         reference PNGs independently. Native transformation never invokes Python.\n\
         Capture and --check reject a binary whose embedded candidate sources or\n\
         Cargo manifests/lockfiles no longer match the checkout. The sanitized\n\
         artifact keeps measurements and hashes but removes private paths and\n\
         diagnostic strings; it cannot be used with --check."
    );
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut check = false;
    let mut sanitize = false;
    let mut manifest: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut input: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "--check" => {
                if check {
                    return Err("--check specified more than once".to_string());
                }
                check = true;
            }
            "--sanitize" => {
                if sanitize {
                    return Err("--sanitize specified more than once".to_string());
                }
                sanitize = true;
            }
            "--manifest" => {
                let value = take_value(args, &mut index, "--manifest")?;
                set_once(&mut manifest, &value, "--manifest")?;
            }
            "--out" => {
                let value = take_value(args, &mut index, "--out")?;
                set_once(&mut out, &value, "--out")?;
            }
            "--report" => {
                let value = take_value(args, &mut index, "--report")?;
                set_once(&mut report, &value, "--report")?;
            }
            "--input" => {
                let value = take_value(args, &mut index, "--input")?;
                set_once(&mut input, &value, "--input")?;
            }
            _ if arg.starts_with("--manifest=") => {
                let value = arg.strip_prefix("--manifest=").unwrap_or("");
                set_once(&mut manifest, value, "--manifest")?;
            }
            _ if arg.starts_with("--out=") => {
                let value = arg.strip_prefix("--out=").unwrap_or("");
                set_once(&mut out, value, "--out")?;
            }
            _ if arg.starts_with("--report=") => {
                let value = arg.strip_prefix("--report=").unwrap_or("");
                set_once(&mut report, value, "--report")?;
            }
            _ if arg.starts_with("--input=") => {
                let value = arg.strip_prefix("--input=").unwrap_or("");
                set_once(&mut input, value, "--input")?;
            }
            _ => return Err(format!("unexpected preflight icons argument '{arg}'")),
        }
        index += 1;
    }

    if sanitize {
        if check {
            return Err("--sanitize cannot be combined with --check".to_string());
        }
        if manifest.is_some() || report.is_some() {
            return Err("--sanitize reads --input RAW and writes --out PUBLIC; \
                 --manifest and --report are not valid"
                .to_string());
        }
        let input = input.ok_or("preflight icons --sanitize requires --input RAW")?;
        let out = out.ok_or("preflight icons --sanitize requires --out PUBLIC")?;
        return Ok(Invocation::Sanitize { input, out });
    }

    if input.is_some() {
        return Err("--input is only valid with --sanitize".to_string());
    }

    if check {
        let report = report.ok_or("preflight icons --check requires --report PATH")?;
        if out.is_some() {
            return Err("--out is only valid for a capture; use --report with --check".to_string());
        }
        return Ok(Invocation::Check { report, manifest });
    }

    if report.is_some() {
        return Err("--report is only valid with --check; use --out for a capture".to_string());
    }
    let manifest = manifest.ok_or("preflight icons requires --manifest PATH")?;
    let out = out.ok_or("preflight icons requires --out PATH")?;
    Ok(Invocation::Capture { manifest, out })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn set_once(slot: &mut Option<PathBuf>, value: &str, flag: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{flag} requires a non-empty value"));
    }
    if slot.is_some() {
        return Err(format!("{flag} specified more than once"));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn capture_command(manifest_path: &Path, out: &Path) -> Result<(), String> {
    capture_command_with_mode(manifest_path, out, OracleMode::Auto)
}

fn capture_command_with_mode(
    manifest_path: &Path,
    out: &Path,
    oracle_mode: OracleMode,
) -> Result<(), String> {
    let manifest_bytes = fs::read(manifest_path)
        .map_err(|error| format!("cannot read manifest {}: {error}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid icon manifest: {error}"))?;
    validate_manifest(&manifest)?;

    let display = manifest_path.to_string_lossy().into_owned();
    let report = evaluate(
        &display,
        manifest_path,
        &manifest,
        sha256_hex(&manifest_bytes),
        oracle_mode,
    )?;
    write_report(&report, Some(out))?;
    if out != Path::new("-") {
        print_summary(&report);
    }

    // Publish the diagnostic report first, then fail whenever acceptance is
    // incomplete — a pixel match alone does not satisfy the proof.
    if !report.summary.acceptance_passed {
        return Err(format!(
            "icon preflight capture did not pass acceptance: {} blocker(s): {}",
            report.blockers.len(),
            blocker_summary(&report.blockers)
        ));
    }
    if report.summary.mismatched != 0 {
        return Err(format!(
            "icon preflight capture failed: {} of {} case(s) did not match",
            report.summary.mismatched, report.summary.total
        ));
    }
    Ok(())
}

fn check_command(report_path: &Path, manifest_override: Option<&Path>) -> Result<(), String> {
    let text = fs::read_to_string(report_path)
        .map_err(|error| format!("cannot read report {}: {error}", report_path.display()))?;
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if value.get("kind").and_then(Value::as_str) == Some(PUBLIC_REPORT_KIND) {
            return Err(format!(
                "{} is a sanitized public summary, not a replayable capture; --check \
                 requires the raw report and the private input files",
                report_path.display()
            ));
        }
    }
    let report: Report = serde_json::from_str(&text)
        .map_err(|error| format!("invalid icon preflight report: {error}"))?;
    verify_report(&report, report_path, manifest_override, OracleMode::Auto)
}

fn sanitize_command(input: &Path, out: &Path) -> Result<(), String> {
    if input == Path::new("-") {
        return Err("--sanitize requires a real --input file, not '-'".to_string());
    }
    if input == out {
        return Err("--sanitize --input and --out must be different files".to_string());
    }
    let text = fs::read_to_string(input)
        .map_err(|error| format!("cannot read report {}: {error}", input.display()))?;
    let report: Report = serde_json::from_str(&text)
        .map_err(|error| format!("invalid icon preflight report {}: {error}", input.display()))?;
    if report.schema_version != REPORT_SCHEMA_VERSION
        || report.kind != REPORT_KIND
        || report.mode != REPORT_MODE
    {
        return Err(format!(
            "{} is not a capture report that can be sanitized",
            input.display()
        ));
    }

    let public = sanitize_report(&report, sha256_hex(text.as_bytes()));
    let json = serde_json::to_string_pretty(&public)
        .map_err(|error| format!("cannot serialize sanitized report: {error}"))?;
    assert_no_private_leak(&json, &forbidden_private_paths(&report))?;
    write_json_text(&json, out)?;
    println!(
        "sanitized icon preflight report: {} case(s), {} matched, acceptance={} \
         (non-replayable public summary)",
        public.summary.total, public.summary.matched, public.summary.acceptance_passed
    );
    Ok(())
}

fn sanitize_report(report: &Report, raw_report_sha256: String) -> PublicReport {
    let mut reference = report.reference.clone();
    reference.source_path = sanitize_path(&reference.source_path);
    reference.environment = reference
        .environment
        .into_iter()
        .map(|(key, value)| (key, sanitize_value(value)))
        .collect();

    let mut oracle = report.oracle.clone();
    oracle.reason = oracle.reason.as_ref().map(|_| "<sanitized>".to_string());

    let mut candidate = report.candidate.clone();
    candidate.binary_file_name = sanitize_path(&candidate.binary_file_name);
    for source in &mut candidate.sources {
        source.path = sanitize_path(&source.path);
    }

    let cases = report
        .cases
        .iter()
        .map(|case| {
            let mut case = case.clone();
            let private = case.visibility == Visibility::Private;
            let id = case.id.clone();
            case.input_path = sanitize_case_path(&case.input_path, private, &id, "input");
            case.reference_rgba_path = case
                .reference_rgba_path
                .as_deref()
                .map(|path| sanitize_case_path(path, private, &id, "reference-rgba"));
            case.reference_png_path = case
                .reference_png_path
                .as_deref()
                .map(|path| sanitize_case_path(path, private, &id, "reference-png"));
            case.error = case.error.as_ref().map(|_| "<sanitized>".to_string());
            case
        })
        .collect();

    let blockers = report
        .blockers
        .iter()
        .map(|blocker| Blocker {
            kind: sanitize_path(&blocker.kind),
            detail: "<sanitized>".to_string(),
        })
        .collect();

    PublicReport {
        schema_version: PUBLIC_REPORT_SCHEMA_VERSION,
        kind: PUBLIC_REPORT_KIND,
        replayable: false,
        check_supported: false,
        notice: PUBLIC_NOTICE,
        generated_at: report.generated_at.clone(),
        mode: PUBLIC_REPORT_MODE,
        raw_report_sha256,
        manifest_sha256: report.manifest_sha256.clone(),
        candidate,
        requirements: report.requirements.clone(),
        reference,
        required_coverage: report.required_coverage.clone(),
        oracle,
        summary: report.summary.clone(),
        cases,
        blockers,
    }
}

fn sanitize_case_path(value: &str, private: bool, id: &str, role: &str) -> String {
    if private {
        format!("<private:{role}:{id}>")
    } else {
        sanitize_path(value)
    }
}

fn sanitize_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize_path(&text)),
        other => other,
    }
}

/// Replaces absolute or home-relative paths with an opaque token. Public
/// repo-relative paths and version strings are left intact.
fn sanitize_path(value: &str) -> String {
    if value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("\\\\")
        || value.contains("/Users/")
        || value.contains("\\Users\\")
        || value.contains("/home/")
        || home_directory()
            .map(|home| !home.is_empty() && value.contains(home.as_str()))
            .unwrap_or(false)
    {
        return "<private-path>".to_string();
    }
    value.to_string()
}

fn home_directory() -> Option<String> {
    env::var_os("HOME").map(|value| value.to_string_lossy().into_owned())
}

fn forbidden_private_paths(report: &Report) -> Vec<String> {
    let mut paths = vec![report.manifest_path.clone()];
    for case in &report.cases {
        if case.visibility == Visibility::Private {
            paths.push(case.input_path.clone());
            if let Some(path) = &case.reference_rgba_path {
                paths.push(path.clone());
            }
            if let Some(path) = &case.reference_png_path {
                paths.push(path.clone());
            }
        }
    }
    paths
}

fn assert_no_private_leak(json: &str, forbidden: &[String]) -> Result<(), String> {
    const MARKERS: &[&str] = &[
        "/Users/",
        "/home/",
        "/root/",
        "/private/",
        "/var/folders/",
        "\\\\Users\\\\",
    ];
    for &marker in MARKERS {
        if json.contains(marker) {
            return Err(format!(
                "sanitized report leaked a private path marker ({marker}); refusing to write"
            ));
        }
    }
    if let Some(home) = home_directory() {
        if home.len() > 1 && json.contains(home.as_str()) {
            return Err(
                "sanitized report leaked the home directory; refusing to write".to_string(),
            );
        }
    }
    for path in forbidden {
        if path.len() >= 6 && json.contains(path.as_str()) {
            return Err(format!(
                "sanitized report leaked a private path ({path}); refusing to write"
            ));
        }
    }

    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("cannot inspect sanitized report: {error}"))?;
    if let Some(found) = find_absolute_string(&value) {
        return Err(format!(
            "sanitized report contains an absolute path ({found}); refusing to write"
        ));
    }
    Ok(())
}

fn find_absolute_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let bytes = text.as_bytes();
            let drive_absolute = bytes.len() > 2
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            if text.starts_with('/')
                || text.starts_with("~/")
                || drive_absolute
                || text.contains("/Users/")
                || text.contains("\\Users\\")
            {
                Some(text.chars().take(120).collect())
            } else {
                None
            }
        }
        Value::Array(items) => items.iter().find_map(find_absolute_string),
        Value::Object(map) => map.values().find_map(find_absolute_string),
        _ => None,
    }
}

fn verify_report(
    report: &Report,
    report_path: &Path,
    manifest_override: Option<&Path>,
    oracle_mode: OracleMode,
) -> Result<(), String> {
    if report.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported report schemaVersion {}",
            report.schema_version
        ));
    }
    if report.kind != REPORT_KIND {
        return Err(format!("report kind must be {REPORT_KIND}"));
    }
    if report.mode != REPORT_MODE {
        return Err(format!("report mode must be {REPORT_MODE}"));
    }
    if parse_utc(&report.generated_at).is_none() {
        return Err("report generatedAt must be a UTC RFC 3339 timestamp".to_string());
    }
    if report.manifest_path.trim().is_empty() {
        return Err("report manifestPath is empty".to_string());
    }

    // Bind the evidence to the native implementation that produced it before
    // doing any work. `current_candidate` also refuses a binary whose embedded
    // sources no longer match the checkout.
    let observed = current_candidate()?;
    if observed.source_fingerprint != report.candidate.source_fingerprint {
        return Err(
            "stale icon preflight report: the compiled native candidate source fingerprint \
             changed after capture (sourceFingerprint mismatch)"
                .to_string(),
        );
    }
    if observed.binary_sha256 != report.candidate.binary_sha256 {
        return Err(
            "stale icon preflight report: this executable is not the binary that captured \
             the report (binarySha256 mismatch)"
                .to_string(),
        );
    }

    let manifest_path = match manifest_override {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(&report.manifest_path),
    };
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "cannot read manifest {} referenced by the report: {error}",
            manifest_path.display()
        )
    })?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    if manifest_sha256 != report.manifest_sha256 {
        return Err(
            "stale icon preflight report: the manifest changed after capture \
             (manifestSha256 mismatch)"
                .to_string(),
        );
    }

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid icon manifest: {error}"))?;
    validate_manifest(&manifest)?;

    let mut fresh = evaluate(
        &report.manifest_path,
        &manifest_path,
        &manifest,
        manifest_sha256,
        oracle_mode,
    )?;
    fresh.generated_at = report.generated_at.clone();

    if &fresh != report {
        let report_value = serde_json::to_value(report)
            .map_err(|error| format!("cannot serialize report: {error}"))?;
        let fresh_value = serde_json::to_value(&fresh)
            .map_err(|error| format!("cannot serialize fresh evidence: {error}"))?;
        let detail = first_json_difference(&report_value, &fresh_value, "$")
            .unwrap_or_else(|| "report differs from fresh evidence".to_string());
        return Err(format!(
            "stale or altered icon preflight report {}: {detail}",
            report_path.display()
        ));
    }

    if !fresh.summary.acceptance_passed {
        let reasons = blocker_summary(&fresh.blockers);
        return Err(format!(
            "icon preflight acceptance failed: {} blocker(s): {reasons}",
            fresh.blockers.len()
        ));
    }

    println!(
        "verified {} icon case(s), {} required coverage tag(s), exact decoded RGBA parity",
        fresh.summary.total,
        fresh.required_coverage.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "unsupported icon manifest schemaVersion {}",
            manifest.schema_version
        ));
    }
    if manifest.cases.is_empty() {
        return Err("icon manifest must declare at least one case".to_string());
    }
    if manifest.required_coverage.is_empty() {
        return Err("icon manifest requiredCoverage must not be empty".to_string());
    }

    let mut required = BTreeSet::new();
    for tag in &manifest.required_coverage {
        validate_tag("requiredCoverage tag", tag)?;
        if !required.insert(tag.as_str()) {
            return Err(format!("duplicate requiredCoverage tag '{tag}'"));
        }
    }

    validate_version(
        "reference.pythonVersion",
        &manifest.reference.python_version,
    )?;
    validate_version(
        "reference.pillowVersion",
        &manifest.reference.pillow_version,
    )?;
    validate_sha256("reference.sourceSha256", &manifest.reference.source_sha256)?;
    if manifest.reference.source_path.trim().is_empty()
        || is_placeholder(&manifest.reference.source_path)
    {
        return Err("reference.sourcePath must identify the capture source".to_string());
    }
    if manifest.reference.environment.is_empty() {
        return Err("reference.environment must not be empty".to_string());
    }
    for (key, value) in &manifest.reference.environment {
        if key.is_empty() || key.len() > 64 {
            return Err("reference.environment keys must be 1..=64 characters".to_string());
        }
        validate_environment_value(&format!("reference.environment.{key}"), value)?;
    }
    for key in REQUIREMENT_KEYS {
        let value =
            manifest.reference.environment.get(*key).ok_or_else(|| {
                format!("reference.environment must record '{key}' from the oracle")
            })?;
        let text = value
            .as_str()
            .ok_or_else(|| format!("reference.environment.{key} must be a string"))?;
        if is_placeholder(text) {
            return Err(format!(
                "reference.environment.{key} must not be a placeholder"
            ));
        }
    }
    if let Some(env_python) = manifest
        .reference
        .environment
        .get("pythonVersion")
        .and_then(Value::as_str)
    {
        if env_python != manifest.reference.python_version {
            return Err(
                "reference.environment.pythonVersion must match reference.pythonVersion"
                    .to_string(),
            );
        }
    }
    if let Some(env_pillow) = manifest
        .reference
        .environment
        .get("pillowVersion")
        .and_then(Value::as_str)
    {
        if env_pillow != manifest.reference.pillow_version {
            return Err(
                "reference.environment.pillowVersion must match reference.pillowVersion"
                    .to_string(),
            );
        }
    }

    if let Some(env_source) = manifest
        .reference
        .environment
        .get("sourcePath")
        .and_then(Value::as_str)
    {
        if env_source != manifest.reference.source_path {
            return Err(
                "reference.environment.sourcePath must match reference.sourcePath".to_string(),
            );
        }
    }

    let mut ids = BTreeSet::new();
    for case in &manifest.cases {
        validate_id("case id", &case.id)?;
        if !ids.insert(case.id.as_str()) {
            return Err(format!("duplicate case id '{}'", case.id));
        }
        if case.coverage.is_empty() {
            return Err(format!("case '{}' has no coverage tags", case.id));
        }
        let mut case_tags = BTreeSet::new();
        for tag in &case.coverage {
            validate_tag("case coverage tag", tag)?;
            if !required.contains(tag.as_str()) {
                return Err(format!(
                    "case '{}' coverage tag '{}' is absent from requiredCoverage",
                    case.id, tag
                ));
            }
            if !case_tags.insert(tag.as_str()) {
                return Err(format!("case '{}' repeats coverage tag '{}'", case.id, tag));
            }
        }
        if case.input_path.trim().is_empty() {
            return Err(format!("case '{}' has an empty inputPath", case.id));
        }
        validate_sha256(
            &format!("case '{}' inputSha256", case.id),
            &case.input_sha256,
        )?;

        match case.expected_outcome {
            ExpectedOutcome::Success => {
                let rgba_path = case.reference_rgba_path.as_deref().ok_or_else(|| {
                    format!("successful case '{}' requires referenceRgbaPath", case.id)
                })?;
                if rgba_path.trim().is_empty() {
                    return Err(format!("case '{}' referenceRgbaPath is empty", case.id));
                }
                let rgba_sha = case.reference_rgba_sha256.as_deref().ok_or_else(|| {
                    format!("successful case '{}' requires referenceRgbaSha256", case.id)
                })?;
                validate_sha256(&format!("case '{}' referenceRgbaSha256", case.id), rgba_sha)?;
                let png_path = case.reference_png_path.as_deref().ok_or_else(|| {
                    format!("successful case '{}' requires referencePngPath", case.id)
                })?;
                if png_path.trim().is_empty() {
                    return Err(format!("case '{}' referencePngPath is empty", case.id));
                }
                if case.width != Some(ICON_SIZE) || case.height != Some(ICON_SIZE) {
                    return Err(format!(
                        "successful case '{}' must expect {ICON_SIZE}x{ICON_SIZE}",
                        case.id
                    ));
                }
            }
            ExpectedOutcome::Error => {
                if case.reference_rgba_path.is_some()
                    || case.reference_rgba_sha256.is_some()
                    || case.reference_png_path.is_some()
                    || case.width.is_some()
                    || case.height.is_some()
                {
                    return Err(format!(
                        "error case '{}' must leave reference paths, hashes, and dimensions null",
                        case.id
                    ));
                }
            }
        }
    }

    for (id, decoded) in &manifest.decoded_inputs {
        let case = manifest
            .cases
            .iter()
            .find(|case| &case.id == id)
            .ok_or_else(|| format!("decodedInputs references unknown case '{id}'"))?;
        if case.expected_outcome != ExpectedOutcome::Success {
            return Err(format!("decodedInputs must not describe error case '{id}'"));
        }
        if decoded.path.trim().is_empty() || is_placeholder(&decoded.path) {
            return Err(format!("decodedInputs.{id}.path must not be empty"));
        }
        validate_sha256(&format!("decodedInputs.{id}.sha256"), &decoded.sha256)?;
        match decoded.encoding {
            DecodedInputEncoding::Zlib => {
                let stored = decoded.stored_sha256.as_deref().ok_or_else(|| {
                    format!("decodedInputs.{id}.storedSha256 is required when encoding is 'zlib'")
                })?;
                validate_sha256(&format!("decodedInputs.{id}.storedSha256"), stored)?;
            }
            DecodedInputEncoding::Raw => {
                if let Some(stored) = decoded.stored_sha256.as_deref() {
                    validate_sha256(&format!("decodedInputs.{id}.storedSha256"), stored)?;
                }
            }
        }
        if decoded.width == 0
            || decoded.height == 0
            || decoded.width > 65_535
            || decoded.height > 65_535
        {
            return Err(format!("decodedInputs.{id} dimensions must be 1..=65535"));
        }
        if decoded.mode.trim().is_empty() || is_placeholder(&decoded.mode) {
            return Err(format!("decodedInputs.{id}.mode must not be a placeholder"));
        }
    }

    Ok(())
}

fn validate_environment_value(field: &str, value: &Value) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if text.trim().is_empty() || is_placeholder(text) {
                return Err(format!("{field} must not be empty or a placeholder"));
            }
            if text.len() > 512 {
                return Err(format!("{field} is too long"));
            }
        }
        Value::Number(_) | Value::Bool(_) => {}
        Value::Null | Value::Array(_) | Value::Object(_) => {
            return Err(format!("{field} must be a scalar value"));
        }
    }
    Ok(())
}

fn validate_tag(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 96 {
        return Err(format!("{field} must be 1..=96 characters"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
    {
        return Err(format!("{field} contains unsupported characters"));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 96 {
        return Err(format!("{field} must be 1..=96 characters"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "{field} must contain only ASCII alphanumerics, '.', '_' or '-'"
        ));
    }
    Ok(())
}

fn validate_version(field: &str, value: &str) -> Result<(), String> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(format!(
            "{field} must be a numeric dotted version (got '{value}')"
        ));
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{field} must be 64 hexadecimal characters"));
    }
    if value.bytes().all(|byte| byte == b'0') {
        return Err(format!("{field} must not be a placeholder digest"));
    }
    Ok(())
}

fn is_placeholder(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "" | "unknown"
            | "none"
            | "null"
            | "n/a"
            | "na"
            | "todo"
            | "tbd"
            | "placeholder"
            | "unset"
            | "?"
    ) || lower.contains("placeholder")
        || lower.contains("<")
        || lower.contains("todo")
}

// ---------------------------------------------------------------------------
// Compiled acceptance floor
// ---------------------------------------------------------------------------

/// Frozen counts from the approved plan-011 corpus: all public fixture cases
/// plus the three captured Play-original private cases. Keeping these counts
/// compiled prevents a rebuilt corpus from silently dropping cases.
const REQUIRED_PUBLIC_CASES: usize = 33;
const REQUIRED_PRIVATE_CASES: usize = 3;

fn required_corpus() -> Result<RequiredCorpus, String> {
    let corpus: RequiredCorpus = serde_json::from_slice(REQUIRED_CORPUS_BYTES)
        .map_err(|error| format!("invalid compiled icon required corpus: {error}"))?;
    if corpus.schema_version != REQUIRED_CORPUS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported required corpus schemaVersion {}",
            corpus.schema_version
        ));
    }
    if corpus.kind != REQUIRED_CORPUS_KIND {
        return Err(format!(
            "required corpus kind must be {REQUIRED_CORPUS_KIND}"
        ));
    }
    if corpus.note.trim().is_empty() {
        return Err("required corpus must record its provenance note".to_string());
    }
    if corpus.required_coverage.is_empty() || corpus.cases.is_empty() {
        return Err("required corpus must declare coverage and cases".to_string());
    }

    let mut required_tags = BTreeSet::new();
    for tag in &corpus.required_coverage {
        validate_tag("required corpus coverage tag", tag)?;
        if !required_tags.insert(tag.as_str()) {
            return Err(format!("required corpus repeats coverage tag '{tag}'"));
        }
    }

    let mut ids = BTreeSet::new();
    let mut public_cases = 0usize;
    let mut private_cases = 0usize;
    for case in &corpus.cases {
        validate_id("required corpus case id", &case.id)?;
        if !ids.insert(case.id.as_str()) {
            return Err(format!("required corpus repeats case id '{}'", case.id));
        }
        match case.visibility {
            Visibility::Public => public_cases += 1,
            Visibility::Private => private_cases += 1,
        }
        if case.coverage.is_empty() {
            return Err(format!(
                "required corpus case '{}' has no coverage tags",
                case.id
            ));
        }
        let mut case_tags = BTreeSet::new();
        for tag in &case.coverage {
            validate_tag("required corpus case coverage tag", tag)?;
            if !required_tags.contains(tag.as_str()) {
                return Err(format!(
                    "required corpus case '{}' coverage tag '{}' is absent from \
                     requiredCoverage",
                    case.id, tag
                ));
            }
            if !case_tags.insert(tag.as_str()) {
                return Err(format!(
                    "required corpus case '{}' repeats coverage tag '{}'",
                    case.id, tag
                ));
            }
        }
        validate_sha256(
            &format!("required corpus case '{}' inputSha256", case.id),
            &case.input_sha256,
        )?;
        match case.expected_outcome {
            ExpectedOutcome::Success => {
                let sha = case.reference_rgba_sha256.as_deref().ok_or_else(|| {
                    format!(
                        "required corpus success case '{}' lacks referenceRgbaSha256",
                        case.id
                    )
                })?;
                validate_sha256(
                    &format!("required corpus case '{}' referenceRgbaSha256", case.id),
                    sha,
                )?;
            }
            ExpectedOutcome::Error => {
                if case.reference_rgba_sha256.is_some() {
                    return Err(format!(
                        "required corpus error case '{}' must not carry a reference hash",
                        case.id
                    ));
                }
            }
        }
    }
    if public_cases != REQUIRED_PUBLIC_CASES || private_cases != REQUIRED_PRIVATE_CASES {
        return Err(format!(
            "required corpus must contain {REQUIRED_PUBLIC_CASES} public and \
             {REQUIRED_PRIVATE_CASES} private cases (found {public_cases} and \
             {private_cases})"
        ));
    }
    Ok(corpus)
}

fn audit_required_corpus(
    manifest: &Manifest,
    corpus: &RequiredCorpus,
) -> (RequirementsReport, Vec<Blocker>) {
    let declared: BTreeSet<&str> = manifest
        .required_coverage
        .iter()
        .map(String::as_str)
        .collect();
    let mut blockers = Vec::new();
    let mut missing_coverage = Vec::new();
    for tag in &corpus.required_coverage {
        if !declared.contains(tag.as_str()) {
            missing_coverage.push(tag.clone());
            blockers.push(Blocker {
                kind: "required-coverage-missing".to_string(),
                detail: format!("mandatory coverage tag '{tag}' is absent from the manifest"),
            });
        }
    }

    let mut missing_cases = Vec::new();
    let mut altered_cases = Vec::new();
    for required in &corpus.cases {
        let Some(case) = manifest.cases.iter().find(|case| case.id == required.id) else {
            missing_cases.push(required.id.clone());
            blockers.push(Blocker {
                kind: "required-case-missing".to_string(),
                detail: format!("mandatory corpus case '{}' is absent", required.id),
            });
            continue;
        };

        let mut altered_fields = Vec::new();
        if case.input_sha256 != required.input_sha256 {
            altered_fields.push("inputSha256");
        }
        if case.expected_outcome != required.expected_outcome {
            altered_fields.push("expectedOutcome");
        }
        if case.visibility != required.visibility {
            altered_fields.push("visibility");
        }
        let mut actual_coverage = case.coverage.clone();
        actual_coverage.sort();
        let mut expected_coverage = required.coverage.clone();
        expected_coverage.sort();
        if actual_coverage != expected_coverage {
            altered_fields.push("coverage");
        }
        if required.expected_outcome == ExpectedOutcome::Success {
            if case.reference_rgba_sha256.as_deref() != required.reference_rgba_sha256.as_deref() {
                altered_fields.push("referenceRgbaSha256");
            }
            if case.width != Some(ICON_SIZE) || case.height != Some(ICON_SIZE) {
                altered_fields.push("dimensions");
            }
        }

        if !altered_fields.is_empty() {
            altered_cases.push(required.id.clone());
            blockers.push(Blocker {
                kind: "required-case-mismatch".to_string(),
                detail: format!(
                    "mandatory corpus case '{}' differs in {}",
                    required.id,
                    altered_fields.join(", ")
                ),
            });
        }
    }

    let satisfied =
        missing_cases.is_empty() && altered_cases.is_empty() && missing_coverage.is_empty();
    (
        RequirementsReport {
            corpus_sha256: sha256_hex(REQUIRED_CORPUS_BYTES),
            required_coverage: corpus.required_coverage.clone(),
            required_cases: corpus.cases.len(),
            missing_cases,
            altered_cases,
            missing_coverage,
            satisfied,
        },
        blockers,
    )
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum OracleMode {
    Auto,
    Disabled,
}

#[derive(Debug)]
struct Oracle {
    report: OracleReport,
    python: Option<PathBuf>,
}

impl Oracle {
    fn usable(&self) -> bool {
        self.report.usable
    }
}

fn evaluate(
    manifest_display: &str,
    manifest_path: &Path,
    manifest: &Manifest,
    manifest_sha256: String,
    oracle_mode: OracleMode,
) -> Result<Report, String> {
    // Reject an already-stale executable before it can produce evidence.
    let candidate = current_candidate()?;
    let required_corpus = required_corpus()?;

    let base = manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let source_path = resolve_source_path(&base, &manifest.reference.source_path);
    let source_verified = match &source_path {
        Some(path) => {
            let digest = sha256_file(path).map_err(|error| {
                format!("cannot hash reference source {}: {error}", path.display())
            })?;
            if digest != manifest.reference.source_sha256 {
                return Err(format!(
                    "reference source {} does not match reference.sourceSha256",
                    manifest.reference.source_path
                ));
            }
            true
        }
        None => false,
    };

    let oracle = probe_oracle(manifest, &base, oracle_mode);
    let workdir = tempfile::Builder::new()
        .prefix("apk-lab-icon-preflight-")
        .tempdir()
        .map_err(|error| format!("cannot create preflight work directory: {error}"))?;
    let candidates = workdir.path().join("candidates");
    fs::create_dir(&candidates)
        .map_err(|error| format!("cannot create candidate directory: {error}"))?;

    let mut cases = Vec::with_capacity(manifest.cases.len());
    for case in &manifest.cases {
        cases.push(evaluate_case(
            case,
            &base,
            &candidates,
            oracle_mode,
            oracle.usable().then_some(&oracle),
            &manifest.decoded_inputs,
        )?);
    }

    let mut covered = BTreeSet::new();
    for case in &cases {
        if case.matched {
            for tag in &case.coverage {
                covered.insert(tag.as_str());
            }
        }
    }
    let covered_coverage: Vec<String> = manifest
        .required_coverage
        .iter()
        .filter(|tag| covered.contains(tag.as_str()))
        .cloned()
        .collect();
    let missing_coverage: Vec<String> = manifest
        .required_coverage
        .iter()
        .filter(|tag| !covered.contains(tag.as_str()))
        .cloned()
        .collect();

    let success_cases = cases
        .iter()
        .filter(|case| case.expected_outcome == ExpectedOutcome::Success)
        .count();
    let success_matched = cases
        .iter()
        .filter(|case| case.expected_outcome == ExpectedOutcome::Success && case.matched)
        .count();
    let error_matched = cases
        .iter()
        .filter(|case| case.expected_outcome == ExpectedOutcome::Error && case.matched)
        .count();
    let mismatched = cases.len() - success_matched - error_matched;
    let private_cases = cases
        .iter()
        .filter(|case| case.visibility == Visibility::Private)
        .count();
    let private_successes = cases
        .iter()
        .filter(|case| {
            case.visibility == Visibility::Private
                && case.expected_outcome == ExpectedOutcome::Success
                && case.matched
        })
        .count();

    let (requirements, requirement_blockers) = audit_required_corpus(manifest, &required_corpus);
    let mut blockers = Vec::new();
    for case in &cases {
        if !case.matched {
            blockers.push(Blocker {
                kind: "case-mismatch".to_string(),
                detail: format!("case '{}' did not match its expected outcome", case.id),
            });
        }
    }
    for tag in &missing_coverage {
        blockers.push(Blocker {
            kind: "missing-coverage".to_string(),
            detail: format!("required coverage '{tag}' has no matched case"),
        });
    }
    if private_successes == 0 {
        blockers.push(Blocker {
            kind: "missing-private-real-coverage".to_string(),
            detail: "full acceptance requires at least one matched successful private \
                     (real downloaded source) case"
                .to_string(),
        });
    }
    if !source_verified {
        blockers.push(Blocker {
            kind: "source-unverified".to_string(),
            detail: format!(
                "reference source '{}' could not be re-hashed on this host",
                manifest.reference.source_path
            ),
        });
    }
    if success_cases > 0 && !oracle.usable() {
        blockers.push(Blocker {
            kind: "oracle-unavailable".to_string(),
            detail: oracle
                .report
                .reason
                .clone()
                .unwrap_or_else(|| "pinned Pillow decoder is unavailable".to_string()),
        });
    }
    if cases.iter().any(|case| {
        case.expected_outcome == ExpectedOutcome::Success
            && case.encoder_roundtrip_matched == Some(false)
    }) {
        blockers.push(Blocker {
            kind: "encoder-roundtrip-mismatch".to_string(),
            detail: "the independently decoded candidate PNG differs from its reference"
                .to_string(),
        });
    }
    if cases.iter().any(|case| {
        case.expected_outcome == ExpectedOutcome::Success
            && case.reference_png_matched == Some(false)
    }) {
        blockers.push(Blocker {
            kind: "reference-png-mismatch".to_string(),
            detail: "the captured reference PNG does not decode to reference RGBA".to_string(),
        });
    }
    blockers.extend(requirement_blockers);

    let acceptance_passed = blockers.is_empty();
    Ok(Report {
        schema_version: REPORT_SCHEMA_VERSION,
        kind: REPORT_KIND.to_string(),
        generated_at: format_utc(unix_now()?),
        mode: REPORT_MODE.to_string(),
        manifest_path: manifest_display.to_string(),
        manifest_sha256,
        candidate,
        requirements,
        reference: ReferenceReport {
            python_version: manifest.reference.python_version.clone(),
            pillow_version: manifest.reference.pillow_version.clone(),
            source_sha256: manifest.reference.source_sha256.clone(),
            source_path: manifest.reference.source_path.clone(),
            source_verified,
            environment: manifest.reference.environment.clone(),
        },
        required_coverage: manifest.required_coverage.clone(),
        oracle: oracle.report,
        summary: Summary {
            total: cases.len(),
            matched: success_matched + error_matched,
            mismatched,
            success_matched,
            error_matched,
            private_cases,
            private_successes,
            covered_coverage,
            missing_coverage,
            acceptance_passed,
        },
        cases,
        blockers,
    })
}

/// Reads and validates a decoded reference fixture, returning the original
/// RGBA bytes and their digest. The digest always covers the decompressed
/// bytes, independent of the on-disk storage encoding.
fn read_decoded_input(
    spec: &DecodedInputSpec,
    base: &Path,
    case_id: &str,
) -> Result<(Vec<u8>, String), String> {
    let expected_len = decoded_input_expected_len(spec, case_id)?;
    let path = resolve_path(base, &spec.path);
    let bytes = match spec.encoding {
        DecodedInputEncoding::Raw => fs::read(&path).map_err(|error| {
            format!(
                "cannot read decoded input '{}' for case '{}': {error}",
                spec.path, case_id
            )
        })?,
        DecodedInputEncoding::Zlib => {
            let stored = spec.stored_sha256.as_deref().ok_or_else(|| {
                format!("decodedInputs.{case_id}.storedSha256 is required when encoding is 'zlib'")
            })?;
            let file = fs::File::open(&path).map_err(|error| {
                format!(
                    "cannot read decoded input '{}' for case '{}': {error}",
                    spec.path, case_id
                )
            })?;
            // The compressed fixture can never exceed the decompressed bound
            // by more than one byte; read no more than that into memory.
            let mut compressed = Vec::new();
            file.take(expected_len as u64 + 1)
                .read_to_end(&mut compressed)
                .map_err(|error| {
                    format!(
                        "cannot read compressed decoded input '{}' for case '{}': {error}",
                        spec.path, case_id
                    )
                })?;
            if compressed.len() > expected_len {
                return Err(format!(
                    "compressed decoded input for case '{}' exceeds {} bytes",
                    case_id, expected_len
                ));
            }
            let stored_digest = sha256_hex(&compressed);
            if stored_digest != stored {
                return Err(format!(
                    "stale or altered compressed decoded input for case '{}': storedSha256 mismatch",
                    case_id
                ));
            }
            let mut raw = Vec::new();
            ZlibDecoder::new(compressed.as_slice())
                .take(expected_len as u64 + 1)
                .read_to_end(&mut raw)
                .map_err(|error| {
                    format!(
                        "cannot inflate decoded input '{}' for case '{}': {error}",
                        spec.path, case_id
                    )
                })?;
            if raw.len() != expected_len {
                return Err(format!(
                    "decoded input for case '{}' must inflate to {} RGBA bytes (got {})",
                    case_id,
                    expected_len,
                    raw.len()
                ));
            }
            raw
        }
    };
    if bytes.len() != expected_len {
        return Err(format!(
            "decoded input for case '{}' must be {} RGBA bytes (got {})",
            case_id,
            expected_len,
            bytes.len()
        ));
    }
    let digest = sha256_hex(&bytes);
    if digest != spec.sha256 {
        return Err(format!(
            "stale or altered decoded input for case '{}': sha256 mismatch",
            case_id
        ));
    }
    Ok((bytes, digest))
}

fn decoded_input_expected_len(spec: &DecodedInputSpec, case_id: &str) -> Result<usize, String> {
    let expected_len = (spec.width as usize)
        .checked_mul(spec.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("decodedInputs.{case_id} dimensions overflow"))?;
    if expected_len > MAX_DECODED_INPUT_BYTES {
        return Err(format!(
            "decoded input for case '{case_id}' exceeds the {MAX_DECODED_INPUT_BYTES}-byte cap"
        ));
    }
    Ok(expected_len)
}

fn evaluate_case(
    case: &CaseSpec,
    base: &Path,
    candidates: &Path,
    oracle_mode: OracleMode,
    oracle: Option<&Oracle>,
    decoded_inputs: &BTreeMap<String, DecodedInputSpec>,
) -> Result<CaseReport, String> {
    let input_path = resolve_path(base, &case.input_path);
    let input_bytes = fs::read(&input_path).map_err(|error| {
        format!(
            "cannot read input '{}' for case '{}': {error}",
            case.input_path, case.id
        )
    })?;
    let input_sha256 = sha256_hex(&input_bytes);
    if input_sha256 != case.input_sha256 {
        return Err(format!(
            "stale or altered input for case '{}': inputSha256 does not match {}",
            case.id, case.input_path
        ));
    }

    let decoded = match decoded_inputs.get(&case.id) {
        Some(spec) => {
            let (_, digest) = read_decoded_input(spec, base, &case.id)?;
            Some(DecodedEvidence {
                sha256: digest,
                width: spec.width,
                height: spec.height,
                mode: spec.mode.clone(),
            })
        }
        None => None,
    };

    let mut reference_bytes = None;
    let mut reference_png_sha256 = None;
    let mut reference_png_path = None;
    if case.expected_outcome == ExpectedOutcome::Success {
        let rgba_rel = case
            .reference_rgba_path
            .as_deref()
            .ok_or_else(|| format!("case '{}' lacks referenceRgbaPath", case.id))?;
        let rgba_path = resolve_path(base, rgba_rel);
        let bytes = fs::read(&rgba_path).map_err(|error| {
            format!(
                "cannot read reference RGBA '{}' for case '{}': {error}",
                rgba_rel, case.id
            )
        })?;
        if bytes.len() != RGBA_LEN {
            return Err(format!(
                "reference RGBA for case '{}' must be exactly {RGBA_LEN} bytes (got {})",
                case.id,
                bytes.len()
            ));
        }
        let expected = case
            .reference_rgba_sha256
            .as_deref()
            .ok_or_else(|| format!("case '{}' lacks referenceRgbaSha256", case.id))?;
        let actual = sha256_hex(&bytes);
        if actual != expected {
            return Err(format!(
                "reference RGBA for case '{}' does not match referenceRgbaSha256",
                case.id
            ));
        }

        let png_rel = case
            .reference_png_path
            .as_deref()
            .ok_or_else(|| format!("case '{}' lacks referencePngPath", case.id))?;
        let png_path = resolve_path(base, png_rel);
        let png_bytes = fs::read(&png_path).map_err(|error| {
            format!(
                "cannot read reference PNG '{}' for case '{}': {error}",
                png_rel, case.id
            )
        })?;
        if !looks_like_png(&png_bytes) {
            return Err(format!(
                "referencePngPath for case '{}' is not a PNG file",
                case.id
            ));
        }
        reference_png_sha256 = Some(sha256_hex(&png_bytes));
        reference_png_path = Some(png_rel.to_string());
        reference_bytes = Some(bytes);
    }

    let base_report = |actual_outcome: &str| CaseReport {
        id: case.id.clone(),
        coverage: case.coverage.clone(),
        visibility: case.visibility,
        input_path: case.input_path.clone(),
        input_sha256: case.input_sha256.clone(),
        expected_outcome: case.expected_outcome,
        reference_rgba_path: case.reference_rgba_path.clone(),
        reference_rgba_sha256: case.reference_rgba_sha256.clone(),
        reference_png_path,
        reference_png_sha256,
        actual_outcome: actual_outcome.to_string(),
        width: None,
        height: None,
        candidate_rgba_sha256: None,
        candidate_png_sha256: None,
        candidate_png_size: None,
        input_decoded_rgba_sha256: decoded.as_ref().map(|item| item.sha256.clone()),
        input_decoded_width: decoded.as_ref().map(|item| item.width),
        input_decoded_height: decoded.as_ref().map(|item| item.height),
        input_decoded_mode: decoded.as_ref().map(|item| item.mode.clone()),
        oracle_decoded_rgba_sha256: None,
        encoder_roundtrip_matched: None,
        reference_png_decoded_sha256: None,
        reference_png_matched: None,
        matched: false,
        differing_bytes: None,
        differing_channels: None,
        first_differences: Vec::new(),
        error: None,
        write: None,
    };

    match (case.expected_outcome, transform_icon(&input_bytes)) {
        (ExpectedOutcome::Success, Ok(image)) => {
            if image.width != ICON_SIZE || image.height != ICON_SIZE {
                return Err(format!(
                    "asset-prep returned {}x{} for case '{}'; {}x{} is required",
                    image.width, image.height, case.id, ICON_SIZE, ICON_SIZE
                ));
            }
            if image.rgba.len() != RGBA_LEN {
                return Err(format!(
                    "asset-prep returned {} RGBA bytes for case '{}'; {RGBA_LEN} required",
                    image.rgba.len(),
                    case.id
                ));
            }
            let reference = reference_bytes
                .as_deref()
                .ok_or_else(|| format!("case '{}' lacks reference bytes", case.id))?;
            let diff = diff_rgba(reference, &image.rgba);

            let candidate_path = candidates.join(format!("{}.png", case.id));
            let first = write_icon_if_changed(&candidate_path, &image).map_err(|error| {
                format!("cannot write candidate for case '{}': {error}", case.id)
            })?;
            if first != IconWriteDecision::Created {
                return Err(format!(
                    "fresh candidate for case '{}' was not created (decision: {first:?})",
                    case.id
                ));
            }
            let first_bytes = fs::read(&candidate_path).map_err(|error| {
                format!("cannot re-read candidate for case '{}': {error}", case.id)
            })?;
            let candidate_png_sha256 = sha256_hex(&first_bytes);
            if candidate_png_sha256 != sha256_hex(&image.png) {
                return Err(format!(
                    "on-disk candidate PNG for case '{}' differs from asset-prep png bytes",
                    case.id
                ));
            }
            let first_mtime = fs::metadata(&candidate_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            let second = write_icon_if_changed(&candidate_path, &image).map_err(|error| {
                format!("cannot re-check candidate for case '{}': {error}", case.id)
            })?;
            if second != IconWriteDecision::Unchanged {
                return Err(format!(
                    "equal-pixel candidate for case '{}' was not preserved (decision: {second:?})",
                    case.id
                ));
            }
            let second_bytes = fs::read(&candidate_path).map_err(|error| {
                format!("cannot verify candidate for case '{}': {error}", case.id)
            })?;
            if first_bytes != second_bytes {
                return Err(format!(
                    "byte-preservation failed for equal-pixel candidate for case '{}'",
                    case.id
                ));
            }
            let second_mtime = fs::metadata(&candidate_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            if first_mtime != second_mtime {
                return Err(format!(
                    "mtime-preservation failed for equal-pixel candidate for case '{}'",
                    case.id
                ));
            }

            let mut matched = diff.is_empty();
            let mut report = base_report("success");
            report.width = Some(image.width);
            report.height = Some(image.height);
            report.candidate_rgba_sha256 = Some(sha256_hex(&image.rgba));
            report.candidate_png_sha256 = Some(candidate_png_sha256.clone());
            report.candidate_png_size = Some(first_bytes.len() as u64);
            report.differing_bytes = Some(diff.differing_bytes);
            report.differing_channels = Some(diff.channels);
            report.first_differences = diff.first_differences;

            if let Some(oracle) = oracle {
                if report.reference_png_path.is_some() {
                    let reference_png = resolve_path(
                        base,
                        report.reference_png_path.as_deref().unwrap_or_default(),
                    );
                    let decoded =
                        decode_png_with_oracle(oracle, &reference_png).map_err(|error| {
                            format!(
                                "cannot independently decode reference PNG for case '{}': {error}",
                                case.id
                            )
                        })?;
                    let decoded_sha = sha256_hex(&decoded);
                    report.reference_png_decoded_sha256 = Some(decoded_sha.clone());
                    if decoded_sha != case.reference_rgba_sha256.clone().unwrap_or_default() {
                        return Err(format!(
                            "reference PNG for case '{}' does not decode to its reference RGBA",
                            case.id
                        ));
                    }
                    report.reference_png_matched = Some(true);
                }

                let decoded = decode_png_with_oracle(oracle, &candidate_path).map_err(|error| {
                    format!(
                        "cannot independently decode candidate PNG for case '{}': {error}",
                        case.id
                    )
                })?;
                let decoded_sha = sha256_hex(&decoded);
                let roundtrip = decoded == reference;
                report.oracle_decoded_rgba_sha256 = Some(decoded_sha);
                report.encoder_roundtrip_matched = Some(roundtrip);
                matched &= roundtrip;
            } else if oracle_mode == OracleMode::Auto {
                // Auto mode with no usable decoder is surfaced by the oracle
                // blocker; Disabled mode is only used by deterministic unit
                // tests that cannot invoke Python.
                report.encoder_roundtrip_matched = None;
            }

            // Stronger write evidence: replace the existing file with a
            // byte-different PNG encoding of the exact same pixels and verify
            // the unchanged predicate still preserves those bytes and mtime.
            let alternate_bytes = alternate_png_encoding(&image.rgba)?;
            let alternate_encoding_sha256 = sha256_hex(&alternate_bytes);
            if alternate_encoding_sha256 == candidate_png_sha256 {
                return Err(format!(
                    "alternate encoding was byte-identical for case '{}'",
                    case.id
                ));
            }
            fs::write(&candidate_path, &alternate_bytes).map_err(|error| {
                format!(
                    "cannot install alternate encoding for case '{}': {error}",
                    case.id
                )
            })?;
            let alternate_mtime = fs::metadata(&candidate_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            let alternate_decision =
                write_icon_if_changed(&candidate_path, &image).map_err(|error| {
                    format!(
                        "cannot evaluate alternate encoding for case '{}': {error}",
                        case.id
                    )
                })?;
            if alternate_decision != IconWriteDecision::Unchanged {
                return Err(format!(
                    "different-encoding equal-pixel candidate for case '{}' was not \
                     preserved (decision: {alternate_decision:?})",
                    case.id
                ));
            }
            let alternate_after = fs::read(&candidate_path).map_err(|error| {
                format!(
                    "cannot verify alternate encoding for case '{}': {error}",
                    case.id
                )
            })?;
            if alternate_after != alternate_bytes {
                return Err(format!(
                    "byte-preservation failed across different PNG encodings for case '{}'",
                    case.id
                ));
            }
            let alternate_mtime_after = fs::metadata(&candidate_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            if alternate_mtime != alternate_mtime_after {
                return Err(format!(
                    "mtime-preservation failed across different PNG encodings for case '{}'",
                    case.id
                ));
            }

            report.write = Some(WriteEvidence {
                first: "created".to_string(),
                second: "unchanged".to_string(),
                candidate_sha256: candidate_png_sha256,
                preserved: true,
                alternate_encoding_sha256,
                alternate_encoding_preserved: true,
                alternate_mtime_preserved: true,
                alternate_encoding_note: "existing file replaced with a different \
                                           compression encoding of the same RGBA; \
                                           write decision returned unchanged and \
                                           preserved bytes/mtime"
                    .to_string(),
            });

            report.matched = matched;
            Ok(report)
        }
        (ExpectedOutcome::Success, Err(error)) => {
            let mut report = base_report("error");
            report.error = Some(format_error(&error.to_string()));
            Ok(report)
        }
        (ExpectedOutcome::Error, Err(error)) => {
            let mut report = base_report("error");
            report.error = Some(format_error(&error.to_string()));
            report.matched = true;
            Ok(report)
        }
        (ExpectedOutcome::Error, Ok(image)) => {
            let mut report = base_report("success");
            report.width = Some(image.width);
            report.height = Some(image.height);
            report.candidate_rgba_sha256 = Some(sha256_hex(&image.rgba));
            report.candidate_png_sha256 = Some(sha256_hex(&image.png));
            Ok(report)
        }
    }
}

fn format_error(message: &str) -> String {
    message
        .replace(['\n', '\r'], " ")
        .chars()
        .take(MAX_ERROR_CHARS)
        .collect()
}

fn looks_like_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

/// Encodes the same RGBA pixels with a different compression level, producing
/// a byte-different but visually identical PNG. Used only as write-decision
/// evidence; it never replaces the candidate transform output.
fn alternate_png_encoding(rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, ICON_SIZE, ICON_SIZE);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("cannot encode alternate candidate PNG: {error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("cannot encode alternate candidate PNG: {error}"))?;
        writer
            .finish()
            .map_err(|error| format!("cannot encode alternate candidate PNG: {error}"))?;
    }
    Ok(out)
}

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn resolve_source_path(base: &Path, value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let direct = base.join(path);
    if direct.is_file() {
        return Some(direct);
    }
    let mut cursor = Some(base.to_path_buf());
    while let Some(directory) = cursor {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Some(candidate);
        }
        cursor = directory.parent().map(Path::to_path_buf);
    }
    let cwd = env::current_dir().ok()?;
    let mut cursor = Some(cwd);
    while let Some(directory) = cursor {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Some(candidate);
        }
        cursor = directory.parent().map(Path::to_path_buf);
    }
    None
}

// ---------------------------------------------------------------------------
// Image comparison
// ---------------------------------------------------------------------------

fn diff_rgba(reference: &[u8], candidate: &[u8]) -> DiffStats {
    let length = reference.len().min(candidate.len());
    let mut stats = DiffStats::default();
    for index in 0..length {
        if reference[index] != candidate[index] {
            stats.differing_bytes += 1;
            let channel_index = index % 4;
            let channel = match channel_index {
                0 => "r",
                1 => "g",
                2 => "b",
                _ => "a",
            };
            match channel_index {
                0 => stats.channels.r += 1,
                1 => stats.channels.g += 1,
                2 => stats.channels.b += 1,
                _ => stats.channels.a += 1,
            }
            if stats.first_differences.len() < MAX_FIRST_DIFFERENCES {
                let pixel_index = index / 4;
                stats.first_differences.push(Difference {
                    byte_index: index,
                    pixel_index,
                    x: (pixel_index % ICON_SIZE as usize) as u32,
                    y: (pixel_index / ICON_SIZE as usize) as u32,
                    channel: channel.to_string(),
                    reference: reference[index],
                    candidate: candidate[index],
                });
            }
        }
    }
    stats.differing_bytes += (reference.len().max(candidate.len()) - length) as u64;
    stats
}

// ---------------------------------------------------------------------------
// Independent Pillow decoder
// ---------------------------------------------------------------------------

fn probe_oracle(manifest: &Manifest, base: &Path, mode: OracleMode) -> Oracle {
    let mut report = OracleReport {
        required_python_version: manifest.reference.python_version.clone(),
        required_pillow_version: manifest.reference.pillow_version.clone(),
        decoder: "<pinned-pillow-oracle>".to_string(),
        available: false,
        usable: false,
        python_version: None,
        pillow_version: None,
        reason: None,
    };
    if mode == OracleMode::Disabled {
        report.reason = Some("oracle decoding disabled by this invocation".to_string());
        return Oracle {
            report,
            python: None,
        };
    }

    let python = match locate_oracle(base) {
        Some(path) => path,
        None => {
            report.reason = Some(format!(
                "no pinned Pillow oracle found; set {ORACLE_ENV} to the isolated Python"
            ));
            return Oracle {
                report,
                python: None,
            };
        }
    };
    report.available = true;

    let probe = "import json,sys\n\
                 import PIL\n\
                 print(json.dumps({\n\
                 \x20 'implementation': sys.implementation.name,\n\
                 \x20 'python': '%d.%d.%d' % sys.version_info[:3],\n\
                 \x20 'pillow': PIL.__version__,\n\
                 }))";
    let output = Command::new(&python)
        .args(["-B", "-c", probe])
        .env_clear()
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            report.reason = Some(format!(
                "pinned oracle probe failed: {}",
                format_error(&String::from_utf8_lossy(&output.stderr))
            ));
            return Oracle {
                report,
                python: None,
            };
        }
        Err(error) => {
            report.reason = Some(format!("cannot execute pinned oracle: {error}"));
            return Oracle {
                report,
                python: None,
            };
        }
    };
    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            report.reason = Some(format!("invalid pinned oracle probe output: {error}"));
            return Oracle {
                report,
                python: None,
            };
        }
    };
    report.python_version = value
        .get("python")
        .and_then(Value::as_str)
        .map(str::to_string);
    report.pillow_version = value
        .get("pillow")
        .and_then(Value::as_str)
        .map(str::to_string);

    if report.python_version.as_deref() != Some(manifest.reference.python_version.as_str()) {
        report.reason = Some(format!(
            "pinned oracle Python version {:?} does not match reference {}",
            report.python_version, manifest.reference.python_version
        ));
        return Oracle {
            report,
            python: None,
        };
    }
    if report.pillow_version.as_deref() != Some(manifest.reference.pillow_version.as_str()) {
        report.reason = Some(format!(
            "pinned oracle Pillow version {:?} does not match reference {}",
            report.pillow_version, manifest.reference.pillow_version
        ));
        return Oracle {
            report,
            python: None,
        };
    }

    report.usable = true;
    Oracle {
        report,
        python: Some(python),
    }
}

fn locate_oracle(base: &Path) -> Option<PathBuf> {
    if let Some(value) = env::var_os(ORACLE_ENV) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }

    let relative = Path::new(".apk-lab-migration/icons-reference/.venv/bin/python");
    let mut candidates = Vec::new();
    let mut cursor = Some(base.to_path_buf());
    while let Some(directory) = cursor {
        candidates.push(directory.join(relative));
        candidates.push(directory.join("tools/apk-lab/.venv/bin/python"));
        cursor = directory.parent().map(Path::to_path_buf);
    }
    if let Ok(cwd) = env::current_dir() {
        let mut cursor = Some(cwd);
        while let Some(directory) = cursor {
            candidates.push(directory.join(relative));
            candidates.push(directory.join("tools/apk-lab/.venv/bin/python"));
            cursor = directory.parent().map(Path::to_path_buf);
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn decode_png_with_oracle(oracle: &Oracle, path: &Path) -> Result<Vec<u8>, String> {
    let python = oracle
        .python
        .as_ref()
        .ok_or_else(|| "pinned oracle is unavailable".to_string())?;
    let script = "import io,sys\n\
                  from PIL import Image\n\
                  with open(sys.argv[1], 'rb') as handle:\n\
                  \x20   data = handle.read()\n\
                  image = Image.open(io.BytesIO(data))\n\
                  image = image.convert('RGBA')\n\
                  if image.size != (128, 128):\n\
                  \x20   raise SystemExit('unexpected size %r' % (image.size,))\n\
                  payload = image.tobytes()\n\
                  if len(payload) != 128 * 128 * 4:\n\
                  \x20   raise SystemExit('unexpected byte count %d' % len(payload))\n\
                  sys.stdout.buffer.write(payload)";
    let output = Command::new(python)
        .args(["-B", "-c", script])
        .arg(path)
        .env_clear()
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot execute pinned oracle: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "pinned oracle decode failed: {}",
            format_error(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    if output.stdout.len() != RGBA_LEN {
        return Err(format!(
            "pinned oracle returned {} RGBA bytes; {RGBA_LEN} required",
            output.stdout.len()
        ));
    }
    Ok(output.stdout)
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn write_report(report: &Report, out: Option<&Path>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("cannot serialize icon preflight report: {error}"))?;
    match out {
        None => {
            println!("{json}");
            Ok(())
        }
        Some(path) if path == Path::new("-") => {
            println!("{json}");
            Ok(())
        }
        Some(path) => write_json_text(&json, path),
    }
}

fn write_json_text(json: &str, path: &Path) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create report directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut file = fs::File::create(path)
        .map_err(|error| format!("cannot write report {}: {error}", path.display()))?;
    file.write_all(json.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| format!("cannot write report {}: {error}", path.display()))
}

fn print_summary(report: &Report) {
    println!(
        "icon preflight: {} case(s), {} matched, {} mismatched, {} required tags, \
         acceptance={}",
        report.summary.total,
        report.summary.matched,
        report.summary.mismatched,
        report.required_coverage.len(),
        report.summary.acceptance_passed
    );
    for blocker in &report.blockers {
        println!("  blocker [{}]: {}", blocker.kind, blocker.detail);
    }
}

fn blocker_summary(blockers: &[Blocker]) -> String {
    blockers
        .iter()
        .map(|blocker| format!("{}: {}", blocker.kind, blocker.detail))
        .collect::<Vec<_>>()
        .join("; ")
}

fn first_json_difference(report: &Value, fresh: &Value, path: &str) -> Option<String> {
    match (report, fresh) {
        (Value::Object(report), Value::Object(fresh)) => {
            for (key, value) in report {
                let child = format!("{path}.{key}");
                match fresh.get(key) {
                    Some(other) => {
                        if let Some(difference) = first_json_difference(value, other, &child) {
                            return Some(difference);
                        }
                    }
                    None => return Some(format!("{child}: missing from fresh evidence")),
                }
            }
            for key in fresh.keys() {
                if !report.contains_key(key) {
                    return Some(format!(
                        "{path}.{key}: unexpected in report, present in fresh evidence"
                    ));
                }
            }
            None
        }
        (Value::Array(report), Value::Array(fresh)) => {
            if report.len() != fresh.len() {
                return Some(format!(
                    "{path}: report has {} entries, fresh evidence has {}",
                    report.len(),
                    fresh.len()
                ));
            }
            for (index, (value, other)) in report.iter().zip(fresh).enumerate() {
                if let Some(difference) =
                    first_json_difference(value, other, &format!("{path}[{index}]"))
                {
                    return Some(difference);
                }
            }
            None
        }
        _ if report != fresh => Some(format!(
            "{path}: report={}, fresh={}",
            compact_value(report),
            compact_value(fresh)
        )),
        _ => None,
    }
}

fn compact_value(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 200 {
        let mut truncated: String = text.chars().take(200).collect();
        truncated.push('…');
        truncated
    } else {
        text
    }
}

// ---------------------------------------------------------------------------
// Hashing and time helpers
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_bytes(&hasher.finalize())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_bytes(&hasher.finalize()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

// ---------------------------------------------------------------------------
// Native candidate freshness
// ---------------------------------------------------------------------------

fn repo_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve the repository root from CARGO_MANIFEST_DIR".to_string())
}

fn update_fingerprint(hasher: &mut Sha256, path: &str, bytes: &[u8]) {
    hasher.update((path.len() as u64).to_be_bytes());
    hasher.update(path.as_bytes());
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn compiled_source_fingerprint() -> String {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_FINGERPRINT_DOMAIN);
    for source in COMPILED_SOURCES {
        update_fingerprint(&mut hasher, source.path, source.bytes);
    }
    hex_bytes(&hasher.finalize())
}

/// Reads every embedded candidate source from `root` and refuses to proceed
/// when any byte differs from what this executable was compiled with.
fn verify_compiled_sources(root: &Path) -> Result<(String, Vec<SourceFileReport>), String> {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_FINGERPRINT_DOMAIN);
    let mut records = Vec::with_capacity(COMPILED_SOURCES.len());
    let mut stale = Vec::new();
    for source in COMPILED_SOURCES {
        let path = root.join(source.path);
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "cannot read native candidate source {}: {error}",
                path.display()
            )
        })?;
        if bytes != source.bytes {
            stale.push(source.path);
            continue;
        }
        update_fingerprint(&mut hasher, source.path, &bytes);
        records.push(SourceFileReport {
            path: source.path.to_string(),
            sha256: sha256_hex(&bytes),
        });
    }
    if !stale.is_empty() {
        return Err(format!(
            "compiled icon candidate is stale: embedded source bytes differ from the \
             checkout for {}; rebuild apk-lab-migrate before capturing or checking \
             evidence",
            stale.join(", ")
        ));
    }
    Ok((hex_bytes(&hasher.finalize()), records))
}

fn current_candidate() -> Result<CandidateReport, String> {
    let root = repo_root()?;
    let (source_fingerprint, sources) = verify_compiled_sources(&root)?;
    let embedded = compiled_source_fingerprint();
    if embedded != source_fingerprint {
        return Err(
            "native candidate fingerprint mismatch after source verification; refusing to \
             produce evidence"
                .to_string(),
        );
    }
    let executable = env::current_exe()
        .map_err(|error| format!("cannot locate the executing binary: {error}"))?;
    let binary_sha256 = sha256_file(&executable).map_err(|error| {
        format!(
            "cannot hash executing binary {}: {error}",
            executable.display()
        )
    })?;
    let binary_file_name = executable
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unknown>".to_string());
    Ok(CandidateReport {
        source_fingerprint,
        binary_sha256,
        binary_file_name,
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        sources,
    })
}

fn unix_now() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| "system clock is before the Unix epoch".to_string())
}

fn parse_utc(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year = parse_digits(bytes, 0, 4)?;
    let month = parse_digits(bytes, 5, 2)? as u32;
    let day = parse_digits(bytes, 8, 2)? as u32;
    let hour = parse_digits(bytes, 11, 2)?;
    let minute = parse_digits(bytes, 14, 2)?;
    let second = parse_digits(bytes, 17, 2)?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn parse_digits(bytes: &[u8], start: usize, length: usize) -> Option<i64> {
    let mut value = 0i64;
    for byte in &bytes[start..start + length] {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(byte - b'0');
    }
    Some(value)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = (if days >= 0 { days } else { days - 146_096 }) / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        (month_prime + 3) as u32
    } else {
        (month_prime - 9) as u32
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = remainder / 3_600;
    let minute = (remainder % 3_600) / 60;
    let second = remainder % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 1x1 RGBA PNG generated once by the pinned Pillow oracle. Tests only use
    // it as deterministic native input; no Python or network is required.
    const TINY_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 16, 84, 50, 118, 1,
        0, 1, 89, 0, 171, 19, 42, 37, 171, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    struct Fixture {
        dir: tempfile::TempDir,
        manifest_path: PathBuf,
        manifest: Manifest,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .prefix("apk-lab-icons-test-")
                .tempdir()
                .unwrap();
            fs::write(dir.path().join("source.py"), "# capture source\n").unwrap();
            let source_sha = sha256_hex(b"# capture source\n");
            fs::write(dir.path().join("input.png"), TINY_PNG).unwrap();
            let image = transform_icon(TINY_PNG).unwrap();
            fs::write(dir.path().join("reference.rgba"), &image.rgba).unwrap();
            fs::write(dir.path().join("reference.png"), &image.png).unwrap();

            let mut environment = BTreeMap::new();
            environment.insert("pythonImplementation".to_string(), json!("cpython"));
            environment.insert("pythonVersion".to_string(), json!("3.12.14"));
            environment.insert("pillowVersion".to_string(), json!("11.3.0"));
            environment.insert("platform".to_string(), json!("Darwin"));
            environment.insert("machine".to_string(), json!("arm64"));
            environment.insert("architecture".to_string(), json!("64bit"));

            let manifest = Manifest {
                schema_version: 1,
                required_coverage: vec!["format:png".to_string()],
                reference: ReferenceSpec {
                    python_version: "3.12.14".to_string(),
                    pillow_version: "11.3.0".to_string(),
                    source_sha256: source_sha.clone(),
                    source_path: "source.py".to_string(),
                    environment,
                },
                cases: vec![CaseSpec {
                    id: "tiny-png".to_string(),
                    coverage: vec!["format:png".to_string()],
                    visibility: Visibility::Public,
                    input_path: "input.png".to_string(),
                    input_sha256: sha256_hex(TINY_PNG),
                    expected_outcome: ExpectedOutcome::Success,
                    reference_rgba_path: Some("reference.rgba".to_string()),
                    reference_rgba_sha256: Some(sha256_hex(&image.rgba)),
                    reference_png_path: Some("reference.png".to_string()),
                    width: Some(128),
                    height: Some(128),
                }],
                decoded_inputs: BTreeMap::new(),
            };
            let manifest_json = serde_json::to_vec_pretty(&json!({
                "schemaVersion": 1,
                "requiredCoverage": ["format:png"],
                "reference": {
                    "pythonVersion": "3.12.14",
                    "pillowVersion": "11.3.0",
                    "sourceSha256": source_sha,
                    "sourcePath": "source.py",
                    "environment": {
                        "pythonImplementation": "cpython",
                        "pythonVersion": "3.12.14",
                        "pillowVersion": "11.3.0",
                        "platform": "Darwin",
                        "machine": "arm64",
                        "architecture": "64bit"
                    }
                },
                "cases": [{
                    "id": "tiny-png",
                    "coverage": ["format:png"],
                    "visibility": "public",
                    "inputPath": "input.png",
                    "inputSha256": sha256_hex(TINY_PNG),
                    "expectedOutcome": "success",
                    "referenceRgbaPath": "reference.rgba",
                    "referenceRgbaSha256": sha256_hex(&image.rgba),
                    "referencePngPath": "reference.png",
                    "width": 128,
                    "height": 128
                }]
            }))
            .unwrap();
            let manifest_path = dir.path().join("manifest.json");
            fs::write(&manifest_path, &manifest_json).unwrap();
            validate_manifest(&manifest).unwrap();
            Fixture {
                dir,
                manifest_path,
                manifest,
            }
        }

        fn report(&self) -> Report {
            evaluate(
                self.manifest_path.to_str().unwrap(),
                &self.manifest_path,
                &self.manifest,
                sha256_hex(&fs::read(&self.manifest_path).unwrap()),
                OracleMode::Disabled,
            )
            .unwrap()
        }
    }

    #[test]
    fn diff_counts_hidden_rgb_bytes_exactly() {
        let mut reference = vec![0u8; RGBA_LEN];
        let mut candidate = vec![0u8; RGBA_LEN];
        for pixel in 0..(ICON_SIZE * ICON_SIZE) as usize {
            let offset = pixel * 4;
            reference[offset] = (pixel % 251) as u8;
            candidate[offset] = (pixel % 251) as u8;
            // Alpha remains zero: hidden RGB must still be compared.
            reference[offset + 1] = 7;
            candidate[offset + 1] = 7;
        }
        candidate[4 * 3 + 1] = 9;
        let diff = diff_rgba(&reference, &candidate);
        assert_eq!(diff.differing_bytes, 1);
        assert_eq!(diff.channels.r, 0);
        assert_eq!(diff.channels.g, 1);
        assert_eq!(diff.channels.b, 0);
        assert_eq!(diff.channels.a, 0);
        assert_eq!(diff.first_differences[0].pixel_index, 3);
        assert_eq!(diff.first_differences[0].channel, "g");
    }

    #[test]
    fn help_and_strict_arguments() {
        assert!(matches!(
            parse_args(&["--help".to_string()]).unwrap(),
            Invocation::Help
        ));
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--check".to_string()]).is_err());
        assert!(parse_args(&[
            "--manifest".to_string(),
            "m".to_string(),
            "--out".to_string(),
            "o".to_string(),
            "--report".to_string(),
            "r".to_string(),
        ])
        .is_err());
        assert!(parse_args(&[
            "--manifest".to_string(),
            "m".to_string(),
            "--manifest".to_string(),
            "n".to_string(),
            "--out".to_string(),
            "o".to_string(),
        ])
        .is_err());
        assert!(matches!(
            parse_args(&[
                "--check".to_string(),
                "--report".to_string(),
                "r".to_string()
            ])
            .unwrap(),
            Invocation::Check { .. }
        ));
        assert!(matches!(
            parse_args(&[
                "--sanitize".to_string(),
                "--input".to_string(),
                "raw.json".to_string(),
                "--out".to_string(),
                "public.json".to_string(),
            ])
            .unwrap(),
            Invocation::Sanitize { .. }
        ));
        assert!(parse_args(&[
            "--sanitize".to_string(),
            "--input".to_string(),
            "raw.json".to_string(),
            "--out".to_string(),
            "public.json".to_string(),
            "--check".to_string(),
            "--report".to_string(),
            "r".to_string(),
        ])
        .is_err());
        assert!(parse_args(&[
            "--sanitize".to_string(),
            "--input".to_string(),
            "raw.json".to_string(),
            "--out".to_string(),
            "public.json".to_string(),
            "--manifest".to_string(),
            "m.json".to_string(),
        ])
        .is_err());
        assert!(parse_args(&["--input".to_string(), "raw.json".to_string()]).is_err());
    }

    #[test]
    fn candidate_identity_records_binary_source_and_platform() {
        let report = Fixture::new().report();
        assert_eq!(report.candidate.os, env::consts::OS);
        assert_eq!(report.candidate.arch, env::consts::ARCH);
        assert_eq!(
            report.candidate.source_fingerprint,
            compiled_source_fingerprint()
        );
        assert_eq!(report.candidate.sources.len(), COMPILED_SOURCES.len());
        for source in &report.candidate.sources {
            assert_eq!(source.sha256.len(), 64);
        }
    }

    #[test]
    fn candidate_source_and_lock_mutations_are_rejected_against_old_binary() {
        let root = repo_root().unwrap();
        let (compiled_fingerprint, _) = verify_compiled_sources(&root).unwrap();
        assert_eq!(compiled_fingerprint, compiled_source_fingerprint());

        let copy = tempfile::Builder::new()
            .prefix("apk-lab-candidate-copy-")
            .tempdir()
            .unwrap();
        for source in COMPILED_SOURCES {
            let destination = copy.path().join(source.path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(&destination, fs::read(root.join(source.path)).unwrap()).unwrap();
        }
        assert_eq!(
            verify_compiled_sources(copy.path()).unwrap().0,
            compiled_fingerprint
        );

        for target in [
            "tools/asset-prep/src/icons.rs",
            "tools/apk-lab/src/migration/icons.rs",
            "tools/asset-prep/Cargo.toml",
            "tools/asset-prep/Cargo.lock",
            "tools/apk-lab/Cargo.lock",
        ] {
            let path = copy.path().join(target);
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(&path, &bytes).unwrap();
            let error = verify_compiled_sources(copy.path()).unwrap_err();
            assert!(
                error.contains("is stale") && error.contains(target),
                "unexpected error for {target}: {error}"
            );
            bytes.pop();
            fs::write(&path, bytes).unwrap();
        }
    }

    #[test]
    fn report_candidate_identity_mutations_are_rejected() {
        let fixture = Fixture::new();
        let report = fixture.report();

        let mut altered = report.clone();
        altered.candidate.source_fingerprint = "0".repeat(64);
        assert!(
            verify_report(&altered, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("native candidate source fingerprint")
        );

        let mut altered = report;
        altered.candidate.binary_sha256 = "0".repeat(64);
        assert!(
            verify_report(&altered, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("binarySha256")
        );
    }

    #[test]
    fn capture_publishes_report_and_fails_when_acceptance_is_blocked() {
        let fixture = Fixture::new();
        let out = fixture.dir.path().join("capture.json");
        let error = capture_command_with_mode(&fixture.manifest_path, &out, OracleMode::Disabled)
            .unwrap_err();
        assert!(error.contains("did not pass acceptance"), "{error}");

        let report: Report = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        assert_eq!(report.summary.mismatched, 0);
        assert!(!report.summary.acceptance_passed);
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker.kind == "missing-private-real-coverage"));
    }

    #[test]
    fn write_evidence_covers_a_different_png_encoding() {
        let report = Fixture::new().report();
        let write = report.cases[0].write.as_ref().unwrap();
        assert!(write.preserved);
        assert!(write.alternate_encoding_preserved);
        assert!(write.alternate_mtime_preserved);
        assert_ne!(write.alternate_encoding_sha256, write.candidate_sha256);
        assert!(write
            .alternate_encoding_note
            .contains("different compression"));
    }

    #[test]
    fn sanitized_public_summary_preserves_measurements_and_refuses_replay() {
        let fixture = Fixture::new();
        let mut report = fixture.report();
        report.manifest_path = "/Users/example/private/manifest.json".to_string();
        report.cases[0].visibility = Visibility::Private;
        report.cases[0].input_path = "/Users/example/private/input.png".to_string();

        let raw = fixture.dir.path().join("raw-report.json");
        fs::write(&raw, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        let public = fixture.dir.path().join("public-report.json");
        sanitize_command(&raw, &public).unwrap();

        let text = fs::read_to_string(&public).unwrap();
        assert!(!text.contains("/Users/"));
        assert!(!text.contains("private/input.png"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], PUBLIC_REPORT_KIND);
        assert_eq!(value["mode"], PUBLIC_REPORT_MODE);
        assert_eq!(value["replayable"], false);
        assert_eq!(value["checkSupported"], false);
        assert_eq!(
            value["cases"][0]["inputPath"],
            format!("<private:input:{}>", report.cases[0].id)
        );
        assert_eq!(
            value["cases"][0]["candidateRgbaSha256"],
            serde_json::to_value(&report.cases[0].candidate_rgba_sha256).unwrap()
        );
        assert_eq!(
            value["summary"],
            serde_json::to_value(&report.summary).unwrap()
        );
        assert_eq!(
            value["candidate"]["sourceFingerprint"],
            serde_json::to_value(&report.candidate.source_fingerprint).unwrap()
        );

        assert!(check_command(&public, None)
            .unwrap_err()
            .contains("sanitized public summary"));
    }

    #[test]
    fn sanitizer_leak_check_refuses_absolute_paths() {
        let error = assert_no_private_leak(r#"{"path":"/Users/example/secret"}"#, &[]).unwrap_err();
        assert!(
            error.contains("leaked") || error.contains("absolute path"),
            "{error}"
        );
    }

    #[test]
    fn compiled_required_corpus_has_all_frozen_cases() {
        let corpus = required_corpus().unwrap();
        let public = corpus
            .cases
            .iter()
            .filter(|case| case.visibility == Visibility::Public)
            .count();
        let private = corpus
            .cases
            .iter()
            .filter(|case| case.visibility == Visibility::Private)
            .count();
        assert_eq!(public, 33);
        assert_eq!(private, 3);
        assert_eq!(corpus.cases.len(), 36);
        assert!(corpus.cases.iter().any(|case| case.id == "rgb-opaque-png"));
        assert!(corpus
            .cases
            .iter()
            .any(|case| case.id == "play-adobe-reader"));
        assert!(corpus
            .required_coverage
            .iter()
            .any(|tag| tag == "source:play-original"));
        assert_eq!(
            sha256_hex(REQUIRED_CORPUS_BYTES),
            "970e1f1d3f83ce70acb67639465eb82a5062e40a0b263f44b564cd25d922e8b9"
        );
    }

    #[test]
    fn reduced_manifest_is_blocked_even_when_its_cases_match() {
        let report = Fixture::new().report();
        assert_eq!(report.summary.mismatched, 0);
        assert!(!report.summary.acceptance_passed);
        assert!(!report.requirements.satisfied);
        assert!(report
            .requirements
            .missing_cases
            .iter()
            .any(|id| id == "rgb-opaque-png"));
        assert!(report
            .requirements
            .missing_coverage
            .iter()
            .any(|tag| tag == "alpha:binary-edge"));
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker.kind == "required-case-missing"));
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker.kind == "required-coverage-missing"));
    }

    #[test]
    fn complete_manifest_satisfies_corpus_and_alterations_are_blocked() {
        let corpus = required_corpus().unwrap();
        let mut manifest = Fixture::new().manifest;
        manifest.required_coverage = corpus.required_coverage.clone();
        manifest.cases = corpus
            .cases
            .iter()
            .map(|required| {
                let success = required.expected_outcome == ExpectedOutcome::Success;
                CaseSpec {
                    id: required.id.clone(),
                    coverage: required.coverage.clone(),
                    visibility: required.visibility,
                    input_path: format!("inputs/{}.bin", required.id),
                    input_sha256: required.input_sha256.clone(),
                    expected_outcome: required.expected_outcome,
                    reference_rgba_path: success.then(|| format!("reference/{}.rgba", required.id)),
                    reference_rgba_sha256: required.reference_rgba_sha256.clone(),
                    reference_png_path: success.then(|| format!("reference/{}.png", required.id)),
                    width: success.then_some(ICON_SIZE),
                    height: success.then_some(ICON_SIZE),
                }
            })
            .collect();

        let (report, blockers) = audit_required_corpus(&manifest, &corpus);
        assert!(blockers.is_empty(), "{blockers:?}");
        assert!(report.satisfied);
        assert_eq!(report.required_cases, 36);

        manifest.cases[0].input_sha256 = "0".repeat(64);
        let (report, blockers) = audit_required_corpus(&manifest, &corpus);
        assert!(!report.satisfied);
        assert!(report.altered_cases.contains(&manifest.cases[0].id));
        assert!(blockers
            .iter()
            .any(|blocker| blocker.kind == "required-case-mismatch"));

        manifest.cases[0].input_sha256 = corpus.cases[0].input_sha256.clone();
        manifest.cases.pop();
        let (report, blockers) = audit_required_corpus(&manifest, &corpus);
        assert!(!report.satisfied);
        assert!(blockers
            .iter()
            .any(|blocker| blocker.kind == "required-case-missing"));

        let mut manifest = Fixture::new().manifest;
        manifest.required_coverage = corpus.required_coverage.clone();
        manifest.cases = corpus
            .cases
            .iter()
            .map(|required| {
                let success = required.expected_outcome == ExpectedOutcome::Success;
                CaseSpec {
                    id: required.id.clone(),
                    coverage: required.coverage.clone(),
                    visibility: required.visibility,
                    input_path: format!("inputs/{}.bin", required.id),
                    input_sha256: required.input_sha256.clone(),
                    expected_outcome: required.expected_outcome,
                    reference_rgba_path: success.then(|| format!("reference/{}.rgba", required.id)),
                    reference_rgba_sha256: required.reference_rgba_sha256.clone(),
                    reference_png_path: success.then(|| format!("reference/{}.png", required.id)),
                    width: success.then_some(ICON_SIZE),
                    height: success.then_some(ICON_SIZE),
                }
            })
            .collect();
        manifest
            .required_coverage
            .retain(|tag| tag != "alpha:binary-edge");
        let (report, blockers) = audit_required_corpus(&manifest, &corpus);
        assert!(!report.satisfied);
        assert!(report
            .missing_coverage
            .iter()
            .any(|tag| tag == "alpha:binary-edge"));
        assert!(blockers
            .iter()
            .any(|blocker| blocker.kind == "required-coverage-missing"));
    }

    #[test]
    fn manifest_rejects_placeholders_and_bad_error_cases() {
        let mut manifest = Fixture::new().manifest;
        manifest
            .reference
            .environment
            .insert("machine".to_string(), Value::String("unknown".to_string()));
        assert!(validate_manifest(&manifest)
            .unwrap_err()
            .contains("placeholder"));

        let mut manifest = Fixture::new().manifest;
        manifest.cases[0].expected_outcome = ExpectedOutcome::Error;
        assert!(validate_manifest(&manifest)
            .unwrap_err()
            .contains("must leave reference paths"));

        let mut manifest = Fixture::new().manifest;
        manifest.cases[0].width = Some(64);
        assert!(validate_manifest(&manifest)
            .unwrap_err()
            .contains("128x128"));
    }

    #[test]
    fn capture_is_exact_and_check_rejects_tampering() {
        let fixture = Fixture::new();
        let report = fixture.report();
        assert_eq!(report.summary.mismatched, 0);
        assert_eq!(report.summary.success_matched, 1);
        // Public-only proof is deliberately not full acceptance.
        assert!(!report.summary.acceptance_passed);
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker.kind == "missing-private-real-coverage"));

        verify_report(&report, &fixture.manifest_path, None, OracleMode::Disabled)
            .unwrap_err()
            .contains("acceptance failed");

        let mut altered = report.clone();
        altered.cases[0].candidate_rgba_sha256 = Some("0".repeat(64));
        assert!(
            verify_report(&altered, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("stale or altered")
        );

        let mut altered = report.clone();
        altered.summary.acceptance_passed = true;
        assert!(
            verify_report(&altered, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("stale or altered")
        );

        let mut altered = report.clone();
        altered.cases.clear();
        assert!(
            verify_report(&altered, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("stale or altered")
        );
    }

    #[test]
    fn altered_inputs_and_references_are_rejected() {
        let fixture = Fixture::new();
        let report = fixture.report();

        fs::write(fixture.dir.path().join("input.png"), b"not a png").unwrap();
        assert!(
            verify_report(&report, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("stale or altered input")
        );

        fs::write(fixture.dir.path().join("input.png"), TINY_PNG).unwrap();
        let reference_path = fixture.dir.path().join("reference.rgba");
        let mut reference = fs::read(&reference_path).unwrap();
        reference[0] ^= 1;
        fs::write(&reference_path, &reference).unwrap();
        assert!(
            verify_report(&report, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("does not match referenceRgbaSha256")
        );
    }

    #[test]
    fn manifest_hash_mismatch_is_stale() {
        let fixture = Fixture::new();
        let mut report = fixture.report();
        report.manifest_sha256 = "f".repeat(64);
        assert!(
            verify_report(&report, &fixture.manifest_path, None, OracleMode::Disabled)
                .unwrap_err()
                .contains("stale icon preflight report")
        );
    }

    fn zlib_compress(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn zlib_spec(raw: &[u8], width: u32, height: u32, stored: &[u8]) -> DecodedInputSpec {
        DecodedInputSpec {
            path: "decoded.zlib".to_string(),
            sha256: sha256_hex(raw),
            width,
            height,
            mode: "RGBA".to_string(),
            encoding: DecodedInputEncoding::Zlib,
            stored_sha256: Some(sha256_hex(stored)),
        }
    }

    #[test]
    fn decoded_zlib_roundtrip_and_tampered_blob_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let raw = vec![0xA5u8; 8 * 4 * 4];
        let compressed = zlib_compress(&raw);
        let path = dir.path().join("decoded.zlib");
        fs::write(&path, &compressed).unwrap();
        let spec = zlib_spec(&raw, 8, 4, &compressed);

        let (bytes, digest) = read_decoded_input(&spec, dir.path(), "case").unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(digest, spec.sha256);

        let mut tampered = compressed;
        tampered[0] ^= 0xff;
        fs::write(&path, &tampered).unwrap();
        let error = read_decoded_input(&spec, dir.path(), "case").unwrap_err();
        assert!(error.contains("storedSha256 mismatch"), "{error}");
    }

    #[test]
    fn decoded_unknown_encoding_rejected() {
        let error = serde_json::from_value::<DecodedInputSpec>(json!({
            "path": "decoded.rgba",
            "sha256": "0".repeat(64),
            "width": 1,
            "height": 1,
            "mode": "RGBA",
            "encoding": "brotli"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown variant"), "{error}");

        let spec: DecodedInputSpec = serde_json::from_value(json!({
            "path": "decoded.rgba",
            "sha256": "0".repeat(64),
            "width": 1,
            "height": 1,
            "mode": "RGBA"
        }))
        .unwrap();
        assert_eq!(spec.encoding, DecodedInputEncoding::Raw);
        assert_eq!(spec.stored_sha256, None);
    }

    #[test]
    fn decoded_zlib_requires_stored_hash() {
        let mut manifest = Fixture::new().manifest;
        manifest.decoded_inputs.insert(
            "tiny-png".to_string(),
            DecodedInputSpec {
                path: "decoded.zlib".to_string(),
                sha256: sha256_hex(&[0u8; 4]),
                width: 1,
                height: 1,
                mode: "RGBA".to_string(),
                encoding: DecodedInputEncoding::Zlib,
                stored_sha256: None,
            },
        );
        let error = validate_manifest(&manifest).unwrap_err();
        assert!(error.contains("storedSha256 is required"), "{error}");
    }

    #[test]
    fn decoded_zlib_length_bomb_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let expected = vec![0u8; 3 * 2 * 4];
        let bomb = zlib_compress(&vec![0u8; expected.len() * 10]);
        fs::write(dir.path().join("decoded.zlib"), &bomb).unwrap();
        let spec = zlib_spec(&expected, 3, 2, &bomb);
        let error = read_decoded_input(&spec, dir.path(), "case").unwrap_err();
        assert!(error.contains("must inflate to 24 RGBA bytes"), "{error}");
    }

    #[test]
    fn decoded_altered_raw_hash_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let raw = vec![7u8; 2 * 5 * 4];
        let compressed = zlib_compress(&raw);
        fs::write(dir.path().join("decoded.zlib"), &compressed).unwrap();
        let mut spec = zlib_spec(&raw, 2, 5, &compressed);
        spec.sha256 = "f".repeat(64);
        let error = read_decoded_input(&spec, dir.path(), "case").unwrap_err();
        assert!(error.contains("sha256 mismatch"), "{error}");
    }

    #[test]
    fn timestamp_roundtrip_is_utc() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let formatted = format_utc(now);
        assert_eq!(formatted.len(), 20);
        assert_eq!(parse_utc(&formatted), Some(now));
    }
}
