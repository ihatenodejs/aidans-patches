//! Process-level benchmark driver for the Python APK Lab oracle.
//!
//! The driver only executes commands declared by a corpus manifest. It never
//! installs an environment, never fabricates a missing input, and records
//! unsupported metrics as `null` with an explicit reason rather than guessing
//! units or values.
//!
//! # Process metrics
//!
//! CPU time and peak process-tree RSS are sampled every 5ms while the oracle
//! process group runs. The effective cadence is bounded below by the cost of
//! the host sampler. Collection fails closed: if the sampler cannot be read or
//! its output does not match the tested format, every metric for that sample
//! stays `null` rather than reusing a partial value.
//!
//! Linux (`/proc`):
//! - CPU: `utime + stime` from `/proc/<pid>/stat`, in clock ticks converted
//!   with `getconf CLK_TCK` (`_SC_CLK_TCK`) to milliseconds.
//! - RSS: `statm` resident pages converted with `getconf PAGESIZE` to bytes.
//! - IO: `read_bytes`/`write_bytes` from `/proc/<pid>/io` to bytes.
//!
//! macOS (`/bin/ps -ax -o pid=,ppid=,pgid=,rss=,time=`):
//! - CPU: `TIME` is cumulative CPU time rendered as `[[dd-]hh:]mm:ss[.cc]`;
//!   the tested parser accepts centisecond fractions and converts to
//!   milliseconds, summing the per-process peak seen in the group. Display
//!   resolution is 10ms per process.
//! - RSS: `rss` is resident set size in KiB (1024-byte units), converted to
//!   bytes and summed across the process group.
//! - IO: unsupported. macOS `ps` exposes no aggregate process-tree byte
//!   counters and no unsafe `libproc` FFI is used, so read/write bytes stay
//!   `null` with that reason.
//!
//! Descendants are observed by process group (`pgid == root pid`, set via
//! `process_group(0)` before spawn) plus a parent-link walk from the root.
//! `/usr/bin/time -l` is deliberately not used: its maximum resident set size
//! belongs to one child and cannot represent aggregate process-tree RSS.
//!
//! # Tool cache
//!
//! Each sample keeps an isolated `HOME`/`XDG_*`/`TMPDIR` sandbox. Workloads that
//! declare `requiredTools` use an explicit `toolCache` source: before any timed
//! sample, the driver reads `tools.lock.json`, verifies the pinned tool layout,
//! and copies only those tools into an owned staging cache. Each sample HOME
//! receives that cache at the `apk_lab.tools` path
//! (`~/.cache/aidans-patches/apk-lab/tools`), so the unmodified Python oracle
//! finds the same pinned tools without downloading, installing, inheriting the
//! user's account files, or mutating the source cache. `strict: true` aborts the
//! run before timing if preparation fails; `strict: false` skips affected
//! workloads and leaves the report incomplete. Public results contain tool
//! aliases, versions, and digests, never the private source path.
//!
//! # Staged asset inputs
//!
//! Asset-generation workloads (`font`, `ocr`, `pdfbox`, icon conversion) read
//! large private originals from a verified cache and must never write to it.
//! Such a workload may declare `stagedFiles` entries: each names an already
//! declared, hash-pinned `inputs` role and a fixed relative destination. The
//! declared source is verified and copied into the sample workspace before the
//! process is spawned. After it exits, both the source and the copy are
//! re-hashed; any mutation invalidates the sample. No hard links are used and
//! the user cache is never modified. Workloads without `stagedFiles` keep the
//! original behavior exactly.
//!
//! # Failure diagnostics
//!
//! A row that actually ran but failed semantic validation keeps bounded
//! stdout/stderr excerpts (8192 characters per stream). Private sandbox,
//! repository, home, and tool-cache prefixes are normalized before the excerpt
//! is truncated, so the diagnostic can never expose a full private path or
//! split a UTF-8 code point. Accepted pins and success semantics are unchanged;
//! failed rows still publish `null` hashes.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod assets;

use assets::{stage_declared_inputs, validate_staged_files, verify_staged_inputs, StagedFile};

const RESULT_SCHEMA_VERSION: u32 = 1;
const PAIRED_RESULT_SCHEMA_VERSION: u32 = 1;
const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub(crate) const POLICY_SCHEMA_VERSION: u32 = 1;
const PINS_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
pub(crate) const WALL_NOISE_FLOOR_MS: f64 = 5.0;
pub(crate) const RSS_NOISE_FLOOR_BYTES: u64 = 1_048_576;

const ALLOWED_CATEGORIES: &[&str] = &[
    "startup",
    "inspection",
    "splits",
    "dex",
    "unity",
    "il2cpp",
    "resources",
    "code",
    "morphe",
];

// ---------------------------------------------------------------------------
// CLI and policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Options {
    engine: String,
    profile: String,
    manifest: PathBuf,
    out: PathBuf,
    /// Native release binary used by the paired (`--engine both`) mode.
    rust_bin: Option<PathBuf>,
    policy: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    private_dir: Option<PathBuf>,
    prepare: bool,
    /// Pure JSON transform mode: no process is executed and no timing occurs.
    sanitize: bool,
    input: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SampleClass {
    pub(crate) warmups: u32,
    pub(crate) samples: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BudgetSpec {
    #[serde(default)]
    pub(crate) ratio_max: Option<f64>,
    #[serde(default)]
    pub(crate) absolute_noise_floor_ms: Option<f64>,
    #[serde(default)]
    pub(crate) absolute_noise_floor_bytes: Option<u64>,
    #[serde(default)]
    pub(crate) bootstrap_confidence: Option<f64>,
    #[serde(default)]
    pub(crate) block_on_worse: Option<bool>,
    #[serde(default)]
    pub(crate) requires_explanation: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OrderPolicy {
    pub(crate) single_engine: String,
    pub(crate) dual_engine: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PerformancePolicy {
    pub(crate) schema_version: u32,
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    pub(crate) sample_classes: BTreeMap<String, SampleClass>,
    pub(crate) budgets: BTreeMap<String, BudgetSpec>,
    pub(crate) required_workloads: Vec<String>,
    pub(crate) seed: u64,
    pub(crate) cache_policy: String,
    pub(crate) order: OrderPolicy,
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BenchmarkManifest {
    schema_version: u32,
    profile: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    repository_root: Option<String>,
    #[serde(default)]
    tool_cache: Option<ToolCacheSpec>,
    engine: EngineSpec,
    workloads: Vec<WorkloadSpec>,
}

/// Explicit source for the already-synced APK Lab tool cache.
///
/// The source is a directory in the exact layout written by `apk_lab.tools`
/// (`<name>/<version>/<sha256>/<executablePath>`). It is read-only input:
/// the benchmark copies only the tools a workload declares into a private
/// staging directory before any timed sample starts. No download or install
/// happens during the run.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolCacheSpec {
    /// Private source path, resolved against `repositoryRoot`. A leading `~/`
    /// expands to the invoking account's home directory. Never copied into a
    /// public result.
    source: String,
    /// Required. When `true`, any missing/unpinned/verification-failed required
    /// tool aborts the benchmark before any workload is timed. When `false`,
    /// affected workloads are skipped unexecuted and the report is marked
    /// incomplete. Neither mode ever installs or modifies the source cache.
    strict: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EngineSpec {
    name: String,
    executable: String,
    #[serde(default = "default_engine_cwd")]
    cwd: String,
    #[serde(default = "default_args_prefix")]
    args_prefix: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

fn default_engine_cwd() -> String {
    "tools/apk-lab".to_string()
}

fn default_args_prefix() -> Vec<String> {
    vec![
        "-B".to_string(),
        "-m".to_string(),
        "apk_lab.cli".to_string(),
    ]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkloadSpec {
    id: String,
    category: String,
    size: String,
    costly: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    warmups: Option<u32>,
    #[serde(default)]
    samples: Option<u32>,
    argv: Vec<String>,
    #[serde(default)]
    inputs: Vec<InputRef>,
    #[serde(default)]
    mpp: Option<InputRef>,
    expected: ExpectedOutput,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    isolate_workspace: bool,
    /// Pinned APK Lab tool names this workload needs. Requires `toolCache`.
    #[serde(default)]
    required_tools: Vec<String>,
    /// Declared inputs copied into the per-sample workspace before timing.
    /// Defaults to empty, preserving every pre-existing manifest.
    #[serde(default)]
    staged_files: Vec<StagedFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputRef {
    role: String,
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedOutput {
    exit_code: i32,
    #[serde(default)]
    stdout_sha256: Option<String>,
    #[serde(default)]
    stderr_sha256: Option<String>,
    #[serde(default)]
    stdout_contains: Vec<String>,
    #[serde(default)]
    files: Vec<ExpectedFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedFile {
    path: String,
    sha256: String,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Bounded, redacted stdout/stderr retained only when a row failed semantic
/// validation. Paths are replaced before the UTF-8-safe character bound is
/// applied, so a truncated excerpt can never split a code point.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputDiagnostics {
    stdout_excerpt: String,
    stderr_excerpt: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputStatus {
    role: String,
    declared_sha256: String,
    actual_sha256: Option<String>,
    size_bytes: Option<u64>,
    available: bool,
    display_path: String,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SampleResult {
    kind: String,
    cache_state: String,
    index: u32,
    wall_ms: f64,
    cpu_ms: Option<f64>,
    peak_child_rss_bytes: Option<u64>,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout_sha256: String,
    stderr_sha256: String,
    raw_stdout_sha256: String,
    raw_stderr_sha256: String,
    stdout_bytes: usize,
    stderr_bytes: usize,
    output_valid: bool,
    outputs_pinned: bool,
    output_normalization: Value,
    validation_errors: Vec<String>,
    /// Present only when this sample failed semantic validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<OutputDiagnostics>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WallSummary {
    count: usize,
    min_ms: f64,
    median_ms: f64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadResult {
    id: String,
    category: String,
    size: String,
    costly: bool,
    status: String,
    sample_class: String,
    warmups_expected: u32,
    samples_expected: u32,
    warmups_completed: u32,
    samples_completed: u32,
    reasons: Vec<String>,
    inputs: Vec<InputStatus>,
    mpp: Option<InputStatus>,
    required_tools: Vec<String>,
    argv_template: Vec<String>,
    environment: BTreeMap<String, String>,
    samples: Vec<SampleResult>,
    summary: Option<WallSummary>,
    output_validation: Value,
    output_normalization: Value,
    /// Unsupported metrics that forced this workload's values to stay null.
    /// Blocking ones are also reported in `acceptance.reasons`; non-blocking
    /// ones remain explanation-required limitations rather than failures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    metric_limitations: Vec<MetricLimitation>,
}

/// An unsupported metric recorded with its policy consequence. `blocking`
/// mirrors the pre-registered `budgets.<metric>.blockOnWorse` flag; a blocking
/// limitation is an acceptance failure, while a non-blocking limitation keeps
/// samples null and asks the cutover analysis to explain the gap.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricLimitation {
    metric: String,
    unit: String,
    blocking: bool,
    requires_explanation: bool,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Acceptance {
    valid: bool,
    reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    schema_version: u32,
    kind: String,
    status: String,
    profile: String,
    generated_at_epoch_seconds: u64,
    duration_ms: f64,
    engine: Value,
    policy: Value,
    source: Value,
    environment: Value,
    settings: Value,
    measurements: Value,
    workloads: Vec<WorkloadResult>,
    acceptance: Acceptance,
    provenance: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadPins {
    id: String,
    status: String,
    exit_code: Option<i32>,
    stdout_sha256: Option<String>,
    stderr_sha256: Option<String>,
    raw_stdout_sha256: Option<String>,
    raw_stderr_sha256: Option<String>,
    output_normalization: Vec<Value>,
    validation_errors: Vec<String>,
    /// Present only when this row ran but failed semantic validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<OutputDiagnostics>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputPinsReport {
    schema_version: u32,
    kind: String,
    status: String,
    engine: String,
    profile: String,
    manifest_sha256: String,
    source: Value,
    workloads: Vec<WorkloadPins>,
    acceptance: Acceptance,
}

#[derive(Debug, Clone)]
struct RunOutcome {
    wall_ms: f64,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    metrics: ProcessMetrics,
}

#[derive(Debug)]
struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "apk-lab-bench-{}-{}-{}",
            std::process::id(),
            nanos,
            unique_counter()
        ));
        fs::create_dir_all(&path).map_err(|e| format!("cannot create benchmark temp root: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_counter() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Pinned tool cache preparation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct ToolsLockFile {
    #[serde(default)]
    tools: BTreeMap<String, ToolLockEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolLockEntry {
    version: String,
    sha256: String,
    executable_path: String,
    #[serde(default)]
    tool_type: Option<String>,
}

#[derive(Debug, Clone)]
struct PreparedTool {
    name: String,
    version: String,
    pinned_sha256: String,
    executable_path: String,
    executable_sha256: String,
    executable_bytes: u64,
    tool_type: String,
    alias: String,
}

#[derive(Debug, Clone)]
struct ToolCacheIssue {
    tool: String,
    reason: String,
}

#[derive(Debug)]
struct PreparedToolCache {
    root: Option<TempRoot>,
    tools: Vec<PreparedTool>,
    issues: Vec<ToolCacheIssue>,
}

impl PreparedToolCache {
    fn cache_root(&self) -> Option<&Path> {
        self.root.as_ref().map(|root| root.path.as_path())
    }

    fn failed_tools(&self) -> BTreeSet<String> {
        self.issues.iter().map(|issue| issue.tool.clone()).collect()
    }
}

fn required_tool_names(manifest: &BenchmarkManifest) -> BTreeSet<String> {
    manifest
        .workloads
        .iter()
        .flat_map(|workload| workload.required_tools.iter().cloned())
        .collect()
}

fn resolve_tool_cache_source(repo_root: &Path, source: &str) -> Result<PathBuf, String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("toolCache.source must not be empty".to_string());
    }
    if trimmed == "~" {
        return invoking_home().ok_or_else(|| {
            "toolCache.source uses '~' but the invoking account home directory is unavailable"
                .to_string()
        });
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return invoking_home().map(|home| home.join(rest)).ok_or_else(|| {
            "toolCache.source uses '~/' but the invoking account home directory is unavailable"
                .to_string()
        });
    }
    Ok(resolve_against(repo_root, Path::new(trimmed)))
}

fn invoking_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Prepare an owned, read-only copy of only the pinned tools the corpus needs.
///
/// Runs before any workload is timed. The source cache is never modified.
fn prepare_tool_cache(
    repo_root: &Path,
    engine_cwd: &Path,
    spec: Option<&ToolCacheSpec>,
    required: &BTreeSet<String>,
) -> Result<PreparedToolCache, String> {
    let mut tools = Vec::new();
    let mut issues = Vec::new();
    if required.is_empty() {
        return Ok(PreparedToolCache {
            root: None,
            tools,
            issues,
        });
    }
    let spec = spec.ok_or_else(|| {
        "benchmark workloads declare requiredTools but the manifest has no toolCache".to_string()
    })?;

    let lock_path = engine_cwd.join("tools.lock.json");
    let lock_bytes = fs::read(&lock_path).map_err(|e| {
        format!(
            "cannot read pinned tool lock file {} before timing: {e}",
            lock_path.display()
        )
    })?;
    let lock: ToolsLockFile = serde_json::from_slice(&lock_bytes).map_err(|e| {
        format!(
            "invalid pinned tool lock file {} before timing: {e}",
            lock_path.display()
        )
    })?;

    let source = match resolve_tool_cache_source(repo_root, &spec.source) {
        Ok(source) if source.is_dir() => source,
        Ok(_) => {
            let reason = "configured private tool cache source is not a directory".to_string();
            if spec.strict {
                return Err(format!(
                    "tool cache preflight failed before timing: {reason}"
                ));
            }
            for tool in required {
                issues.push(ToolCacheIssue {
                    tool: tool.clone(),
                    reason: reason.clone(),
                });
            }
            return Ok(PreparedToolCache {
                root: None,
                tools,
                issues,
            });
        }
        Err(error) => {
            if spec.strict {
                return Err(format!(
                    "tool cache preflight failed before timing: {error}"
                ));
            }
            for tool in required {
                issues.push(ToolCacheIssue {
                    tool: tool.clone(),
                    reason: error.clone(),
                });
            }
            return Ok(PreparedToolCache {
                root: None,
                tools,
                issues,
            });
        }
    };

    let staging = TempRoot::new()?;
    for name in required {
        match stage_pinned_tool(&source, &staging.path, name, &lock) {
            Ok(tool) => tools.push(tool),
            Err(reason) => issues.push(ToolCacheIssue {
                tool: name.clone(),
                reason,
            }),
        }
    }

    if spec.strict && !issues.is_empty() {
        let detail = issues
            .iter()
            .map(|issue| format!("{}: {}", issue.tool, issue.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "tool cache preflight failed before any timed sample: {detail}"
        ));
    }

    Ok(PreparedToolCache {
        root: Some(staging),
        tools,
        issues,
    })
}

fn stage_pinned_tool(
    source: &Path,
    staging_root: &Path,
    name: &str,
    lock: &ToolsLockFile,
) -> Result<PreparedTool, String> {
    let entry = lock
        .tools
        .get(name)
        .ok_or_else(|| "no tools.lock.json entry (unpinned)".to_string())?;
    if entry.version.trim().is_empty()
        || entry.version == "."
        || entry.version == ".."
        || entry.version.contains('/')
        || entry.version.contains('\\')
    {
        return Err(
            "tools.lock.json version is missing or is not a safe path component (unpinned)"
                .to_string(),
        );
    }
    if entry.sha256.len() != 64 || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("tools.lock.json sha256 is missing or malformed (unpinned)".to_string());
    }
    if entry.executable_path.trim().is_empty()
        || Path::new(&entry.executable_path)
            .components()
            .any(|component| {
                matches!(
                    component,
                    std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                        | std::path::Component::ParentDir
                )
            })
    {
        return Err(
            "tools.lock.json executablePath is empty or escapes the install directory (unpinned)"
                .to_string(),
        );
    }

    let relative = Path::new(name).join(&entry.version).join(&entry.sha256);
    let source_install = source.join(&relative);
    if !source_install.is_dir() {
        return Err("not present in the configured private tool cache".to_string());
    }
    let source_executable = source_install.join(&entry.executable_path);
    if !source_executable.is_file() {
        return Err("pinned executable is missing from the private tool cache".to_string());
    }

    let staged_install = staging_root.join(&relative);
    let copy_result = (|| -> Result<PreparedTool, String> {
        copy_tool_tree(&source_install, &staged_install)?;
        let staged_executable = staged_install.join(&entry.executable_path);
        let digest = hash_file(&staged_executable)?;
        let tool_type = entry
            .tool_type
            .clone()
            .unwrap_or_else(|| "binary".to_string());
        if tool_type != "zip" && !digest.sha256.eq_ignore_ascii_case(&entry.sha256) {
            return Err(format!(
                "staged executable sha256 {} does not match the pinned tools.lock.json sha256 {}",
                digest.sha256, entry.sha256
            ));
        }
        Ok(PreparedTool {
            name: name.to_string(),
            version: entry.version.clone(),
            pinned_sha256: entry.sha256.to_ascii_lowercase(),
            executable_path: entry.executable_path.clone(),
            executable_sha256: digest.sha256,
            executable_bytes: digest.size_bytes,
            tool_type,
            alias: relative
                .join(&entry.executable_path)
                .to_string_lossy()
                .replace('\\', "/"),
        })
    })();
    if copy_result.is_err() {
        let _ = fs::remove_dir_all(&staged_install);
    }
    copy_result
}

fn copy_tool_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|e| format!("cannot inspect cached tool entry: {e}"))?;
    if metadata.file_type().is_symlink() {
        return Err(
            "private tool cache contains a symlink; only owned regular files are staged"
                .to_string(),
        );
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .map_err(|e| format!("cannot create staged tool directory: {e}"))?;
        let entries =
            fs::read_dir(source).map_err(|e| format!("cannot read cached tool directory: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read cached tool entry: {e}"))?;
            copy_tool_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())
            .map_err(|e| format!("cannot set staged tool directory permissions: {e}"))?;
        Ok(())
    } else if metadata.is_file() {
        fs::copy(source, destination).map_err(|e| format!("cannot copy cached tool file: {e}"))?;
        fs::set_permissions(destination, metadata.permissions())
            .map_err(|e| format!("cannot set staged tool file permissions: {e}"))?;
        Ok(())
    } else {
        Err("private tool cache contains an unsupported entry type".to_string())
    }
}

/// Point only the sample's APK Lab tool-cache path at the prepared owned cache.
/// HOME, XDG_*, TMPDIR, and PYTHONPYCACHEPREFIX stay inside the per-sample root.
fn install_sample_tool_cache(home: &Path, cache_root: &Path) -> Result<(), String> {
    let cache_root = cache_root
        .canonicalize()
        .map_err(|e| format!("cannot resolve prepared tool cache: {e}"))?;
    let parent = home.join(".cache").join("aidans-patches").join("apk-lab");
    fs::create_dir_all(&parent)
        .map_err(|e| format!("cannot create sample tool-cache parent: {e}"))?;
    let destination = parent.join("tools");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&cache_root, &destination)
            .map_err(|e| format!("cannot link prepared tool cache into sample HOME: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        copy_tool_tree(&cache_root, &destination)?;
    }
    Ok(())
}

/// Public toolchain view: aliases, versions, and digests only. The private
/// source cache path is deliberately omitted.
fn tool_cache_report(spec: Option<&ToolCacheSpec>, prepared: &PreparedToolCache) -> Value {
    let tools = prepared
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "version": tool.version,
                "toolType": tool.tool_type,
                "alias": tool.alias,
                "executablePath": tool.executable_path,
                "pinnedSha256": tool.pinned_sha256,
                "executableSha256": tool.executable_sha256,
                "executableBytes": tool.executable_bytes,
                "provenance": if tool.tool_type == "zip" {
                    "lockfile archive sha256 pin; extracted executable digest inventoried"
                } else {
                    "executable sha256 verified against tools.lock.json pin"
                },
            })
        })
        .collect::<Vec<_>>();
    let issues = prepared
        .issues
        .iter()
        .map(|issue| {
            json!({
                "tool": issue.tool,
                "reason": issue.reason,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "configured": spec.is_some(),
        "strict": spec.map(|spec| spec.strict),
        "cachePolicy": "Prepared once before any timed sample; the same staged files are warm-file-cache for every warmup and measured sample.",
        "layout": "<name>/<version>/<sha256>/<executablePath> (apk_lab.tools layout)",
        "tools": tools,
        "issues": issues,
    })
}

struct SourceArchiveInfo {
    location: String,
    digest: String,
    file_count: usize,
}

/// Sanitized source identity shared by baseline results and output-pin
/// preparation. The archive location is a private relative label, never the
/// absolute private directory used on this host.
fn source_report_value(
    source: &SourceSnapshot,
    source_archive: &Result<SourceArchiveInfo, String>,
) -> Value {
    let archive_value = match source_archive {
        Ok(info) => json!({
            "kind": "private-source-archive",
            "location": info.location.clone(),
            "digest": info.digest.clone(),
            "fileCount": info.file_count,
            "reconstruction": "Restore the archived files exactly; dirty.patch documents the pre-existing worktree difference and must not be applied on top.",
        }),
        Err(_) => Value::Null,
    };
    json!({
        "revision": source.revision.clone(),
        "dirty": source.dirty,
        "worktreeDigest": source.digest.clone(),
        "headIsExecutedSource": !source.dirty,
        "warning": if source.dirty {
            Value::String("HEAD is not the executed source because the worktree was dirty; use worktreeDigest and the private source archive.".to_string())
        } else {
            Value::Null
        },
        "archive": archive_value,
    })
}

struct SampleRun {
    sample: SampleResult,
    hard_errors: Vec<String>,
    unpinned: bool,
}

#[derive(Clone)]
#[allow(dead_code)]
struct MetricUnits {
    clock_ticks_per_second: f64,
    page_size_bytes: u64,
}

#[derive(Clone)]
#[allow(dead_code)]
struct MetricCapability {
    cpu_supported: bool,
    rss_supported: bool,
    io_supported: bool,
    units: Option<MetricUnits>,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ProcessMetrics {
    cpu_ms: Option<f64>,
    peak_child_rss_bytes: Option<u64>,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

impl ProcessMetrics {
    fn unsupported() -> Self {
        Self {
            cpu_ms: None,
            peak_child_rss_bytes: None,
            read_bytes: None,
            write_bytes: None,
        }
    }
}

struct ProcessSampler {
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<ProcessMetrics>,
}

impl ProcessSampler {
    fn finish(self) -> ProcessMetrics {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .join()
            .unwrap_or_else(|_| ProcessMetrics::unsupported())
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Runs the `bench` subcommand. `args` are the CLI words after `bench`.
pub fn run(args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    if opts.sanitize {
        let input = opts
            .input
            .as_deref()
            .ok_or_else(|| "bench --sanitize requires --input".to_string())?;
        return sanitize_result(input, &opts.out);
    }
    if !matches!(opts.engine.as_str(), "python" | "both") {
        return Err(format!(
            "unsupported benchmark engine '{}'; use 'python' for the baseline or 'both' for paired measurement",
            opts.engine
        ));
    }
    if opts.engine == "both" && opts.prepare {
        return Err("'--prepare' is only supported with '--engine python'".to_string());
    }
    if opts.profile != "local" {
        return Err(format!(
            "unsupported benchmark profile '{}'; only 'local' is implemented",
            opts.profile
        ));
    }

    let policy_path = match &opts.policy {
        Some(path) => path.clone(),
        None => {
            let root = find_repo_root(&opts.repo_root)?;
            root.join("tools/apk-lab/migration/performance-policy.json")
        }
    };
    let policy_bytes = fs::read(&policy_path).map_err(|e| {
        format!(
            "cannot read performance policy {}: {e}",
            policy_path.display()
        )
    })?;
    let policy_sha256 = sha256_bytes(&policy_bytes);
    let policy: PerformancePolicy = serde_json::from_slice(&policy_bytes)
        .map_err(|e| format!("invalid performance policy {}: {e}", policy_path.display()))?;
    validate_policy(&policy)?;

    let manifest_bytes = fs::read(&opts.manifest).map_err(|e| {
        format!(
            "cannot read benchmark manifest {}: {e}",
            opts.manifest.display()
        )
    })?;
    let manifest: BenchmarkManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        format!(
            "invalid benchmark manifest {}: {e}",
            opts.manifest.display()
        )
    })?;
    validate_manifest(&manifest, &policy, &opts.profile)?;

    let repo_root = resolve_repo_root(&opts, &manifest)?;

    if opts.engine == "both" {
        return run_paired(
            &repo_root,
            &opts,
            &policy,
            &policy_sha256,
            &manifest,
            &manifest_bytes,
        );
    }

    if opts.prepare {
        let report = execute_prepare(&repo_root, &opts, &policy, &manifest, &manifest_bytes)?;
        let value = serde_json::to_value(&report)
            .map_err(|e| format!("cannot serialize benchmark output pins: {e}"))?;
        validate_pins_shape(&value)?;
        write_json_atomic(&opts.out, &value)?;

        if report.acceptance.valid {
            return Ok(());
        }
        let reasons = report.acceptance.reasons.join("; ");
        return Err(format!("output pin preparation incomplete: {reasons}"));
    }

    let report = execute_benchmark(
        &repo_root,
        &opts,
        &policy,
        &policy_sha256,
        &manifest,
        &manifest_bytes,
    )?;

    let value = serde_json::to_value(&report)
        .map_err(|e| format!("cannot serialize benchmark report: {e}"))?;
    validate_result_shape(&value)?;
    write_json_atomic(&opts.out, &value)?;

    if report.acceptance.valid {
        Ok(())
    } else {
        let reasons = report.acceptance.reasons.join("; ");
        Err(format!("benchmark incomplete: {reasons}"))
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut engine: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut rust_bin: Option<PathBuf> = None;
    let mut policy: Option<PathBuf> = None;
    let mut repo_root: Option<PathBuf> = None;
    let mut private_dir: Option<PathBuf> = None;
    let mut prepare = false;
    let mut sanitize = false;
    let mut input: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline_value) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        let value = |index: &mut usize| -> Result<String, String> {
            if let Some(value) = inline_value.clone() {
                return Ok(value);
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for '{flag}'"))
        };

        match flag {
            "--engine" => engine = Some(value(&mut index)?),
            "--profile" => profile = Some(value(&mut index)?),
            "--manifest" => manifest = Some(PathBuf::from(value(&mut index)?)),
            "--out" => out = Some(PathBuf::from(value(&mut index)?)),
            "--rust-bin" => rust_bin = Some(PathBuf::from(value(&mut index)?)),
            "--policy" => policy = Some(PathBuf::from(value(&mut index)?)),
            "--repo-root" => repo_root = Some(PathBuf::from(value(&mut index)?)),
            "--private-dir" => private_dir = Some(PathBuf::from(value(&mut index)?)),
            "--input" => input = Some(PathBuf::from(value(&mut index)?)),
            "--sanitize" => {
                if inline_value.is_some() {
                    return Err("'--sanitize' does not take a value".to_string());
                }
                sanitize = true;
            }
            "--prepare" => {
                if inline_value.is_some() {
                    return Err("'--prepare' does not take a value".to_string());
                }
                prepare = true;
            }
            "-h" | "--help" => {
                return Err(
                    "usage: bench [--prepare] --engine python|both --profile local --manifest PATH --out PATH \
                     [--rust-bin PATH] [--policy PATH] [--repo-root PATH] [--private-dir PATH]\n       \
                     bench --sanitize --input RAW.json --out PUBLIC.json"
                        .to_string(),
                )
            }
            other => return Err(format!("unknown benchmark option '{other}'")),
        }
        index += 1;
    }

    let out = out.ok_or_else(|| "bench requires --out".to_string())?;
    if sanitize {
        if prepare {
            return Err("'--prepare' cannot be combined with '--sanitize'".to_string());
        }
        for (name, present) in [
            ("--engine", engine.is_some()),
            ("--profile", profile.is_some()),
            ("--manifest", manifest.is_some()),
            ("--policy", policy.is_some()),
            ("--repo-root", repo_root.is_some()),
            ("--private-dir", private_dir.is_some()),
            ("--rust-bin", rust_bin.is_some()),
        ] {
            if present {
                return Err(format!("'{name}' cannot be combined with '--sanitize'"));
            }
        }
        let input = input.ok_or_else(|| "bench --sanitize requires --input".to_string())?;
        return Ok(Options {
            engine: String::new(),
            profile: String::new(),
            manifest: PathBuf::new(),
            out,
            rust_bin: None,
            policy: None,
            repo_root: None,
            private_dir: None,
            prepare: false,
            sanitize: true,
            input: Some(input),
        });
    }
    if input.is_some() {
        return Err("'--input' is only valid with '--sanitize'".to_string());
    }

    Ok(Options {
        engine: engine.ok_or_else(|| "bench requires --engine".to_string())?,
        profile: profile.ok_or_else(|| "bench requires --profile".to_string())?,
        manifest: manifest.ok_or_else(|| "bench requires --manifest".to_string())?,
        out,
        rust_bin,
        policy,
        repo_root,
        private_dir,
        prepare,
        sanitize: false,
        input: None,
    })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

pub(crate) fn validate_policy(policy: &PerformancePolicy) -> Result<(), String> {
    if policy.schema_version != POLICY_SCHEMA_VERSION {
        return Err(format!(
            "performance policy schemaVersion must be {POLICY_SCHEMA_VERSION}, got {}",
            policy.schema_version
        ));
    }
    if policy.id.trim().is_empty() {
        return Err("performance policy id must not be empty".to_string());
    }
    let local = policy
        .sample_classes
        .get("local")
        .ok_or_else(|| "performance policy is missing sampleClasses.local".to_string())?;
    if local.warmups != 5 || local.samples != 30 {
        return Err(format!(
            "sampleClasses.local must pre-register 5 warmups and 30 samples, got {}/{}",
            local.warmups, local.samples
        ));
    }
    let costly = policy
        .sample_classes
        .get("costly")
        .ok_or_else(|| "performance policy is missing sampleClasses.costly".to_string())?;
    if costly.warmups != 2 || costly.samples != 10 {
        return Err(format!(
            "sampleClasses.costly must pre-register 2 warmups and 10 samples, got {}/{}",
            costly.warmups, costly.samples
        ));
    }

    let wall = policy
        .budgets
        .get("wallTime")
        .ok_or_else(|| "performance policy is missing budgets.wallTime".to_string())?;
    let wall_ratio = wall
        .ratio_max
        .ok_or_else(|| "budgets.wallTime.ratioMax is missing".to_string())?;
    if (wall_ratio - 1.05).abs() > f64::EPSILON {
        return Err(format!(
            "budgets.wallTime.ratioMax must be pre-registered as 1.05, got {wall_ratio}"
        ));
    }
    if wall.absolute_noise_floor_ms != Some(WALL_NOISE_FLOOR_MS) {
        return Err(format!(
            "budgets.wallTime.absoluteNoiseFloorMs must be {WALL_NOISE_FLOOR_MS}"
        ));
    }
    if wall.bootstrap_confidence != Some(0.95) || wall.block_on_worse != Some(true) {
        return Err(
            "budgets.wallTime must require bootstrapConfidence 0.95 and blockOnWorse true"
                .to_string(),
        );
    }

    let rss = policy
        .budgets
        .get("rss")
        .ok_or_else(|| "performance policy is missing budgets.rss".to_string())?;
    let rss_ratio = rss
        .ratio_max
        .ok_or_else(|| "budgets.rss.ratioMax is missing".to_string())?;
    if (rss_ratio - 1.05).abs() > f64::EPSILON {
        return Err(format!(
            "budgets.rss.ratioMax must be pre-registered as 1.05, got {rss_ratio}"
        ));
    }
    if rss.absolute_noise_floor_bytes != Some(RSS_NOISE_FLOOR_BYTES) {
        return Err(format!(
            "budgets.rss.absoluteNoiseFloorBytes must be {RSS_NOISE_FLOOR_BYTES}"
        ));
    }
    if rss.bootstrap_confidence != Some(0.95) || rss.block_on_worse != Some(true) {
        return Err(
            "budgets.rss must require bootstrapConfidence 0.95 and blockOnWorse true".to_string(),
        );
    }

    let mut ids = BTreeSet::new();
    for id in &policy.required_workloads {
        if id.trim().is_empty() {
            return Err("requiredWorkloads contains an empty id".to_string());
        }
        if !ids.insert(id.clone()) {
            return Err(format!("requiredWorkloads contains duplicate id '{id}'"));
        }
    }
    if ids.is_empty() {
        return Err("requiredWorkloads must not be empty".to_string());
    }
    if policy.cache_policy.trim().is_empty() {
        return Err("performance policy cachePolicy must not be empty".to_string());
    }
    if policy.order.single_engine.trim().is_empty() || policy.order.dual_engine.trim().is_empty() {
        return Err("performance policy order settings must not be empty".to_string());
    }
    Ok(())
}

fn validate_manifest(
    manifest: &BenchmarkManifest,
    policy: &PerformancePolicy,
    profile: &str,
) -> Result<(), String> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "benchmark manifest schemaVersion must be {MANIFEST_SCHEMA_VERSION}, got {}",
            manifest.schema_version
        ));
    }
    if manifest.profile != profile {
        return Err(format!(
            "benchmark manifest profile '{}' does not match requested profile '{}'",
            manifest.profile, profile
        ));
    }
    if manifest.engine.name != "python" {
        return Err(format!(
            "benchmark manifest engine.name must be 'python', got '{}'",
            manifest.engine.name
        ));
    }
    if manifest.engine.executable.trim().is_empty() {
        return Err("benchmark manifest engine.executable must not be empty".to_string());
    }
    let executable_name = Path::new(&manifest.engine.executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if executable_name == "uv"
        || manifest.engine.args_prefix.first().map(String::as_str) == Some("uv")
    {
        return Err(
            "benchmark engine must execute a pre-synced Python interpreter directly, not uv"
                .to_string(),
        );
    }
    if manifest.engine.cwd.trim().is_empty() {
        return Err("benchmark manifest engine.cwd must not be empty".to_string());
    }
    for key in manifest.engine.env.keys() {
        check_env_key(key)?;
    }
    if let Some(tool_cache) = &manifest.tool_cache {
        if tool_cache.source.trim().is_empty() {
            return Err("benchmark manifest toolCache.source must not be empty".to_string());
        }
    }
    if manifest.workloads.is_empty() {
        return Err("benchmark manifest must declare at least one workload".to_string());
    }

    let mut seen = BTreeSet::new();
    for workload in &manifest.workloads {
        if workload.id.trim().is_empty() {
            return Err("workload id must not be empty".to_string());
        }
        if !workload
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err(format!(
                "workload id '{}' may only contain ASCII letters, digits, '.', '-', '_'",
                workload.id
            ));
        }
        if !seen.insert(workload.id.clone()) {
            return Err(format!("duplicate workload id '{}'", workload.id));
        }
        if !ALLOWED_CATEGORIES.contains(&workload.category.as_str()) {
            return Err(format!(
                "workload '{}' has unknown category '{}'",
                workload.id, workload.category
            ));
        }
        if !matches!(workload.size.as_str(), "small" | "large") {
            return Err(format!(
                "workload '{}' size must be 'small' or 'large', got '{}'",
                workload.id, workload.size
            ));
        }
        if workload.argv.is_empty() {
            return Err(format!("workload '{}' has an empty argv", workload.id));
        }
        for arg in &workload.argv {
            if arg.contains('\0') {
                return Err(format!(
                    "workload '{}' argv contains a NUL byte",
                    workload.id
                ));
            }
        }
        for input in &workload.inputs {
            validate_input_ref(&workload.id, input)?;
        }
        if let Some(mpp) = &workload.mpp {
            validate_input_ref(&workload.id, mpp)?;
        }
        validate_sha256(
            &workload.id,
            "expected.stdoutSha256",
            workload.expected.stdout_sha256.as_deref(),
        )?;
        validate_sha256(
            &workload.id,
            "expected.stderrSha256",
            workload.expected.stderr_sha256.as_deref(),
        )?;
        for file in &workload.expected.files {
            if file.path.trim().is_empty() {
                return Err(format!(
                    "workload '{}' declares an expected file with an empty path",
                    workload.id
                ));
            }
            validate_hex_sha256(
                &format!("workload '{}' expected file '{}'", workload.id, file.path),
                &file.sha256,
            )?;
        }
        for key in workload.env.keys() {
            check_env_key(key)?;
        }
        if !workload.required_tools.is_empty() && manifest.tool_cache.is_none() {
            return Err(format!(
                "workload '{}' declares requiredTools but the manifest has no toolCache source",
                workload.id
            ));
        }
        validate_staged_files(&workload.id, &workload.inputs, &workload.staged_files)?;
        for tool in &workload.required_tools {
            if tool.trim().is_empty() {
                return Err(format!(
                    "workload '{}' declares an empty requiredTools entry",
                    workload.id
                ));
            }
            if !tool
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
            {
                return Err(format!(
                    "workload '{}' required tool '{}' may only contain ASCII letters, digits, '.', '-', '_'",
                    workload.id, tool
                ));
            }
        }

        let class_name = if workload.costly { "costly" } else { "local" };
        let class = policy
            .sample_classes
            .get(class_name)
            .ok_or_else(|| format!("performance policy is missing sample class '{class_name}'"))?;
        if let Some(warmups) = workload.warmups {
            if warmups < class.warmups {
                return Err(format!(
                    "workload '{}' requests {warmups} warmups but the pre-registered minimum for '{class_name}' is {}",
                    workload.id, class.warmups
                ));
            }
        }
        if let Some(samples) = workload.samples {
            if samples < class.samples {
                return Err(format!(
                    "workload '{}' requests {samples} samples but the pre-registered minimum for '{class_name}' is {}",
                    workload.id, class.samples
                ));
            }
        }
    }
    Ok(())
}

fn validate_input_ref(workload_id: &str, input: &InputRef) -> Result<(), String> {
    if input.role.trim().is_empty() {
        return Err(format!(
            "workload '{workload_id}' declares an input with an empty role"
        ));
    }
    if input.path.trim().is_empty() {
        return Err(format!(
            "workload '{workload_id}' input '{}' has an empty path",
            input.role
        ));
    }
    validate_hex_sha256(
        &format!("workload '{workload_id}' input '{}'", input.role),
        &input.sha256,
    )
}

fn validate_sha256(workload_id: &str, field: &str, value: Option<&str>) -> Result<(), String> {
    if let Some(value) = value {
        validate_hex_sha256(&format!("workload '{workload_id}' {field}"), value)?;
    }
    Ok(())
}

fn validate_hex_sha256(context: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "{context} sha256 must be 64 hexadecimal characters"
        ));
    }
    Ok(())
}

fn check_env_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("workload env contains an empty key".to_string());
    }
    if is_secret_key(key) {
        return Err(format!(
            "workload env key '{key}' looks secret; benchmark manifests must not carry credentials"
        ));
    }
    if !is_allowed_env_key(key) {
        return Err(format!(
            "workload env key '{key}' is not on the explicit safe allowlist"
        ));
    }
    Ok(())
}

fn is_allowed_env_key(key: &str) -> bool {
    SAFE_INHERITED_ENV_KEYS.contains(&key)
        || matches!(key, "PYTHONHASHSEED" | "PYTHONDONTWRITEBYTECODE")
        || key.starts_with("APK_LAB_")
}

fn is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.contains("PASSWORD")
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("CREDENTIAL")
        || upper.contains("PRIVATE_KEY")
}

fn validate_pins_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "output pins report must be a JSON object".to_string())?;
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(PINS_SCHEMA_VERSION as u64) {
        return Err("output pins schemaVersion is missing or wrong".to_string());
    }
    if object.get("kind").and_then(Value::as_str) != Some("apk-lab-benchmark-output-pins") {
        return Err("output pins kind is missing or wrong".to_string());
    }
    match object.get("status").and_then(Value::as_str) {
        Some("complete") | Some("incomplete") => {}
        _ => return Err("output pins status must be 'complete' or 'incomplete'".to_string()),
    }
    if object.get("engine").and_then(Value::as_str) != Some("python") {
        return Err("output pins engine must be 'python'".to_string());
    }
    if object.get("profile").and_then(Value::as_str) != Some("local") {
        return Err("output pins profile must be 'local'".to_string());
    }
    if object
        .get("manifestSha256")
        .and_then(Value::as_str)
        .map(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        != Some(true)
    {
        return Err("output pins manifestSha256 must be 64 hexadecimal characters".to_string());
    }
    if !object.get("source").map(Value::is_object).unwrap_or(false) {
        return Err("output pins source must be an object".to_string());
    }
    let workloads = object
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "output pins workloads must be an array".to_string())?;
    for workload in workloads {
        match workload.get("status").and_then(Value::as_str) {
            Some("prepared") | Some("failed") | Some("incomplete") => {}
            _ => {
                return Err(
                    "output pins workload status must be prepared, failed, or incomplete"
                        .to_string(),
                )
            }
        }
        if !workload
            .get("outputNormalization")
            .map(Value::is_array)
            .unwrap_or(false)
        {
            return Err("output pins workload outputNormalization must be an array".to_string());
        }
        if !workload
            .get("validationErrors")
            .map(Value::is_array)
            .unwrap_or(false)
        {
            return Err("output pins workload validationErrors must be an array".to_string());
        }
        if let Some(diagnostics) = workload.get("diagnostics") {
            if workload.get("status").and_then(Value::as_str) != Some("failed") {
                return Err("output pins diagnostics are only allowed on failed rows".to_string());
            }
            for field in ["stdoutExcerpt", "stderrExcerpt"] {
                if !diagnostics
                    .get(field)
                    .map(Value::is_string)
                    .unwrap_or(false)
                {
                    return Err(format!("output pins diagnostics.{field} must be a string"));
                }
            }
            for field in ["stdoutTruncated", "stderrTruncated"] {
                if !diagnostics
                    .get(field)
                    .map(Value::is_boolean)
                    .unwrap_or(false)
                {
                    return Err(format!("output pins diagnostics.{field} must be a boolean"));
                }
            }
        }
    }
    if object
        .get("acceptance")
        .and_then(|value| value.get("valid"))
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err("output pins acceptance.valid must be a boolean".to_string());
    }
    Ok(())
}

fn validate_result_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "benchmark result must be a JSON object".to_string())?;
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(RESULT_SCHEMA_VERSION as u64) {
        return Err("benchmark result schemaVersion is missing or wrong".to_string());
    }
    if object.get("kind").and_then(Value::as_str) != Some("apk-lab-benchmark-result") {
        return Err("benchmark result kind is missing or wrong".to_string());
    }
    match object.get("status").and_then(Value::as_str) {
        Some("complete") | Some("incomplete") => {}
        _ => return Err("benchmark result status must be 'complete' or 'incomplete'".to_string()),
    }
    if !object
        .get("workloads")
        .map(Value::is_array)
        .unwrap_or(false)
    {
        return Err("benchmark result workloads must be an array".to_string());
    }
    if object
        .get("acceptance")
        .and_then(|value| value.get("valid"))
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err("benchmark result acceptance.valid must be a boolean".to_string());
    }
    if !object.get("source").map(Value::is_object).unwrap_or(false) {
        return Err("benchmark result source must be an object".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

fn execute_benchmark(
    repo_root: &Path,
    opts: &Options,
    policy: &PerformancePolicy,
    policy_sha256: &str,
    manifest: &BenchmarkManifest,
    manifest_bytes: &[u8],
) -> Result<BenchmarkReport, String> {
    let started = Instant::now();
    let metric_capability = detect_metric_capability();
    let engine_exe = resolve_against(repo_root, Path::new(&manifest.engine.executable));
    let engine_cwd = resolve_against(repo_root, Path::new(&manifest.engine.cwd));

    let required_tools = required_tool_names(manifest);
    let prepared_tools = prepare_tool_cache(
        repo_root,
        &engine_cwd,
        manifest.tool_cache.as_ref(),
        &required_tools,
    )?;
    let failed_tools = prepared_tools.failed_tools();

    let python_version = command_version(&engine_exe, "--version");
    let git_version = command_version(Path::new("git"), "--version");
    let (source, mut provenance_reasons) = collect_source(repo_root);
    let environment = collect_environment(
        repo_root,
        &engine_exe,
        &engine_cwd,
        python_version,
        git_version,
    );

    let base_env = effective_base_env(&manifest.engine.env, policy.seed);
    let private_dir = opts.private_dir.clone().unwrap_or_else(|| {
        let parent = opts.out.parent().unwrap_or_else(|| Path::new("."));
        parent.join("baseline-source")
    });

    let source_archive = archive_source(repo_root, &private_dir, &source);
    if let Err(error) = &source_archive {
        provenance_reasons.push(format!("private source archive failed: {error}"));
    }
    let source_value = source_report_value(&source, &source_archive);
    let provenance_value = json!({
        "schemaVersion": 1,
        "policySha256": policy_sha256,
        "manifestSha256": sha256_bytes(manifest_bytes),
        "manifestStatus": "local-private-manifest-not-committed",
        "sourceRevision": source.revision.clone(),
        "dirty": source.dirty,
        "worktreeDigest": source.digest.clone(),
        "archiveDigest": source_archive.as_ref().map(|info| info.digest.clone()),
        "reconstruction": {
            "executable": "Pre-synced Python interpreter from engine.executable (no uv install).",
            "source": "Restore the private archive exactly as executed and verify its file digests; dirty.patch is evidence, not a patch to re-apply.",
            "inputs": "Verify every input sha256 before executing a workload.",
        },
        "toolchain": tool_cache_report(manifest.tool_cache.as_ref(), &prepared_tools),
    });

    let oracle_ready = engine_exe.is_file();
    if !oracle_ready {
        provenance_reasons.push(format!(
            "python oracle executable not found: {}",
            display_path(&engine_exe, repo_root)
        ));
    }
    if !engine_cwd.is_dir() {
        provenance_reasons.push(format!(
            "python oracle cwd not found: {}",
            display_path(&engine_cwd, repo_root)
        ));
    }

    let mut results = Vec::with_capacity(manifest.workloads.len());
    for workload in &manifest.workloads {
        let result = benchmark_workload(
            repo_root,
            &engine_exe,
            &engine_cwd,
            &manifest.engine.args_prefix,
            &base_env,
            policy,
            workload,
            &metric_capability,
            oracle_ready && engine_cwd.is_dir(),
            &failed_tools,
            prepared_tools.cache_root(),
        );
        results.push(result);
    }

    let present: BTreeSet<String> = manifest
        .workloads
        .iter()
        .map(|workload| workload.id.clone())
        .collect();
    let mut acceptance_reasons = provenance_reasons;
    for issue in &prepared_tools.issues {
        acceptance_reasons.push(format!(
            "required tool '{}' was not staged ({})",
            issue.tool, issue.reason
        ));
    }
    let metric_evaluation = evaluate_metric_policy(policy, &metric_capability);
    acceptance_reasons.extend(metric_evaluation.blockers.iter().cloned());
    for result in &mut results {
        if result
            .samples
            .iter()
            .any(|sample| sample.kind == "measured" && sample.output_valid)
        {
            result.metric_limitations = metric_evaluation.limitations.clone();
        }
    }
    for result in &results {
        let measured: Vec<&SampleResult> = result
            .samples
            .iter()
            .filter(|sample| sample.kind == "measured" && sample.output_valid)
            .collect();
        if measured.is_empty() {
            continue;
        }
        let mut missing = Vec::new();
        if budget_blocks_on_worse(policy, "cpu")
            && metric_capability.cpu_supported
            && measured.iter().all(|sample| sample.cpu_ms.is_none())
        {
            missing.push("CPU time");
        }
        if budget_blocks_on_worse(policy, "rss")
            && metric_capability.rss_supported
            && measured
                .iter()
                .all(|sample| sample.peak_child_rss_bytes.is_none())
        {
            missing.push("peak descendant RSS");
        }
        if budget_blocks_on_worse(policy, "io")
            && metric_capability.io_supported
            && measured
                .iter()
                .all(|sample| sample.read_bytes.is_none() && sample.write_bytes.is_none())
        {
            missing.push("process IO");
        }
        if !missing.is_empty() {
            acceptance_reasons.push(format!(
                "workload '{}' measured without mandatory metrics: {}",
                result.id,
                missing.join(", ")
            ));
        }
    }
    for required in &policy.required_workloads {
        if !present.contains(required) {
            acceptance_reasons.push(format!(
                "required workload '{required}' is missing from the manifest"
            ));
        }
    }
    for result in &results {
        if result.status != "measured" {
            let detail = if result.reasons.is_empty() {
                result.status.clone()
            } else {
                result.reasons.join(", ")
            };
            acceptance_reasons.push(format!(
                "workload '{}' is not measured ({detail})",
                result.id
            ));
        }
    }
    let acceptance = Acceptance {
        valid: acceptance_reasons.is_empty(),
        reasons: acceptance_reasons,
    };
    let status = if acceptance.valid {
        "complete"
    } else {
        "incomplete"
    };

    Ok(BenchmarkReport {
        schema_version: RESULT_SCHEMA_VERSION,
        kind: "apk-lab-benchmark-result".to_string(),
        status: status.to_string(),
        profile: manifest.profile.clone(),
        generated_at_epoch_seconds: now_epoch_seconds(),
        duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        engine: json!({
            "name": "python",
            "executable": display_path(&engine_exe, repo_root),
            "version": environment.get("pythonVersion").cloned().unwrap_or(Value::Null),
            "cwd": display_path(&engine_cwd, repo_root),
            "argsPrefix": manifest.engine.args_prefix,
            "directExecution": true,
            "uvInstallPerformed": false,
        }),
        policy: json!({
            "id": policy.id.clone(),
            "sha256": policy_sha256,
            "sampleClasses": policy.sample_classes.clone(),
            "budgets": policy.budgets.clone(),
            "seed": policy.seed,
            "cachePolicy": policy.cache_policy.clone(),
            "order": policy.order.clone(),
        }),
        source: source_value,
        environment,
        settings: json!({
            "warmups": policy.sample_classes.clone(),
            "order": "fixed-manifest-order (single engine)",
            "cachePolicy": policy.cache_policy,
            "cacheEviction": "none; measured samples are warm-file-cache after declared warmups",
            "seed": policy.seed,
            "environmentVariableKeys": base_env.keys().cloned().collect::<Vec<_>>(),
            "environmentIsolation": "Each sample runs with HOME, XDG_*, TMPDIR/TMP/TEMP, and PYTHONPYCACHEPREFIX inside its own temp root; only a fixed safe allowlist of parent variables is inherited; credential-like manifest keys are rejected.",
            "toolCachePreparation": "Only workload-required tools are copied once from the explicitly configured private cache into an owned staging directory before any timed sample. Each sample HOME receives a link at the tools.py cache path (.cache/aidans-patches/apk-lab/tools); the source cache is never modified and nothing is downloaded or installed during the run.",
            "stagedInputs": "Workloads may declare stagedFiles entries naming a declared, hash-verified input role and a fixed relative destination. Every sample verifies the source and copies it into the per-sample workspace before spawn (fs::copy, never a hard link); after exit the source and copy are re-hashed and any mutation invalidates the sample. The private source cache is read-only. Workloads without stagedFiles execute unchanged.",
            "outputNormalization": "Morphe workloads only: replace the numeric value span at $.testCases[*].durationSeconds with null, from the end; every other stdout byte, including options.durationSeconds, order, Unicode escapes, nulls, counts, statuses, and errors, is preserved. Raw stdout/stderr hashes are retained separately.",
            "safeInheritedEnvironmentKeys": SAFE_INHERITED_ENV_KEYS,
        }),
        measurements: json!({
            "wall": {
                "supported": true,
                "unit": "milliseconds",
                "method": "std::time::Instant from spawn through process exit and full stdout/stderr drain; includes timeout kill/reap grace",
            },
            "cpu": metric_value(
                metric_capability.cpu_supported,
                "milliseconds",
                CPU_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "descendantRss": metric_value(
                metric_capability.rss_supported,
                "bytes",
                RSS_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "io": metric_value(
                metric_capability.io_supported,
                "bytes",
                IO_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "limitations": metric_evaluation.limitations,
        }),
        workloads: results,
        acceptance,
        provenance: provenance_value,
    })
}

// ---------------------------------------------------------------------------
// Paired (Python vs native Rust) measurement
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PairedEngine {
    role: &'static str,
    executable: PathBuf,
    cwd: PathBuf,
    args_prefix: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairedEngineResult {
    name: String,
    executable: String,
    version: Option<String>,
    cwd: String,
    args_prefix: Vec<String>,
    direct_execution: bool,
    compilation_included: bool,
    status: String,
    warmups_expected: u32,
    samples_expected: u32,
    warmups_completed: u32,
    samples_completed: u32,
    reasons: Vec<String>,
    samples: Vec<SampleResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairedSamplePair {
    index: u32,
    /// Engine order actually executed for this pair, e.g. ["rust", "python"].
    order: Vec<String>,
    python_index: u32,
    rust_index: u32,
    python_output_valid: bool,
    rust_output_valid: bool,
    exit_codes_match: bool,
    stdout_sha256_match: bool,
    stderr_sha256_match: bool,
    correct: bool,
    mismatches: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairedWorkloadResult {
    id: String,
    category: String,
    size: String,
    costly: bool,
    sample_class: String,
    status: String,
    warmups_expected: u32,
    samples_expected: u32,
    measured_pairs: u32,
    reasons: Vec<String>,
    inputs: Vec<InputStatus>,
    mpp: Option<InputStatus>,
    required_tools: Vec<String>,
    argv_template: Vec<String>,
    environment: BTreeMap<String, String>,
    engines: BTreeMap<String, PairedEngineResult>,
    pairs: Vec<PairedSamplePair>,
    correctness: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    metric_limitations: Vec<MetricLimitation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairedBenchmarkReport {
    schema_version: u32,
    kind: String,
    status: String,
    profile: String,
    generated_at_epoch_seconds: u64,
    duration_ms: f64,
    engines: Value,
    policy: Value,
    source: Value,
    corpus: Value,
    environment: Value,
    settings: Value,
    measurements: Value,
    workloads: Vec<PairedWorkloadResult>,
    acceptance: Acceptance,
    provenance: Value,
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }
}

fn order_seed(policy_seed: u64, workload_id: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in workload_id.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    policy_seed ^ hash
}

/// Alternating randomized order: choose a seeded starting engine per workload,
/// then strictly alternate for every subsequent warmup or measured pair.
fn pair_order(rust_first_base: bool, ordinal: u32) -> Vec<&'static str> {
    let rust_first = if ordinal.is_multiple_of(2) {
        rust_first_base
    } else {
        !rust_first_base
    };
    if rust_first {
        vec!["rust", "python"]
    } else {
        vec!["python", "rust"]
    }
}

fn rust_engine_executable(repo_root: &Path, opts: &Options) -> Result<PathBuf, String> {
    let candidate = opts
        .rust_bin
        .clone()
        .unwrap_or_else(|| PathBuf::from("tools/apk-lab/target/release/apk-lab"));
    let executable = resolve_against(repo_root, &candidate);
    if !executable.is_file() {
        return Err(format!(
            "native release binary not found: {} (build it with `cargo build --release --locked --manifest-path tools/apk-lab/Cargo.toml --bins`)",
            display_path(&executable, repo_root)
        ));
    }
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name == "cargo" || name == "uv" {
        return Err(
            "paired mode must execute a pre-built native binary directly, not cargo/uv".to_string(),
        );
    }
    Ok(executable)
}

/// Runs `bench --engine both`. The report is written before the process exits
/// nonzero so a partial measurement remains inspectable.
fn run_paired(
    repo_root: &Path,
    opts: &Options,
    policy: &PerformancePolicy,
    policy_sha256: &str,
    manifest: &BenchmarkManifest,
    manifest_bytes: &[u8],
) -> Result<(), String> {
    let report = execute_paired_benchmark(
        repo_root,
        opts,
        policy,
        policy_sha256,
        manifest,
        manifest_bytes,
    )?;
    let value = serde_json::to_value(&report)
        .map_err(|e| format!("cannot serialize paired benchmark report: {e}"))?;
    validate_paired_result_shape(&value)?;
    write_json_atomic(&opts.out, &value)?;
    if report.acceptance.valid {
        Ok(())
    } else {
        Err(format!(
            "paired benchmark incomplete: {}",
            report.acceptance.reasons.join("; ")
        ))
    }
}

fn corpus_identity(manifest: &BenchmarkManifest, manifest_bytes: &[u8]) -> Value {
    let workloads: Vec<Value> = manifest
        .workloads
        .iter()
        .map(|workload| {
            json!({
                "id": workload.id,
                "category": workload.category,
                "size": workload.size,
                "costly": workload.costly,
                "argv": workload.argv,
                "inputs": workload.inputs.iter().map(|input| json!({
                    "role": input.role,
                    "sha256": input.sha256,
                })).collect::<Vec<_>>(),
                "mpp": workload.mpp.as_ref().map(|mpp| json!({
                    "role": mpp.role,
                    "sha256": mpp.sha256,
                })),
                "requiredTools": workload.required_tools,
            })
        })
        .collect();
    let canonical = serde_json::to_vec(&workloads).unwrap_or_default();
    json!({
        "profile": manifest.profile,
        "manifestSha256": sha256_bytes(manifest_bytes),
        "identitySha256": sha256_bytes(&canonical),
        "workloads": workloads,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_paired_benchmark(
    repo_root: &Path,
    opts: &Options,
    policy: &PerformancePolicy,
    policy_sha256: &str,
    manifest: &BenchmarkManifest,
    manifest_bytes: &[u8],
) -> Result<PairedBenchmarkReport, String> {
    let started = Instant::now();
    let metric_capability = detect_metric_capability();

    let python_exe = resolve_against(repo_root, Path::new(&manifest.engine.executable));
    let python_cwd = resolve_against(repo_root, Path::new(&manifest.engine.cwd));
    let rust_exe = rust_engine_executable(repo_root, opts)?;
    let rust_cwd = repo_root.to_path_buf();

    let required_tools = required_tool_names(manifest);
    let prepared_tools = prepare_tool_cache(
        repo_root,
        &python_cwd,
        manifest.tool_cache.as_ref(),
        &required_tools,
    )?;
    let failed_tools = prepared_tools.failed_tools();

    let (source, mut provenance_reasons) = collect_source(repo_root);
    let python_version = command_version(&python_exe, "--version");
    let rust_version = command_version(&rust_exe, "--version");
    let git_version = command_version(Path::new("git"), "--version");
    let environment = collect_environment(
        repo_root,
        &python_exe,
        &python_cwd,
        python_version.clone(),
        git_version,
    );

    let rust_hash = hash_file(&rust_exe);
    let rust_identity = match &rust_hash {
        Ok(hash) => json!({
            "name": "rust",
            "executable": display_path(&rust_exe, repo_root),
            "version": rust_version,
            "cwd": display_cwd(&rust_cwd, repo_root),
            "argsPrefix": [],
            "directExecution": true,
            "compilationIncluded": false,
            "binarySha256": hash.sha256,
            "binaryBytes": hash.size_bytes,
        }),
        Err(error) => {
            provenance_reasons.push(format!(
                "native release binary could not be hashed: {error}"
            ));
            json!({
                "name": "rust",
                "executable": display_path(&rust_exe, repo_root),
                "version": rust_version,
                "cwd": display_cwd(&rust_cwd, repo_root),
                "argsPrefix": [],
                "directExecution": true,
                "compilationIncluded": false,
                "binarySha256": Value::Null,
                "binaryBytes": Value::Null,
            })
        }
    };

    let corpus = corpus_identity(manifest, manifest_bytes);
    let base_env = effective_base_env(&manifest.engine.env, policy.seed);
    let private_dir = opts.private_dir.clone().unwrap_or_else(|| {
        let parent = opts.out.parent().unwrap_or_else(|| Path::new("."));
        parent.join("baseline-source")
    });
    let source_archive = archive_source(repo_root, &private_dir, &source);
    if let Err(error) = &source_archive {
        provenance_reasons.push(format!("private source archive failed: {error}"));
    }
    let source_value = source_report_value(&source, &source_archive);
    let provenance_value = json!({
        "schemaVersion": 1,
        "policySha256": policy_sha256,
        "manifestSha256": sha256_bytes(manifest_bytes),
        "manifestStatus": "local-private-manifest-not-committed",
        "sourceRevision": source.revision.clone(),
        "dirty": source.dirty,
        "worktreeDigest": source.digest.clone(),
        "archiveDigest": source_archive.as_ref().map(|info| info.digest.clone()),
        "reconstruction": {
            "executable": "Pre-synced Python interpreter from engine.executable and the pre-built native release binary from --rust-bin, both executed directly.",
            "source": "Restore the private archive exactly as executed and verify its file digests; dirty.patch is evidence, not a patch to re-apply.",
            "inputs": "Verify every input sha256 before executing a workload.",
        },
        "toolchain": tool_cache_report(manifest.tool_cache.as_ref(), &prepared_tools),
    });

    let python_ready = python_exe.is_file();
    if !python_ready {
        provenance_reasons.push(format!(
            "python oracle executable not found: {}",
            display_path(&python_exe, repo_root)
        ));
    }
    if !python_cwd.is_dir() {
        provenance_reasons.push(format!(
            "python oracle cwd not found: {}",
            display_path(&python_cwd, repo_root)
        ));
    }

    let python_engine = PairedEngine {
        role: "python",
        executable: python_exe,
        cwd: python_cwd,
        args_prefix: manifest.engine.args_prefix.clone(),
    };
    let rust_engine = PairedEngine {
        role: "rust",
        executable: rust_exe,
        cwd: rust_cwd,
        args_prefix: Vec::new(),
    };
    let engines_ready = python_engine.executable.is_file() && python_engine.cwd.is_dir();

    let mut results = Vec::with_capacity(manifest.workloads.len());
    for workload in &manifest.workloads {
        let result = benchmark_paired_workload(
            repo_root,
            &python_engine,
            &rust_engine,
            &base_env,
            policy,
            workload,
            &metric_capability,
            engines_ready,
            &failed_tools,
            prepared_tools.cache_root(),
        );
        results.push(result);
    }

    let present: BTreeSet<String> = manifest
        .workloads
        .iter()
        .map(|workload| workload.id.clone())
        .collect();
    let mut acceptance_reasons = provenance_reasons;
    for issue in &prepared_tools.issues {
        acceptance_reasons.push(format!(
            "required tool '{}' was not staged ({})",
            issue.tool, issue.reason
        ));
    }
    let metric_evaluation = evaluate_metric_policy(policy, &metric_capability);
    acceptance_reasons.extend(metric_evaluation.blockers.iter().cloned());
    for result in &mut results {
        if result.measured_pairs > 0 {
            result.metric_limitations = metric_evaluation.limitations.clone();
        }
    }
    for required in &policy.required_workloads {
        if !present.contains(required) {
            acceptance_reasons.push(format!(
                "required workload '{required}' is missing from the manifest"
            ));
        }
    }
    for result in &results {
        if result.status != "measured-paired" {
            let detail = if result.reasons.is_empty() {
                result.status.clone()
            } else {
                result.reasons.join(", ")
            };
            acceptance_reasons.push(format!(
                "workload '{}' is not measured with paired valid outputs ({detail})",
                result.id
            ));
        }
    }
    let acceptance = Acceptance {
        valid: acceptance_reasons.is_empty(),
        reasons: acceptance_reasons,
    };

    Ok(PairedBenchmarkReport {
        schema_version: PAIRED_RESULT_SCHEMA_VERSION,
        kind: "apk-lab-paired-benchmark-result".to_string(),
        status: if acceptance.valid {
            "complete".to_string()
        } else {
            "incomplete".to_string()
        },
        profile: manifest.profile.clone(),
        generated_at_epoch_seconds: now_epoch_seconds(),
        duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        engines: json!({
            "python": {
                "name": "python",
                "executable": display_path(&python_engine.executable, repo_root),
                "version": python_version,
                "cwd": display_path(&python_engine.cwd, repo_root),
                "argsPrefix": python_engine.args_prefix,
                "directExecution": true,
                "uvInstallPerformed": false,
            },
            "rust": rust_identity,
        }),
        policy: json!({
            "id": policy.id.clone(),
            "sha256": policy_sha256,
            "sampleClasses": policy.sample_classes.clone(),
            "budgets": policy.budgets.clone(),
            "requiredWorkloads": policy.required_workloads.clone(),
            "seed": policy.seed,
            "cachePolicy": policy.cache_policy.clone(),
            "order": policy.order.clone(),
        }),
        source: source_value,
        corpus,
        environment,
        settings: json!({
            "sampleClasses": policy.sample_classes.clone(),
            "order": {
                "policy": policy.order.dual_engine.clone(),
                "seed": policy.seed,
                "implementation": "Seeded starting engine per workload, then strict alternation for every warmup and measured pair. The manifest order is never used for timing.",
            },
            "cachePolicy": policy.cache_policy,
            "cacheEviction": "none; measured samples are warm-file-cache after declared warmups",
            "engineIsolation": "Every sample gets a fresh HOME/XDG/TMPDIR sandbox. Python and Rust variants of a pair receive identical input hashes, argv, options, job counts, and sandbox layouts; only the executable differs.",
            "compilation": {
                "included": false,
                "measurement": Value::Null,
                "reason": "The native release binary is pre-built and executed directly. Compilation and cargo/uv convenience overhead are never part of a timed sample.",
            },
            "environmentVariableKeys": base_env.keys().cloned().collect::<Vec<_>>(),
            "toolCachePreparation": "Only workload-required tools are copied once from the explicitly configured private cache into an owned staging directory before any timed sample.",
            "outputNormalization": "Morphe workloads only: replace the numeric value span at $.testCases[*].durationSeconds with null, from the end; every other stdout byte is preserved. Raw hashes are retained separately.",
        }),
        measurements: json!({
            "wall": {
                "supported": true,
                "unit": "milliseconds",
                "method": "std::time::Instant from spawn through process exit and full stdout/stderr drain; includes timeout kill/reap grace",
            },
            "cpu": metric_value(
                metric_capability.cpu_supported,
                "milliseconds",
                CPU_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "descendantRss": metric_value(
                metric_capability.rss_supported,
                "bytes",
                RSS_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "io": metric_value(
                metric_capability.io_supported,
                "bytes",
                IO_METRIC_METHOD,
                metric_capability.reason.as_deref(),
            ),
            "limitations": metric_evaluation.limitations,
        }),
        workloads: results,
        acceptance,
        provenance: provenance_value,
    })
}

#[allow(clippy::too_many_arguments)]
fn benchmark_paired_workload(
    repo_root: &Path,
    python_engine: &PairedEngine,
    rust_engine: &PairedEngine,
    base_env: &BTreeMap<String, String>,
    policy: &PerformancePolicy,
    workload: &WorkloadSpec,
    metric_capability: &MetricCapability,
    engines_ready: bool,
    failed_tools: &BTreeSet<String>,
    tool_cache_root: Option<&Path>,
) -> PairedWorkloadResult {
    let sample_class = if workload.costly { "costly" } else { "local" };
    let class_counts = policy
        .sample_classes
        .get(sample_class)
        .cloned()
        .unwrap_or(SampleClass {
            warmups: 0,
            samples: 0,
        });
    let warmups_expected = workload.warmups.unwrap_or(class_counts.warmups);
    let samples_expected = workload.samples.unwrap_or(class_counts.samples);

    let mut reasons = Vec::new();
    let mut inputs = Vec::new();
    let mut mpp_status = None;
    for input in &workload.inputs {
        inputs.push(verify_input(repo_root, input));
    }
    if let Some(mpp) = &workload.mpp {
        mpp_status = Some(verify_input(repo_root, mpp));
    }
    let input_failed = inputs.iter().any(|input| !input.available)
        || mpp_status
            .as_ref()
            .map(|mpp| !mpp.available)
            .unwrap_or(false);
    if input_failed {
        reasons.push(
            "one or more declared inputs are missing or failed sha256 verification".to_string(),
        );
    }

    let mut environment = base_env.clone();
    for (key, value) in &workload.env {
        environment.insert(key.clone(), value.clone());
    }
    let redacted_environment: BTreeMap<String, String> = environment
        .iter()
        .map(|(key, value)| {
            let shown = if is_secret_key(key) {
                "<redacted>".to_string()
            } else {
                value.clone()
            };
            (key.clone(), shown)
        })
        .collect();

    let mut engines = BTreeMap::new();
    for engine in [python_engine, rust_engine] {
        engines.insert(
            engine.role.to_string(),
            PairedEngineResult {
                name: engine.role.to_string(),
                executable: display_path(&engine.executable, repo_root),
                version: command_version(&engine.executable, "--version"),
                cwd: display_cwd(&engine.cwd, repo_root),
                args_prefix: engine.args_prefix.clone(),
                direct_execution: true,
                compilation_included: false,
                status: "incomplete".to_string(),
                warmups_expected,
                samples_expected,
                warmups_completed: 0,
                samples_completed: 0,
                reasons: Vec::new(),
                samples: Vec::new(),
            },
        );
    }
    let mut result = PairedWorkloadResult {
        id: workload.id.clone(),
        category: workload.category.clone(),
        size: workload.size.clone(),
        costly: workload.costly,
        sample_class: sample_class.to_string(),
        status: "incomplete".to_string(),
        warmups_expected,
        samples_expected,
        measured_pairs: 0,
        reasons,
        inputs,
        mpp: mpp_status,
        required_tools: workload.required_tools.clone(),
        argv_template: workload.argv.clone(),
        environment: redacted_environment,
        engines,
        pairs: Vec::new(),
        correctness: json!({
            "matched": false,
            "mismatchedPairs": 0,
            "checkedPerSample": true,
            "checks": ["exitCode", "stdoutSha256", "stderrSha256", "expectedFiles", "expectedExitCode", "stdoutContains", "stderrContains"],
        }),
        metric_limitations: Vec::new(),
    };

    if !engines_ready {
        result.reasons.push(
            "python oracle or native release binary is unavailable; workload was not executed"
                .to_string(),
        );
        for engine in result.engines.values_mut() {
            engine.status = "unavailable".to_string();
        }
        return result;
    }
    if input_failed {
        return result;
    }
    let unavailable_tools: Vec<&String> = workload
        .required_tools
        .iter()
        .filter(|tool| failed_tools.contains(*tool))
        .collect();
    if !unavailable_tools.is_empty() {
        result.reasons.push(format!(
            "required tool(s) {} were not staged from the pinned private cache; workload was not executed",
            unavailable_tools
                .iter()
                .map(|tool| tool.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        return result;
    }
    let sample_tool_cache = if workload.required_tools.is_empty() {
        None
    } else {
        tool_cache_root
    };
    let timeout = Duration::from_secs(workload.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));
    let mut rng = SplitMix64::new(order_seed(policy.seed, &workload.id));
    let rust_first_base = rng.next_u64() & 1 == 1;
    let mut any_unpinned = false;
    let mut hard_failure = false;
    let engines = [python_engine, rust_engine];

    'warmups: for index in 0..warmups_expected {
        let order = pair_order(rust_first_base, index);
        for role in order {
            let engine = engines
                .iter()
                .find(|engine| engine.role == role)
                .expect("declared engine role must exist");
            match run_paired_sample(
                repo_root,
                engine,
                &environment,
                workload,
                metric_capability,
                timeout,
                sample_tool_cache,
            ) {
                Ok(run) => {
                    let mut sample = run.sample;
                    sample.kind = "warmup".to_string();
                    sample.cache_state = "warmup".to_string();
                    sample.index = index;
                    any_unpinned |= run.unpinned;
                    let valid = run.hard_errors.is_empty();
                    let engine_result = result
                        .engines
                        .get_mut(role)
                        .expect("engine result must exist");
                    engine_result.samples.push(sample);
                    engine_result.warmups_completed += 1;
                    if !valid {
                        let errors: Vec<String> = run
                            .hard_errors
                            .iter()
                            .map(|error| sanitise_report_message(error, repo_root, &PathBuf::new()))
                            .collect();
                        engine_result.reasons.extend(errors.clone());
                        result.reasons.extend(errors);
                        hard_failure = true;
                        break 'warmups;
                    }
                }
                Err(error) => {
                    let message = sanitise_report_message(&error, repo_root, &PathBuf::new());
                    if let Some(engine_result) = result.engines.get_mut(role) {
                        engine_result.reasons.push(message.clone());
                    }
                    result
                        .reasons
                        .push(format!("warmup {index} ({role}) failed: {message}"));
                    hard_failure = true;
                    break 'warmups;
                }
            }
        }
    }

    if !hard_failure {
        'measured: for index in 0..samples_expected {
            let order = pair_order(rust_first_base, index);
            let order_names: Vec<String> = order.iter().map(|role| role.to_string()).collect();
            let mut pair = PairedSamplePair {
                index,
                order: order_names,
                python_index: index,
                rust_index: index,
                python_output_valid: false,
                rust_output_valid: false,
                exit_codes_match: false,
                stdout_sha256_match: false,
                stderr_sha256_match: false,
                correct: false,
                mismatches: Vec::new(),
            };
            let mut pair_runs: BTreeMap<&'static str, SampleRun> = BTreeMap::new();
            for role in order {
                let engine = engines
                    .iter()
                    .find(|engine| engine.role == role)
                    .expect("declared engine role must exist");
                match run_paired_sample(
                    repo_root,
                    engine,
                    &environment,
                    workload,
                    metric_capability,
                    timeout,
                    sample_tool_cache,
                ) {
                    Ok(run) => {
                        any_unpinned |= run.unpinned;
                        pair_runs.insert(role, run);
                    }
                    Err(error) => {
                        let message = sanitise_report_message(&error, repo_root, &PathBuf::new());
                        if let Some(engine_result) = result.engines.get_mut(role) {
                            engine_result.reasons.push(message.clone());
                        }
                        result
                            .reasons
                            .push(format!("measured pair {index} ({role}) failed: {message}"));
                        hard_failure = true;
                        break;
                    }
                }
            }
            if hard_failure {
                break 'measured;
            }
            for (role, run) in pair_runs {
                let mut sample = run.sample;
                sample.kind = "measured".to_string();
                sample.cache_state = "warm-file-cache".to_string();
                sample.index = index;
                let valid = run.hard_errors.is_empty();
                let engine_result = result
                    .engines
                    .get_mut(role)
                    .expect("engine result must exist");
                engine_result.samples.push(sample);
                if valid {
                    engine_result.samples_completed += 1;
                } else {
                    let errors: Vec<String> = run
                        .hard_errors
                        .iter()
                        .map(|error| sanitise_report_message(error, repo_root, &PathBuf::new()))
                        .collect();
                    engine_result.reasons.extend(errors.clone());
                    result.reasons.extend(errors);
                    hard_failure = true;
                }
                if role == "python" {
                    pair.python_output_valid = valid;
                } else if role == "rust" {
                    pair.rust_output_valid = valid;
                }
            }
            if hard_failure {
                break 'measured;
            }
            // Locate the two samples by measured ordinal to compare semantics.
            let python_sample = result.engines["python"]
                .samples
                .iter()
                .find(|sample| sample.kind == "measured" && sample.index == index);
            let rust_sample = result.engines["rust"]
                .samples
                .iter()
                .find(|sample| sample.kind == "measured" && sample.index == index);
            let (Some(python_sample), Some(rust_sample)) = (python_sample, rust_sample) else {
                result
                    .reasons
                    .push(format!("measured pair {index} is missing a sample"));
                hard_failure = true;
                break 'measured;
            };
            pair.exit_codes_match = python_sample.exit_code == rust_sample.exit_code;
            pair.stdout_sha256_match = python_sample.stdout_sha256 == rust_sample.stdout_sha256;
            pair.stderr_sha256_match = python_sample.stderr_sha256 == rust_sample.stderr_sha256;
            if !pair.exit_codes_match {
                pair.mismatches.push(format!(
                    "exit code differs: python {:?}, rust {:?}",
                    python_sample.exit_code, rust_sample.exit_code
                ));
            }
            if !pair.stdout_sha256_match {
                pair.mismatches
                    .push("normalized stdout sha256 differs".to_string());
            }
            if !pair.stderr_sha256_match {
                pair.mismatches
                    .push("normalized stderr sha256 differs".to_string());
            }
            pair.correct = pair.python_output_valid
                && pair.rust_output_valid
                && pair.exit_codes_match
                && pair.stdout_sha256_match
                && pair.stderr_sha256_match;
            if !pair.correct {
                let detail = if pair.mismatches.is_empty() {
                    "one or both samples failed declared output validation".to_string()
                } else {
                    pair.mismatches.join("; ")
                };
                result
                    .reasons
                    .push(format!("measured pair {index} incorrect: {detail}"));
                hard_failure = true;
            }
            result.measured_pairs += 1;
            result.pairs.push(pair);
            if hard_failure {
                break 'measured;
            }
        }
    }

    if hard_failure {
        result.status = "invalid-output".to_string();
    } else if result
        .engines
        .values()
        .any(|engine| engine.warmups_completed < warmups_expected)
        || result
            .engines
            .values()
            .any(|engine| engine.samples_completed < samples_expected)
        || result.pairs.len() < samples_expected as usize
    {
        result.status = "incomplete".to_string();
        result.reasons.push(format!(
            "sample counts incomplete: pairs {}/{}, python {}/{}, rust {}/{}",
            result.pairs.len(),
            samples_expected,
            result.engines["python"].samples_completed,
            samples_expected,
            result.engines["rust"].samples_completed,
            samples_expected
        ));
    } else if any_unpinned {
        result.status = "measured-unverified".to_string();
        result.reasons.push(
            "expected.stdoutSha256 is not pinned; observed hashes were recorded but correctness is not verified"
                .to_string(),
        );
    } else {
        result.status = "measured-paired".to_string();
    }

    let mismatched = result.pairs.iter().filter(|pair| !pair.correct).count();
    result.correctness = json!({
        "matched": mismatched == 0 && !result.pairs.is_empty(),
        "mismatchedPairs": mismatched,
        "checkedPerSample": true,
        "checks": ["exitCode", "stdoutSha256", "stderrSha256", "expectedFiles", "expectedExitCode", "stdoutContains"],
        "crossEngine": ["exitCode", "stdoutSha256", "stderrSha256"],
    });
    for engine in result.engines.values_mut() {
        engine.status = if engine.samples_completed == samples_expected
            && engine.warmups_completed == warmups_expected
        {
            "measured".to_string()
        } else if engine.samples_completed == 0 {
            "incomplete".to_string()
        } else {
            "partial".to_string()
        };
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_paired_sample(
    repo_root: &Path,
    engine: &PairedEngine,
    environment: &BTreeMap<String, String>,
    workload: &WorkloadSpec,
    metric_capability: &MetricCapability,
    timeout: Duration,
    tool_cache_root: Option<&Path>,
) -> Result<SampleRun, String> {
    run_one(
        repo_root,
        &engine.executable,
        &engine.cwd,
        &engine.args_prefix,
        environment,
        workload,
        metric_capability,
        timeout,
        tool_cache_root,
    )
}

fn validate_paired_result_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "paired benchmark result must be a JSON object".to_string())?;
    if object.get("schemaVersion").and_then(Value::as_u64)
        != Some(PAIRED_RESULT_SCHEMA_VERSION as u64)
    {
        return Err("paired benchmark result schemaVersion is missing or wrong".to_string());
    }
    if object.get("kind").and_then(Value::as_str) != Some("apk-lab-paired-benchmark-result") {
        return Err("paired benchmark result kind is missing or wrong".to_string());
    }
    match object.get("status").and_then(Value::as_str) {
        Some("complete") | Some("incomplete") => {}
        _ => {
            return Err(
                "paired benchmark result status must be 'complete' or 'incomplete'".to_string(),
            )
        }
    }
    let engines = object
        .get("engines")
        .and_then(Value::as_object)
        .ok_or_else(|| "paired benchmark result engines must be an object".to_string())?;
    for engine in ["python", "rust"] {
        if !engines.get(engine).map(Value::is_object).unwrap_or(false) {
            return Err(format!(
                "paired benchmark result engines.{engine} is missing"
            ));
        }
    }
    if !object.get("corpus").map(Value::is_object).unwrap_or(false) {
        return Err("paired benchmark result corpus must be an object".to_string());
    }
    let workloads = object
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "paired benchmark result workloads must be an array".to_string())?;
    for workload in workloads {
        let id = workload
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "paired workload id is missing".to_string())?;
        let engines = workload
            .get("engines")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("paired workload '{id}' engines must be an object"))?;
        for engine in ["python", "rust"] {
            let samples = engines
                .get(engine)
                .and_then(|value| value.get("samples"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!("paired workload '{id}' engines.{engine}.samples must be an array")
                })?;
            if engine == "rust" && samples.is_empty() && workloads.len() == 1 {
                // An unavailable engine is allowed; acceptance carries the reason.
            }
        }
        if !workload.get("pairs").map(Value::is_array).unwrap_or(false) {
            return Err(format!("paired workload '{id}' pairs must be an array"));
        }
    }
    if object
        .get("acceptance")
        .and_then(|value| value.get("valid"))
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err("paired benchmark result acceptance.valid must be a boolean".to_string());
    }
    Ok(())
}

/// Replace host-private roots before a message reaches a committed report.
fn sanitise_report_message(message: &str, repo_root: &Path, private_dir: &Path) -> String {
    let mut result = message.to_string();
    for (path, replacement) in [(private_dir, "<private-dir>"), (repo_root, "<repo>")] {
        if let Some(text) = path.to_str() {
            if !text.is_empty() {
                result = result.replace(text, replacement);
            }
        }
    }
    if let Some(home) = invoking_home() {
        if let Some(text) = home.to_str() {
            if !text.is_empty() && text != "/" {
                result = result.replace(text, "~");
            }
        }
    }
    result
}

/// Output-pin reports always carry an array. A workload without a
/// normalization rule has no entries; a morphe workload carries the exact
/// timing-only rule object `run_one` applied.
fn normalization_pins(value: &Value) -> Vec<Value> {
    if value.is_null() {
        Vec::new()
    } else {
        vec![value.clone()]
    }
}

/// Prepare accepted output pins for every workload selected by the manifest.
///
/// This deliberately performs one unmeasured execution per workload. It reuses
/// the normal isolated `run_one` path, so input hashes, exit codes, configured
/// output hashes, `stdoutContains`, expected files, and morphe duration
/// normalization are validated exactly as during a timed run. Unpinned expected
/// hashes are intentional; they are not a validation failure.
fn execute_prepare(
    repo_root: &Path,
    opts: &Options,
    policy: &PerformancePolicy,
    manifest: &BenchmarkManifest,
    manifest_bytes: &[u8],
) -> Result<OutputPinsReport, String> {
    let metric_capability = detect_metric_capability();
    let engine_exe = resolve_against(repo_root, Path::new(&manifest.engine.executable));
    let engine_cwd = resolve_against(repo_root, Path::new(&manifest.engine.cwd));
    let private_dir = opts.private_dir.clone().unwrap_or_else(|| {
        let parent = opts.out.parent().unwrap_or_else(|| Path::new("."));
        parent.join("baseline-source")
    });

    let required_tools = required_tool_names(manifest);
    let mut acceptance_reasons = Vec::new();
    let prepared_tools = match prepare_tool_cache(
        repo_root,
        &engine_cwd,
        manifest.tool_cache.as_ref(),
        &required_tools,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            acceptance_reasons.push(sanitise_report_message(
                &format!("tool cache preflight failed before execution: {error}"),
                repo_root,
                &private_dir,
            ));
            PreparedToolCache {
                root: None,
                tools: Vec::new(),
                issues: required_tools
                    .iter()
                    .map(|tool| ToolCacheIssue {
                        tool: tool.clone(),
                        reason: "tool cache preflight failed before execution".to_string(),
                    })
                    .collect(),
            }
        }
    };
    let failed_tools = prepared_tools.failed_tools();

    let (source, mut provenance_reasons) = collect_source(repo_root);
    let source_archive = archive_source(repo_root, &private_dir, &source);
    if let Err(error) = &source_archive {
        provenance_reasons.push(format!("private source archive failed: {error}"));
    }
    let source_value = source_report_value(&source, &source_archive);
    for reason in &mut provenance_reasons {
        *reason = sanitise_report_message(reason, repo_root, &private_dir);
    }
    acceptance_reasons.extend(provenance_reasons);

    let executable_ready = engine_exe.is_file();
    let cwd_ready = engine_cwd.is_dir();
    if !executable_ready {
        acceptance_reasons.push(sanitise_report_message(
            &format!(
                "python oracle executable not found: {}",
                display_path(&engine_exe, repo_root)
            ),
            repo_root,
            &private_dir,
        ));
    }
    if !cwd_ready {
        acceptance_reasons.push(sanitise_report_message(
            &format!(
                "python oracle cwd not found: {}",
                display_path(&engine_cwd, repo_root)
            ),
            repo_root,
            &private_dir,
        ));
    }
    let oracle_ready = executable_ready && cwd_ready;

    let base_env = effective_base_env(&manifest.engine.env, policy.seed);
    let mut workloads = Vec::with_capacity(manifest.workloads.len());
    for workload in &manifest.workloads {
        workloads.push(prepare_workload(
            repo_root,
            &engine_exe,
            &engine_cwd,
            &manifest.engine.args_prefix,
            &base_env,
            workload,
            &metric_capability,
            oracle_ready,
            &failed_tools,
            prepared_tools.cache_root(),
            &private_dir,
        ));
    }

    for issue in &prepared_tools.issues {
        acceptance_reasons.push(sanitise_report_message(
            &format!(
                "required tool '{}' was not staged ({})",
                issue.tool, issue.reason
            ),
            repo_root,
            &private_dir,
        ));
    }
    for workload in &workloads {
        if workload.status == "prepared" {
            continue;
        }
        let detail = if workload.validation_errors.is_empty() {
            workload.status.clone()
        } else {
            workload.validation_errors.join(", ")
        };
        acceptance_reasons.push(format!(
            "workload '{}' is {} ({detail})",
            workload.id, workload.status
        ));
    }

    let acceptance = Acceptance {
        valid: acceptance_reasons.is_empty(),
        reasons: acceptance_reasons,
    };
    let status = if acceptance.valid {
        "complete"
    } else {
        "incomplete"
    };

    Ok(OutputPinsReport {
        schema_version: PINS_SCHEMA_VERSION,
        kind: "apk-lab-benchmark-output-pins".to_string(),
        status: status.to_string(),
        engine: "python".to_string(),
        profile: manifest.profile.clone(),
        manifest_sha256: sha256_bytes(manifest_bytes),
        source: source_value,
        workloads,
        acceptance,
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_workload(
    repo_root: &Path,
    engine_exe: &Path,
    engine_cwd: &Path,
    args_prefix: &[String],
    base_env: &BTreeMap<String, String>,
    workload: &WorkloadSpec,
    metric_capability: &MetricCapability,
    oracle_ready: bool,
    failed_tools: &BTreeSet<String>,
    tool_cache_root: Option<&Path>,
    private_dir: &Path,
) -> WorkloadPins {
    let mut validation_errors = Vec::new();
    let mut inputs_available = true;
    for input in &workload.inputs {
        let status = verify_input(repo_root, input);
        if !status.available {
            inputs_available = false;
            validation_errors.push(sanitise_report_message(
                status
                    .reason
                    .as_deref()
                    .unwrap_or("declared input is unavailable"),
                repo_root,
                private_dir,
            ));
        }
    }
    if let Some(mpp) = &workload.mpp {
        let status = verify_input(repo_root, mpp);
        if !status.available {
            inputs_available = false;
            validation_errors.push(sanitise_report_message(
                status
                    .reason
                    .as_deref()
                    .unwrap_or("declared mpp input is unavailable"),
                repo_root,
                private_dir,
            ));
        }
    }
    if !oracle_ready {
        validation_errors
            .push("pre-synced python oracle is unavailable; workload was not executed".to_string());
    }
    if !oracle_ready || !inputs_available {
        return WorkloadPins {
            id: workload.id.clone(),
            status: "incomplete".to_string(),
            exit_code: None,
            stdout_sha256: None,
            stderr_sha256: None,
            raw_stdout_sha256: None,
            raw_stderr_sha256: None,
            output_normalization: Vec::new(),
            validation_errors,
            diagnostics: None,
        };
    }

    let unavailable_tools: Vec<&String> = workload
        .required_tools
        .iter()
        .filter(|tool| failed_tools.contains(*tool))
        .collect();
    if !unavailable_tools.is_empty() {
        validation_errors.push(format!(
            "required tool(s) {} were not staged from the pinned private cache; workload was not executed",
            unavailable_tools
                .iter()
                .map(|tool| tool.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        return WorkloadPins {
            id: workload.id.clone(),
            status: "incomplete".to_string(),
            exit_code: None,
            stdout_sha256: None,
            stderr_sha256: None,
            raw_stdout_sha256: None,
            raw_stderr_sha256: None,
            output_normalization: Vec::new(),
            validation_errors,
            diagnostics: None,
        };
    }

    let mut environment = base_env.clone();
    for (key, value) in &workload.env {
        environment.insert(key.clone(), value.clone());
    }
    let sample_tool_cache = if workload.required_tools.is_empty() {
        None
    } else {
        tool_cache_root
    };
    let timeout = Duration::from_secs(workload.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

    match run_one(
        repo_root,
        engine_exe,
        engine_cwd,
        args_prefix,
        &environment,
        workload,
        metric_capability,
        timeout,
        sample_tool_cache,
    ) {
        Ok(run) => {
            let output_normalization = normalization_pins(&run.sample.output_normalization);
            let errors: Vec<String> = run
                .hard_errors
                .iter()
                .map(|error| sanitise_report_message(error, repo_root, private_dir))
                .collect();
            if errors.is_empty() {
                WorkloadPins {
                    id: workload.id.clone(),
                    status: "prepared".to_string(),
                    exit_code: run.sample.exit_code,
                    stdout_sha256: Some(run.sample.stdout_sha256),
                    stderr_sha256: Some(run.sample.stderr_sha256),
                    raw_stdout_sha256: Some(run.sample.raw_stdout_sha256),
                    raw_stderr_sha256: Some(run.sample.raw_stderr_sha256),
                    output_normalization,
                    validation_errors: Vec::new(),
                    diagnostics: None,
                }
            } else {
                WorkloadPins {
                    id: workload.id.clone(),
                    status: "failed".to_string(),
                    exit_code: run.sample.exit_code,
                    stdout_sha256: None,
                    stderr_sha256: None,
                    raw_stdout_sha256: None,
                    raw_stderr_sha256: None,
                    output_normalization,
                    validation_errors: errors,
                    diagnostics: run.sample.diagnostics,
                }
            }
        }
        Err(error) => WorkloadPins {
            id: workload.id.clone(),
            status: "incomplete".to_string(),
            exit_code: None,
            stdout_sha256: None,
            stderr_sha256: None,
            raw_stdout_sha256: None,
            raw_stderr_sha256: None,
            output_normalization: Vec::new(),
            validation_errors: vec![sanitise_report_message(&error, repo_root, private_dir)],
            diagnostics: None,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn benchmark_workload(
    repo_root: &Path,
    engine_exe: &Path,
    engine_cwd: &Path,
    args_prefix: &[String],
    base_env: &BTreeMap<String, String>,
    policy: &PerformancePolicy,
    workload: &WorkloadSpec,
    metric_capability: &MetricCapability,
    oracle_ready: bool,
    failed_tools: &BTreeSet<String>,
    tool_cache_root: Option<&Path>,
) -> WorkloadResult {
    let sample_class = if workload.costly { "costly" } else { "local" };
    let class_counts = policy
        .sample_classes
        .get(sample_class)
        .cloned()
        .unwrap_or(SampleClass {
            warmups: 0,
            samples: 0,
        });
    let warmups_expected = workload.warmups.unwrap_or(class_counts.warmups);
    let samples_expected = workload.samples.unwrap_or(class_counts.samples);

    let mut reasons = Vec::new();
    let mut inputs = Vec::new();
    let mut mpp_status = None;
    for input in &workload.inputs {
        inputs.push(verify_input(repo_root, input));
    }
    if let Some(mpp) = &workload.mpp {
        let status = verify_input(repo_root, mpp);
        mpp_status = Some(status);
    }
    let input_failed = inputs.iter().any(|input| !input.available)
        || mpp_status
            .as_ref()
            .map(|mpp| !mpp.available)
            .unwrap_or(false);
    if input_failed {
        reasons.push(
            "one or more declared inputs are missing or failed sha256 verification".to_string(),
        );
    }

    let mut environment = base_env.clone();
    for (key, value) in &workload.env {
        environment.insert(key.clone(), value.clone());
    }
    let redacted_environment: BTreeMap<String, String> = environment
        .iter()
        .map(|(key, value)| {
            let shown = if is_secret_key(key) {
                "<redacted>".to_string()
            } else {
                value.clone()
            };
            (key.clone(), shown)
        })
        .collect();

    let mut result = WorkloadResult {
        id: workload.id.clone(),
        category: workload.category.clone(),
        size: workload.size.clone(),
        costly: workload.costly,
        status: "incomplete".to_string(),
        sample_class: sample_class.to_string(),
        warmups_expected,
        samples_expected,
        warmups_completed: 0,
        samples_completed: 0,
        reasons,
        inputs,
        mpp: mpp_status,
        required_tools: workload.required_tools.clone(),
        argv_template: workload.argv.clone(),
        environment: redacted_environment,
        samples: Vec::new(),
        summary: None,
        output_validation: json!({
            "stdoutHashPinned": workload.expected.stdout_sha256.is_some(),
            "stderrHashPinned": workload.expected.stderr_sha256.is_some(),
            "allOutputsValid": false,
        }),
        output_normalization: if workload.category == "morphe" {
            json!({
                "rule": "morphe-test-case-duration-seconds",
                "path": "$.testCases[*].durationSeconds",
                "replacement": "null",
                "scope": "category == 'morphe' only",
                "preserved": "all other stdout bytes, order, Unicode escapes, nulls, counts, statuses, options, and errors",
            })
        } else {
            Value::Null
        },
        metric_limitations: Vec::new(),
    };

    if !oracle_ready {
        result
            .reasons
            .push("pre-synced python oracle is unavailable; workload was not executed".to_string());
        return result;
    }
    if input_failed {
        return result;
    }
    let unavailable_tools: Vec<&String> = workload
        .required_tools
        .iter()
        .filter(|tool| failed_tools.contains(*tool))
        .collect();
    if !unavailable_tools.is_empty() {
        result.reasons.push(format!(
            "required tool(s) {} were not staged from the pinned private cache; workload was not executed",
            unavailable_tools
                .iter()
                .map(|tool| tool.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        return result;
    }
    let sample_tool_cache = if workload.required_tools.is_empty() {
        None
    } else {
        tool_cache_root
    };

    let timeout = Duration::from_secs(workload.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));
    let mut hard_failure = false;
    let mut any_unpinned = false;

    'warmups: for index in 0..warmups_expected {
        match run_one(
            repo_root,
            engine_exe,
            engine_cwd,
            args_prefix,
            &environment,
            workload,
            metric_capability,
            timeout,
            sample_tool_cache,
        ) {
            Ok(run) => {
                let mut sample = run.sample;
                sample.kind = "warmup".to_string();
                sample.cache_state = "warmup".to_string();
                sample.index = index;
                any_unpinned |= run.unpinned;
                let valid = run.hard_errors.is_empty();
                result.samples.push(sample);
                result.warmups_completed += 1;
                if !valid {
                    result.reasons.extend(run.hard_errors);
                    hard_failure = true;
                    break 'warmups;
                }
            }
            Err(error) => {
                result
                    .reasons
                    .push(format!("warmup {index} failed: {error}"));
                hard_failure = true;
                break 'warmups;
            }
        }
    }

    if !hard_failure {
        'measured: for index in 0..samples_expected {
            match run_one(
                repo_root,
                engine_exe,
                engine_cwd,
                args_prefix,
                &environment,
                workload,
                metric_capability,
                timeout,
                sample_tool_cache,
            ) {
                Ok(run) => {
                    let mut sample = run.sample;
                    sample.kind = "measured".to_string();
                    sample.cache_state = "warm-file-cache".to_string();
                    sample.index = index;
                    any_unpinned |= run.unpinned;
                    let valid = run.hard_errors.is_empty();
                    result.samples.push(sample);
                    if valid {
                        result.samples_completed += 1;
                    } else {
                        result.reasons.extend(run.hard_errors);
                        hard_failure = true;
                        break 'measured;
                    }
                }
                Err(error) => {
                    result
                        .reasons
                        .push(format!("measured sample {index} failed: {error}"));
                    hard_failure = true;
                    break 'measured;
                }
            }
        }
    }

    if hard_failure {
        result.status = "invalid-output".to_string();
    } else if result.warmups_completed < warmups_expected
        || result.samples_completed < samples_expected
    {
        result.status = "incomplete".to_string();
        result.reasons.push(format!(
            "sample counts incomplete: warmups {}/{}, samples {}/{}",
            result.warmups_completed, warmups_expected, result.samples_completed, samples_expected
        ));
    } else if any_unpinned {
        result.status = "measured-unverified".to_string();
        result.reasons.push(
            "expected.stdoutSha256 is not pinned; observed hashes were recorded but correctness is not verified"
                .to_string(),
        );
    } else {
        result.status = "measured".to_string();
    }

    result.summary = wall_summary(&result.samples);
    result.output_validation = json!({
        "stdoutHashPinned": workload.expected.stdout_sha256.is_some(),
        "stderrHashPinned": workload.expected.stderr_sha256.is_some(),
        "allOutputsValid": result.status == "measured",
    });
    result
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    repo_root: &Path,
    engine_exe: &Path,
    engine_cwd: &Path,
    args_prefix: &[String],
    environment: &BTreeMap<String, String>,
    workload: &WorkloadSpec,
    metric_capability: &MetricCapability,
    timeout: Duration,
    tool_cache_root: Option<&Path>,
) -> Result<SampleRun, String> {
    let temp = TempRoot::new()?;
    let tmp_dir = temp.path.join("tmp");
    let workspace = temp.path.join("workspace");
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("cannot create benchmark tmp dir: {e}"))?;
    fs::create_dir_all(&workspace)
        .map_err(|e| format!("cannot create benchmark workspace dir: {e}"))?;
    for relative in ["home", "xdg/config", "xdg/cache", "xdg/data", "pycache"] {
        fs::create_dir_all(temp.path.join(relative))
            .map_err(|e| format!("cannot create benchmark isolation directory: {e}"))?;
    }
    if let Some(cache_root) = tool_cache_root {
        install_sample_tool_cache(&temp.path.join("home"), cache_root)?;
    }
    // Copy declared, hash-verified originals into the sample workspace before
    // the process exists. The user's cache is only read, never linked or
    // written. Workloads without stagedFiles take the untouched path.
    let staged_inputs = stage_declared_inputs(
        repo_root,
        &workload.inputs,
        &workload.staged_files,
        &workspace,
    )?;

    let mut argv: Vec<String> = Vec::with_capacity(workload.argv.len() + 2);
    let uses_workspace_placeholder = workload.argv.iter().any(|arg| arg.contains("{workspace}"));
    let declares_workspace_root = workload.argv.iter().any(|arg| arg == "--workspace-root");
    if workload.isolate_workspace && !uses_workspace_placeholder && !declares_workspace_root {
        argv.push("--workspace-root".to_string());
        argv.push(workspace.display().to_string());
    }
    for arg in &workload.argv {
        argv.push(resolve_placeholder(arg, &workspace, &tmp_dir, repo_root));
    }

    let outcome = measure_process(
        engine_exe,
        engine_cwd,
        args_prefix,
        &argv,
        environment,
        &temp.path,
        metric_capability,
        timeout,
    )?;
    let normalizers = normalizer_pairs(repo_root, engine_cwd, &temp.path, &workspace);
    let raw_stdout_sha256 = sha256_bytes(&outcome.stdout);
    let raw_stderr_sha256 = sha256_bytes(&outcome.stderr);
    let mut stdout_text = normalise(&String::from_utf8_lossy(&outcome.stdout), &normalizers);
    let stderr_text = normalise(&String::from_utf8_lossy(&outcome.stderr), &normalizers);
    // A workload must not mutate its private source or its own staged copy.
    let mut hard_errors = verify_staged_inputs(&staged_inputs);
    let mut duration_replacements = 0usize;
    let mut duration_applied = false;
    let mut duration_error: Option<String> = None;
    if workload.category == "morphe" {
        match normalize_morphe_durations(&stdout_text) {
            Ok((normalized, replacements)) => {
                stdout_text = normalized;
                duration_replacements = replacements;
                duration_applied = true;
            }
            Err(error) => {
                duration_error = Some(error.clone());
                hard_errors.push(format!("morphe duration normalization rejected: {error}"));
            }
        }
    }
    let stdout_sha256 = sha256_bytes(stdout_text.as_bytes());
    let stderr_sha256 = sha256_bytes(stderr_text.as_bytes());
    let output_normalization = if workload.category == "morphe" {
        json!({
            "rule": "morphe-test-case-duration-seconds",
            "path": "$.testCases[*].durationSeconds",
            "replacement": "null",
            "applied": duration_applied,
            "replacedValues": duration_replacements,
            "error": duration_error,
        })
    } else {
        Value::Null
    };

    if outcome.timed_out {
        hard_errors.push(format!(
            "process exceeded timeout of {} seconds",
            timeout.as_secs()
        ));
    }
    if outcome.exit_code != Some(workload.expected.exit_code) {
        hard_errors.push(format!(
            "exit code mismatch: expected {}, got {:?}",
            workload.expected.exit_code, outcome.exit_code
        ));
    }
    if let Some(expected) = &workload.expected.stdout_sha256 {
        if expected.as_str() != stdout_sha256.as_str() {
            hard_errors.push(format!(
                "stdout sha256 mismatch: expected {expected}, got {stdout_sha256}"
            ));
        }
    }
    if let Some(expected) = &workload.expected.stderr_sha256 {
        if expected.as_str() != stderr_sha256.as_str() {
            hard_errors.push(format!(
                "stderr sha256 mismatch: expected {expected}, got {stderr_sha256}"
            ));
        }
    }
    for needle in &workload.expected.stdout_contains {
        if !stdout_text.contains(needle) {
            hard_errors.push(format!("stdout does not contain required text {needle:?}"));
        }
    }
    for file in &workload.expected.files {
        let resolved = PathBuf::from(resolve_placeholder(
            &file.path, &workspace, &tmp_dir, repo_root,
        ));
        let resolved = if resolved.is_absolute() {
            resolved
        } else {
            repo_root.join(resolved)
        };
        match verify_file_hash(&resolved, &file.sha256) {
            Ok(_) => {}
            Err(error) => {
                hard_errors.push(format!("expected output file '{}': {error}", file.path))
            }
        }
    }

    let diagnostics = if hard_errors.is_empty() {
        None
    } else {
        Some(output_diagnostics(
            &stdout_text,
            &stderr_text,
            repo_root,
            engine_cwd,
            &temp.path,
            &workspace,
            tool_cache_root,
        ))
    };

    let pinned = workload.expected.stdout_sha256.is_some();
    let sample = SampleResult {
        kind: "sample".to_string(),
        cache_state: "unset".to_string(),
        index: 0,
        wall_ms: outcome.wall_ms,
        cpu_ms: outcome.metrics.cpu_ms,
        peak_child_rss_bytes: outcome.metrics.peak_child_rss_bytes,
        read_bytes: outcome.metrics.read_bytes,
        write_bytes: outcome.metrics.write_bytes,
        exit_code: outcome.exit_code,
        timed_out: outcome.timed_out,
        stdout_sha256,
        stderr_sha256,
        raw_stdout_sha256,
        raw_stderr_sha256,
        stdout_bytes: outcome.stdout.len(),
        stderr_bytes: outcome.stderr.len(),
        output_valid: hard_errors.is_empty(),
        outputs_pinned: pinned,
        output_normalization,
        validation_errors: hard_errors.clone(),
        diagnostics,
    };
    Ok(SampleRun {
        sample,
        hard_errors,
        unpinned: !pinned,
    })
}

const SAFE_INHERITED_ENV_KEYS: &[&str] = &[
    "PATH",
    "SYSTEMROOT",
    "PATHEXT",
    "JAVA_HOME",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "ANDROID_NDK_HOME",
    "ANDROID_NDK_ROOT",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
];

#[allow(clippy::too_many_arguments)]
fn measure_process(
    engine_exe: &Path,
    engine_cwd: &Path,
    args_prefix: &[String],
    argv: &[String],
    environment: &BTreeMap<String, String>,
    sandbox_root: &Path,
    metric_capability: &MetricCapability,
    timeout: Duration,
) -> Result<RunOutcome, String> {
    let home = sandbox_root.join("home");
    let xdg_config = sandbox_root.join("xdg/config");
    let xdg_cache = sandbox_root.join("xdg/cache");
    let xdg_data = sandbox_root.join("xdg/data");
    let tmp = sandbox_root.join("tmp");
    let pycache = sandbox_root.join("pycache");

    let mut command = Command::new(engine_exe);
    command
        .current_dir(engine_cwd)
        .args(args_prefix)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    for key in SAFE_INHERITED_ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("TMPDIR", &tmp)
        .env("TMP", &tmp)
        .env("TEMP", &tmp)
        .env("PYTHONPYCACHEPREFIX", &pycache);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot spawn python oracle: {e}"))?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "python oracle stdout pipe unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "python oracle stderr pipe unavailable".to_string())?;
    let stdout_rx = spawn_reader(stdout);
    let stderr_rx = spawn_reader(stderr);
    let mut sampler = start_process_sampler(pid, metric_capability);

    let deadline = started + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    terminate_process_tree(pid, false);
                    thread::sleep(Duration::from_millis(200));
                    terminate_process_tree(pid, true);
                    let wait_result = child.wait();
                    if let Some(sampler) = sampler.take() {
                        let _ = sampler.finish();
                    }
                    break wait_result
                        .map_err(|e| format!("cannot wait for terminated python oracle: {e}"))?;
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => {
                terminate_process_tree(pid, true);
                let _ = child.wait();
                if let Some(sampler) = sampler.take() {
                    let _ = sampler.finish();
                }
                let _ = drain_output(&stdout_rx, pid, Duration::from_secs(2));
                let _ = drain_output(&stderr_rx, pid, Duration::from_secs(2));
                return Err(format!("cannot poll python oracle: {error}"));
            }
        }
    };

    let stdout_result = drain_output(&stdout_rx, pid, Duration::from_secs(5));
    let stderr_result = drain_output(&stderr_rx, pid, Duration::from_secs(5));
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let metrics = match sampler {
        Some(sampler) => sampler.finish(),
        None => ProcessMetrics::unsupported(),
    };
    let stdout = stdout_result?;
    let stderr = stderr_result?;

    Ok(RunOutcome {
        wall_ms,
        exit_code: status.code(),
        timed_out,
        stdout,
        stderr,
        metrics,
    })
}

fn spawn_reader<R>(mut reader: R) -> mpsc::Receiver<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let result = reader
            .read_to_end(&mut buffer)
            .map(|_| buffer)
            .map_err(|e| format!("cannot read python oracle output: {e}"));
        let _ = sender.send(result);
    });
    receiver
}

fn drain_output(
    receiver: &mpsc::Receiver<Result<Vec<u8>, String>>,
    pid: u32,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            terminate_process_tree(pid, false);
            thread::sleep(Duration::from_millis(150));
            terminate_process_tree(pid, true);
            match receiver.recv_timeout(Duration::from_secs(2)) {
                Ok(result) => result,
                Err(_) => Err(
                    "timed out draining oracle output after terminating the process tree"
                        .to_string(),
                ),
            }
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("oracle output reader disconnected before returning a result".to_string())
        }
    }
}

fn terminate_process_tree(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        let _ = Command::new("kill")
            .arg(signal)
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command.arg("/PID").arg(pid.to_string()).arg("/T");
        if force {
            command.arg("/F");
        }
        let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, force);
    }
}

#[cfg(target_os = "linux")]
const HOST_SAMPLER: &str = "linux-proc";
#[cfg(target_os = "linux")]
const CPU_METRIC_METHOD: &str = "sampled /proc process tree utime+stime every 5ms; max observed; ticks converted with getconf CLK_TCK";
#[cfg(target_os = "linux")]
const RSS_METRIC_METHOD: &str = "sampled /proc process tree resident pages every 5ms; maximum summed descendant RSS; pages converted with getconf PAGESIZE";
#[cfg(target_os = "linux")]
const IO_METRIC_METHOD: &str =
    "sampled /proc process tree read_bytes/write_bytes every 5ms; max observed";
#[cfg(target_os = "macos")]
const HOST_SAMPLER: &str = "macos-ps";
#[cfg(target_os = "macos")]
const CPU_METRIC_METHOD: &str = "sampled /bin/ps -ax -o pid=,ppid=,pgid=,rss=,time= every 5ms; CPU is the sum of each observed process's peak cumulative TIME, not exact per-sample elapsed CPU; ps TIME has 10ms display resolution and processes that exit between samples contribute only their last observed TIME";
#[cfg(target_os = "macos")]
const RSS_METRIC_METHOD: &str = "sampled /bin/ps -ax -o pid=,ppid=,pgid=,rss=,time= every 5ms; maximum summed descendant rss, KiB converted to bytes (1024-byte units)";
#[cfg(target_os = "macos")]
const IO_METRIC_METHOD: &str =
    "macOS ps exposes no aggregate process-tree byte counters; no unsafe libproc FFI is used";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const HOST_SAMPLER: &str = "unsupported";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const CPU_METRIC_METHOD: &str = "no tested process-tree CPU sampler on this host";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const RSS_METRIC_METHOD: &str = "no tested process-tree RSS sampler on this host";
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const IO_METRIC_METHOD: &str = "no tested process-tree IO sampler on this host";

fn metric_value(supported: bool, unit: &str, method: &str, reason: Option<&str>) -> Value {
    if supported {
        json!({
            "supported": true,
            "unit": unit,
            "method": method,
            "sampler": HOST_SAMPLER,
        })
    } else {
        json!({
            "supported": false,
            "unit": Value::Null,
            "method": Value::Null,
            "reason": reason.unwrap_or("no supported host measurement API"),
        })
    }
}

/// Whether the pre-registered policy makes a metric mandatory. Wall time is
/// not read here: it is always required and always measurable via
/// `std::time::Instant`, and `validate_policy` fixes its `blockOnWorse` to true.
fn budget_blocks_on_worse(policy: &PerformancePolicy, metric: &str) -> bool {
    policy
        .budgets
        .get(metric)
        .and_then(|budget| budget.block_on_worse)
        .unwrap_or(false)
}

fn budget_requires_explanation(policy: &PerformancePolicy, metric: &str) -> bool {
    policy
        .budgets
        .get(metric)
        .and_then(|budget| budget.requires_explanation)
        .unwrap_or(false)
}

struct MetricEvaluation {
    limitations: Vec<MetricLimitation>,
    blockers: Vec<String>,
}

/// Derive acceptance consequences from host capability plus the loaded policy.
/// An unsupported non-blocking metric stays a limitation (samples keep null and
/// a reason); an unsupported blocking metric is always a hard acceptance
/// failure, including `io` if a future policy registers it as blocking.
fn evaluate_metric_policy(
    policy: &PerformancePolicy,
    capability: &MetricCapability,
) -> MetricEvaluation {
    let checks = [
        ("cpu", "CPU-time", "milliseconds", capability.cpu_supported),
        (
            "rss",
            "peak descendant RSS",
            "bytes",
            capability.rss_supported,
        ),
        ("io", "process IO", "bytes", capability.io_supported),
    ];
    let reason = capability
        .reason
        .clone()
        .unwrap_or_else(|| "no supported host measurement API".to_string());
    let mut limitations = Vec::new();
    let mut blockers = Vec::new();
    for (metric, label, unit, supported) in checks {
        if supported {
            continue;
        }
        let blocking = budget_blocks_on_worse(policy, metric);
        if blocking {
            blockers.push(format!(
                "mandatory {label} metric is not collected on this host ({reason})"
            ));
        }
        limitations.push(MetricLimitation {
            metric: metric.to_string(),
            unit: unit.to_string(),
            blocking,
            requires_explanation: budget_requires_explanation(policy, metric),
            reason: reason.clone(),
        });
    }
    MetricEvaluation {
        limitations,
        blockers,
    }
}

fn detect_metric_capability() -> MetricCapability {
    #[cfg(target_os = "linux")]
    {
        let clock_ticks = command_output(Path::new("getconf"), &["CLK_TCK"])
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| *value > 0.0);
        let page_size = command_output(Path::new("getconf"), &["PAGESIZE"])
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0);
        match (clock_ticks, page_size) {
            (Some(clock_ticks_per_second), Some(page_size_bytes)) => MetricCapability {
                cpu_supported: true,
                rss_supported: true,
                io_supported: true,
                units: Some(MetricUnits {
                    clock_ticks_per_second,
                    page_size_bytes,
                }),
                reason: None,
            },
            _ => MetricCapability {
                cpu_supported: false,
                rss_supported: false,
                io_supported: false,
                units: None,
                reason: Some(
                    "getconf CLK_TCK/PAGESIZE did not provide tested /proc unit conversions"
                        .to_string(),
                ),
            },
        }
    }
    #[cfg(target_os = "macos")]
    {
        detect_macos_metric_capability()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        MetricCapability {
            cpu_supported: false,
            rss_supported: false,
            io_supported: false,
            units: None,
            reason: Some(
                "host does not expose a tested process-tree CPU/RSS/IO sampler; values stay null and blocking metrics fail acceptance rather than being guessed"
                    .to_string(),
            ),
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_metric_capability() -> MetricCapability {
    let ps = Path::new(MACOS_PS_PATH);
    let own_pid = std::process::id();
    let pid_arg = own_pid.to_string();
    let probe_ok = ps.is_file()
        && Command::new(ps)
            .args(["-o", "pid=,ppid=,pgid=,rss=,time=", "-p", pid_arg.as_str()])
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|text| {
                let mut rows = text
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(parse_macos_ps_row);
                matches!(
                    (rows.next(), rows.next()),
                    (Some(Some(row)), None) if row.pid == own_pid && row.rss_kib > 0
                )
            })
            .unwrap_or(false);
    if probe_ok {
        MetricCapability {
            cpu_supported: true,
            rss_supported: true,
            io_supported: false,
            units: None,
            reason: Some(
                "macOS /bin/ps exposes process-tree CPU time and resident memory but no tested aggregate read/write byte counter; process IO stays null"
                    .to_string(),
            ),
        }
    } else {
        MetricCapability {
            cpu_supported: false,
            rss_supported: false,
            io_supported: false,
            units: None,
            reason: Some(format!(
                "{} is unavailable or its -o pid/ppid/pgid/rss/time output did not match the tested parser",
                MACOS_PS_PATH
            )),
        }
    }
}

fn start_process_sampler(pid: u32, capability: &MetricCapability) -> Option<ProcessSampler> {
    #[cfg(target_os = "linux")]
    {
        let units = capability.units.clone()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handle = thread::spawn(move || sample_process_tree(pid, &units, &stop_clone));
        Some(ProcessSampler { stop, handle })
    }
    #[cfg(target_os = "macos")]
    {
        if !capability.cpu_supported || !capability.rss_supported {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handle = thread::spawn(move || sample_process_tree_macos(pid, &stop_clone));
        Some(ProcessSampler { stop, handle })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, capability);
        None
    }
}

#[cfg(target_os = "linux")]
struct ProcessSnapshot {
    cpu_ms: f64,
    rss_bytes: u64,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

#[cfg(target_os = "linux")]
struct ProcStat {
    ppid: u32,
    rss_pages: u64,
    ticks: u64,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

#[cfg(target_os = "linux")]
fn sample_process_tree(root_pid: u32, units: &MetricUnits, stop: &AtomicBool) -> ProcessMetrics {
    let mut peak_rss_bytes = 0u64;
    let mut cpu_ms: Option<f64> = None;
    let mut read_bytes: Option<u64> = None;
    let mut write_bytes: Option<u64> = None;
    let mut sampled = false;
    loop {
        if let Some(snapshot) = sample_process_snapshot(root_pid, units) {
            sampled = true;
            peak_rss_bytes = peak_rss_bytes.max(snapshot.rss_bytes);
            cpu_ms = Some(cpu_ms.unwrap_or(0.0).max(snapshot.cpu_ms));
            if let Some(bytes) = snapshot.read_bytes {
                read_bytes = Some(read_bytes.unwrap_or(0).max(bytes));
            }
            if let Some(bytes) = snapshot.write_bytes {
                write_bytes = Some(write_bytes.unwrap_or(0).max(bytes));
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !sampled {
        return ProcessMetrics::unsupported();
    }
    ProcessMetrics {
        cpu_ms,
        peak_child_rss_bytes: Some(peak_rss_bytes),
        read_bytes,
        write_bytes,
    }
}

#[cfg(target_os = "linux")]
fn sample_process_snapshot(root_pid: u32, units: &MetricUnits) -> Option<ProcessSnapshot> {
    let mut processes = BTreeMap::new();
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if let Some(stat) = read_proc_stat(pid) {
            processes.insert(pid, stat);
        }
    }
    if !processes.contains_key(&root_pid) {
        return None;
    }

    let mut selected: BTreeSet<u32> = BTreeSet::new();
    selected.insert(root_pid);
    loop {
        let mut added = false;
        for (pid, stat) in &processes {
            if !selected.contains(pid) && selected.contains(&stat.ppid) {
                selected.insert(*pid);
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    let mut ticks = 0u64;
    let mut rss_pages = 0u64;
    let mut read_total = Some(0u64);
    let mut write_total = Some(0u64);
    for pid in selected {
        let Some(stat) = processes.get(&pid) else {
            continue;
        };
        ticks = ticks.saturating_add(stat.ticks);
        rss_pages = rss_pages.saturating_add(stat.rss_pages);
        match (read_total, stat.read_bytes) {
            (Some(total), Some(value)) => read_total = Some(total.saturating_add(value)),
            _ => read_total = None,
        }
        match (write_total, stat.write_bytes) {
            (Some(total), Some(value)) => write_total = Some(total.saturating_add(value)),
            _ => write_total = None,
        }
    }

    Some(ProcessSnapshot {
        cpu_ms: ticks as f64 / units.clock_ticks_per_second * 1000.0,
        rss_bytes: rss_pages.saturating_mul(units.page_size_bytes),
        read_bytes: read_total,
        write_bytes: write_total,
    })
}

#[cfg(target_os = "linux")]
fn read_proc_stat(pid: u32) -> Option<ProcStat> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    if fields.len() < 13 {
        return None;
    }
    let ppid = fields[1].parse::<u32>().ok()?;
    let utime = fields[11].parse::<u64>().ok()?;
    let stime = fields[12].parse::<u64>().ok()?;
    let statm = fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let rss_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    let (read_bytes, write_bytes) = read_proc_io(pid);
    Some(ProcStat {
        ppid,
        rss_pages,
        ticks: utime.saturating_add(stime),
        read_bytes,
        write_bytes,
    })
}

#[cfg(target_os = "linux")]
fn read_proc_io(pid: u32) -> (Option<u64>, Option<u64>) {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/io")) else {
        return (None, None);
    };
    let mut read_bytes = None;
    let mut write_bytes = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("read_bytes:") {
            read_bytes = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.strip_prefix("write_bytes:") {
            write_bytes = value.trim().parse::<u64>().ok();
        }
    }
    (read_bytes, write_bytes)
}

#[cfg(target_os = "macos")]
const MACOS_PS_PATH: &str = "/bin/ps";
/// macOS `ps -o rss` reports resident set size in 1024-byte units.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MACOS_PS_RSS_UNIT_BYTES: u64 = 1024;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct MacPsRow {
    pid: u32,
    ppid: u32,
    pgid: u32,
    rss_kib: u64,
    cpu_seconds: f64,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct MacTreeSnapshot {
    cpu_by_pid: BTreeMap<u32, f64>,
    rss_bytes: u64,
}

/// Parses macOS `ps` `TIME` into seconds.
///
/// Two clock shapes occur in live output: `mm:ss.cc`, where macOS accumulates
/// minutes without an hours field (observed `0:00.17`, `71:24.25`,
/// `190:44.36`, `238:02.23`), and `[[dd-]hh:]mm:ss`, where a real minutes
/// field must stay below 60 (tested `1-02:03:04`). A centisecond fraction is
/// the display resolution, so a single process contributes CPU time in 10ms
/// steps.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_macos_ps_cpu_time(raw: &str) -> Option<f64> {
    if raw.is_empty() {
        return None;
    }
    let (days, clock) = match raw.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, raw),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds, has_minutes_field) = match parts.as_slice() {
        // macOS prints long-running processes with accumulating minutes, e.g.
        // observed live values `190:44.36`, `71:24.25`, and `238:02.23`.
        [minutes, seconds] => (
            0u64,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<f64>().ok()?,
            false,
        ),
        [hours, minutes, seconds] => (
            hours.parse::<u64>().ok()?,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<f64>().ok()?,
            true,
        ),
        _ => return None,
    };
    if !(0.0..60.0).contains(&seconds) {
        return None;
    }
    if has_minutes_field && minutes >= 60 {
        return None;
    }
    let total = days as f64 * 86_400.0 + hours as f64 * 3_600.0 + minutes as f64 * 60.0 + seconds;
    total.is_finite().then_some(total)
}

/// Parses one `pid=,ppid=,pgid=,rss=,time=` row. Extra or malformed fields
/// are rejected so a changed `ps` format fails closed instead of guessing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_macos_ps_row(line: &str) -> Option<MacPsRow> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse::<u32>().ok()?;
    let ppid = fields.next()?.parse::<u32>().ok()?;
    let pgid = fields.next()?.parse::<u32>().ok()?;
    let rss_kib = fields.next()?.parse::<u64>().ok()?;
    let cpu_seconds = parse_macos_ps_cpu_time(fields.next()?)?;
    if fields.next().is_some() {
        return None;
    }
    Some(MacPsRow {
        pid,
        ppid,
        pgid,
        rss_kib,
        cpu_seconds,
    })
}

/// Selects the oracle process tree from one `ps` snapshot.
///
/// Processes in the oracle's process group are included even when they were
/// reparented, and a parent-link walk also catches descendants that left the
/// group while their parent is still visible. Returns `None` when neither the
/// root nor any group member is present.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_tree_snapshot(rows: &[MacPsRow], root_pid: u32) -> Option<MacTreeSnapshot> {
    let mut by_pid = BTreeMap::new();
    for row in rows {
        if by_pid.insert(row.pid, *row).is_some() {
            return None;
        }
    }
    let mut selected: BTreeSet<u32> = by_pid
        .values()
        .filter(|row| row.pid == root_pid || row.pgid == root_pid)
        .map(|row| row.pid)
        .collect();
    if selected.is_empty() {
        return None;
    }
    loop {
        let mut added = false;
        for (pid, row) in &by_pid {
            if !selected.contains(pid) && selected.contains(&row.ppid) {
                selected.insert(*pid);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut cpu_by_pid = BTreeMap::new();
    let mut rss_kib = 0u64;
    for pid in selected {
        let row = by_pid.get(&pid)?;
        cpu_by_pid.insert(pid, row.cpu_seconds);
        rss_kib = rss_kib.saturating_add(row.rss_kib);
    }
    Some(MacTreeSnapshot {
        cpu_by_pid,
        rss_bytes: rss_kib.saturating_mul(MACOS_PS_RSS_UNIT_BYTES),
    })
}

/// Runs `ps` once. Command failure or an unexpected row is an error so the
/// caller can fail closed; an absent process tree is `Ok(None)`.
#[cfg(target_os = "macos")]
fn macos_ps_snapshot(root_pid: u32) -> Result<Option<MacTreeSnapshot>, String> {
    let output = Command::new(MACOS_PS_PATH)
        .args(["-ax", "-o", "pid=,ppid=,pgid=,rss=,time="])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", MACOS_PS_PATH))?;
    if !output.status.success() {
        return Err(format!(
            "{} exited with {}",
            MACOS_PS_PATH,
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "a signal".to_string())
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| format!("{} emitted non-UTF-8 output", MACOS_PS_PATH))?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row = parse_macos_ps_row(line)
            .ok_or_else(|| format!("unexpected {} row {}: '{line}'", MACOS_PS_PATH, index + 1))?;
        rows.push(row);
    }
    Ok(macos_tree_snapshot(&rows, root_pid))
}

#[cfg(target_os = "macos")]
fn sample_process_tree_macos(root_pid: u32, stop: &AtomicBool) -> ProcessMetrics {
    let mut peak_rss_bytes = 0u64;
    let mut peak_cpu_by_pid: BTreeMap<u32, f64> = BTreeMap::new();
    let mut sampled = false;
    loop {
        match macos_ps_snapshot(root_pid) {
            Ok(Some(snapshot)) => {
                sampled = true;
                peak_rss_bytes = peak_rss_bytes.max(snapshot.rss_bytes);
                for (pid, seconds) in snapshot.cpu_by_pid {
                    let peak = peak_cpu_by_pid.entry(pid).or_insert(0.0);
                    *peak = peak.max(seconds);
                }
            }
            Ok(None) => {}
            // A changed/unreadable ps format invalidates the whole sample.
            Err(_) => return ProcessMetrics::unsupported(),
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !sampled {
        return ProcessMetrics::unsupported();
    }
    let cpu_seconds: f64 = peak_cpu_by_pid.values().sum();
    ProcessMetrics {
        cpu_ms: Some(cpu_seconds * 1000.0),
        peak_child_rss_bytes: Some(peak_rss_bytes),
        read_bytes: None,
        write_bytes: None,
    }
}

fn resolve_placeholder(value: &str, workspace: &Path, tmp: &Path, repo: &Path) -> String {
    value
        .replace("{workspace}", &workspace.display().to_string())
        .replace("{tmp}", &tmp.display().to_string())
        .replace("{repo}", &repo.display().to_string())
}

fn normalizer_pairs(
    repo_root: &Path,
    engine_cwd: &Path,
    temp_root: &Path,
    workspace: &Path,
) -> Vec<(String, &'static str)> {
    let mut pairs = Vec::new();
    for (path, label) in [
        (temp_root.to_path_buf(), "<tmp>"),
        (workspace.to_path_buf(), "<workspace>"),
        (repo_root.to_path_buf(), "<repo>"),
        (engine_cwd.to_path_buf(), "<oracle-cwd>"),
    ] {
        pairs.push((path.display().to_string(), label));
        if let Ok(canonical) = path.canonicalize() {
            pairs.push((canonical.display().to_string(), label));
        }
    }
    pairs.sort_by_key(|pair| Reverse(pair.0.len()));
    pairs.dedup();
    pairs
}

fn normalise(text: &str, pairs: &[(String, &'static str)]) -> String {
    let mut result = text.to_string();
    for (path, label) in pairs {
        if !path.is_empty() {
            result = result.replace(path, label);
        }
    }
    result
}

/// Character bound for one diagnostic stream. Redaction happens before this
/// bound is applied so a full private prefix is replaced even when the excerpt
/// is later truncated.
const DIAGNOSTIC_EXCERPT_CHARS: usize = 8192;

fn bounded_excerpt(text: &str) -> (String, bool) {
    let excerpt: String = text.chars().take(DIAGNOSTIC_EXCERPT_CHARS).collect();
    let truncated = excerpt.len() < text.len();
    (excerpt, truncated)
}

/// Build the failed-row excerpt pair from the already normalized output. The
/// additional private roots (invoking home and the pinned cache source) are
/// redacted here, before either stream is bounded.
fn output_diagnostics(
    stdout: &str,
    stderr: &str,
    repo_root: &Path,
    engine_cwd: &Path,
    temp_root: &Path,
    workspace: &Path,
    tool_cache_root: Option<&Path>,
) -> OutputDiagnostics {
    let mut pairs = normalizer_pairs(repo_root, engine_cwd, temp_root, workspace);
    if let Some(home) = invoking_home() {
        if let Some(text) = home.to_str() {
            if !text.is_empty() && text != "/" {
                pairs.push((text.to_string(), "~"));
                if let Ok(canonical) = home.canonicalize() {
                    pairs.push((canonical.display().to_string(), "~"));
                }
            }
        }
    }
    if let Some(cache_root) = tool_cache_root {
        pairs.push((cache_root.display().to_string(), "<tool-cache>"));
        if let Ok(canonical) = cache_root.canonicalize() {
            pairs.push((canonical.display().to_string(), "<tool-cache>"));
        }
    }
    pairs.sort_by_key(|pair| Reverse(pair.0.len()));
    pairs.dedup();

    let stdout_text = normalise(stdout, &pairs);
    let stderr_text = normalise(stderr, &pairs);
    let (stdout_excerpt, stdout_truncated) = bounded_excerpt(&stdout_text);
    let (stderr_excerpt, stderr_truncated) = bounded_excerpt(&stderr_text);
    OutputDiagnostics {
        stdout_excerpt,
        stderr_excerpt,
        stdout_truncated,
        stderr_truncated,
    }
}

#[derive(Deserialize)]
struct MorpheReportRaw<'a> {
    #[serde(borrow, rename = "testCases")]
    test_cases: &'a RawValue,
}

#[derive(Deserialize)]
struct MorpheCaseRaw<'a> {
    #[serde(borrow, rename = "durationSeconds")]
    duration_seconds: &'a RawValue,
}

fn raw_slice_offset(base: &str, slice: &str) -> Option<usize> {
    let base_start = base.as_ptr() as usize;
    let slice_start = slice.as_ptr() as usize;
    let base_end = base_start.checked_add(base.len())?;
    let slice_end = slice_start.checked_add(slice.len())?;
    if slice_start >= base_start && slice_end <= base_end {
        Some(slice_start - base_start)
    } else {
        None
    }
}

/// Replace only the numeric value spans at
/// `$.testCases[*].durationSeconds` with `null`.
///
/// The rest of the report is preserved byte-for-byte: ordering, whitespace,
/// Unicode escapes, nulls, counts, statuses, options, and error text are never
/// reserialized or canonicalized. `options.durationSeconds` and every other
/// same-named key stay substantive. A missing or non-numeric duration field, or
/// a stdout document without the expected report structure, is rejected rather
/// than treated as an absent value.
fn normalize_morphe_durations(text: &str) -> Result<(String, usize), String> {
    let report: MorpheReportRaw = serde_json::from_str(text).map_err(|error| {
        format!("stdout is not the expected PatchCompatibilityReport JSON: {error}")
    })?;
    let cases: Vec<&RawValue> = serde_json::from_str(report.test_cases.get()).map_err(|error| {
        format!("stdout testCases must be a JSON array of test-case objects: {error}")
    })?;

    let mut spans = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let parsed: MorpheCaseRaw = serde_json::from_str(case.get()).map_err(|error| {
            format!("stdout testCases[{index}] must be an object with durationSeconds: {error}")
        })?;
        let value = parsed.duration_seconds.get();
        let _: serde_json::Number = serde_json::from_str(value).map_err(|error| {
            format!(
                "stdout testCases[{index}].durationSeconds must be a JSON number, got {value}: {error}"
            )
        })?;
        let start = raw_slice_offset(text, value).ok_or_else(|| {
            format!(
                "stdout testCases[{index}].durationSeconds is not a borrowed span of the report"
            )
        })?;
        spans.push((start, start + value.len()));
    }

    spans.sort_by_key(|&(start, _)| Reverse(start));
    let mut normalized = text.to_string();
    for &(start, end) in &spans {
        normalized.replace_range(start..end, "null");
    }
    Ok((normalized, spans.len()))
}

fn wall_summary(samples: &[SampleResult]) -> Option<WallSummary> {
    let mut values: Vec<f64> = samples
        .iter()
        .filter(|sample| sample.kind != "warmup")
        .map(|sample| sample.wall_ms)
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let count = values.len();
    let sum: f64 = values.iter().sum();
    let median = if count % 2 == 1 {
        values[count / 2]
    } else {
        (values[count / 2 - 1] + values[count / 2]) / 2.0
    };
    let p95_index = ((count as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(count - 1);
    Some(WallSummary {
        count,
        min_ms: values[0],
        median_ms: median,
        mean_ms: sum / count as f64,
        p95_ms: values[p95_index],
        max_ms: values[count - 1],
    })
}

// ---------------------------------------------------------------------------
// Inputs and environment
// ---------------------------------------------------------------------------

fn verify_input(repo_root: &Path, input: &InputRef) -> InputStatus {
    let path = resolve_against(repo_root, Path::new(&input.path));
    let display = display_path(&path, repo_root);
    match verify_file_hash(&path, &input.sha256) {
        Ok(file) => InputStatus {
            role: input.role.clone(),
            declared_sha256: input.sha256.clone(),
            actual_sha256: Some(file.sha256),
            size_bytes: Some(file.size_bytes),
            available: true,
            display_path: display,
            reason: None,
        },
        Err(error) => InputStatus {
            role: input.role.clone(),
            declared_sha256: input.sha256.clone(),
            actual_sha256: None,
            size_bytes: None,
            available: false,
            display_path: display,
            reason: Some(format!("input role '{}': {error}", input.role)),
        },
    }
}

#[derive(Debug)]
struct FileHash {
    size_bytes: u64,
    sha256: String,
}

fn hash_file(path: &Path) -> Result<FileHash, String> {
    if !path.is_file() {
        return Err("file does not exist".to_string());
    }
    let mut file = fs::File::open(path).map_err(|e| format!("cannot open file: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size_bytes = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot read file: {e}"))?;
        if read == 0 {
            break;
        }
        size_bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok(FileHash {
        size_bytes,
        sha256: hex(&hasher.finalize()),
    })
}

fn verify_file_hash(path: &Path, expected_hex: &str) -> Result<FileHash, String> {
    let hash = hash_file(path)?;
    if !hash.sha256.eq_ignore_ascii_case(expected_hex) {
        return Err(format!(
            "sha256 mismatch: expected {expected_hex}, got {}",
            hash.sha256
        ));
    }
    Ok(hash)
}

fn effective_base_env(
    manifest_env: &BTreeMap<String, String>,
    seed: u64,
) -> BTreeMap<String, String> {
    let mut env = manifest_env.clone();
    env.entry("PYTHONDONTWRITEBYTECODE".to_string())
        .or_insert_with(|| "1".to_string());
    env.entry("PYTHONHASHSEED".to_string())
        .or_insert_with(|| seed.to_string());
    env
}

fn collect_environment(
    repo_root: &Path,
    engine_exe: &Path,
    engine_cwd: &Path,
    python_version: Option<String>,
    git_version: Option<String>,
) -> Value {
    let logical_cpus = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let hostname = command_output(Path::new("hostname"), &[])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let storage = storage_info(repo_root);
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "osVersion": os_version(),
        "hostname": hostname,
        "cpuModel": cpu_model(),
        "logicalCpus": logical_cpus,
        "ramBytes": total_ram_bytes(),
        "storage": storage,
        "pythonVersion": python_version,
        "gitVersion": git_version,
        "javaVersion": command_version(Path::new("java"), "-version"),
        "toolsLock": configured_tool_versions(repo_root),
        "oracleExecutable": display_path(engine_exe, repo_root),
        "oracleCwd": display_path(engine_cwd, repo_root),
        "measurementTools": measurement_tools(),
        "workerCount": 1,
        "timezone": std::env::var("TZ").unwrap_or_else(|_| "process-default".to_string()),
        "locale": {
            "LANG": std::env::var("LANG").ok(),
            "LC_ALL": std::env::var("LC_ALL").ok(),
            "LC_CTYPE": std::env::var("LC_CTYPE").ok(),
            "TZ": std::env::var("TZ").ok(),
        },
        "inheritedPathSha256": std::env::var("PATH").ok().map(|value| sha256_bytes(value.as_bytes())),
    })
}

fn measurement_tools() -> Vec<&'static str> {
    #[cfg(target_os = "linux")]
    {
        vec![
            "std::time::Instant",
            "sha2",
            "df -Pk (1024-byte blocks)",
            "/proc/<pid>/{stat,statm,io}",
            "getconf CLK_TCK/PAGESIZE",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            "std::time::Instant",
            "sha2",
            "df -Pk (1024-byte blocks)",
            "/bin/ps -ax -o pid=,ppid=,pgid=,rss=,time=",
        ]
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        vec!["std::time::Instant", "sha2", "df -Pk (1024-byte blocks)"]
    }
}

fn configured_tool_versions(repo_root: &Path) -> Value {
    let path = repo_root.join("tools/apk-lab/tools.lock.json");
    let Ok(bytes) = fs::read(&path) else {
        return json!({
            "supported": false,
            "reason": "tools/apk-lab/tools.lock.json is unavailable",
        });
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return json!({
            "supported": false,
            "reason": "tools/apk-lab/tools.lock.json is not valid JSON",
        });
    };
    let versions = value
        .get("tools")
        .and_then(Value::as_object)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|(name, spec)| {
                    spec.get("version")
                        .and_then(Value::as_str)
                        .map(|version| (name.clone(), Value::String(version.to_string())))
                })
                .collect::<serde_json::Map<String, Value>>()
        })
        .unwrap_or_default();
    json!({
        "supported": true,
        "path": "tools/apk-lab/tools.lock.json",
        "sha256": sha256_bytes(&bytes),
        "versions": versions,
    })
}

fn command_version(executable: &Path, flag: &str) -> Option<String> {
    command_output(executable, &[flag]).map(|value| value.trim().to_string())
}

fn command_output(executable: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(executable).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            None
        } else {
            Some(err)
        }
    } else {
        Some(text)
    }
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "macos") {
        command_output(Path::new("sw_vers"), &["-productVersion"])
    } else if cfg!(target_os = "linux") {
        fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|value| value.trim().to_string())
    } else {
        None
    }
}

fn cpu_model() -> Option<String> {
    if cfg!(target_os = "macos") {
        return command_output(Path::new("sysctl"), &["-n", "machdep.cpu.brand_string"]);
    }
    if cfg!(target_os = "linux") {
        let text = fs::read_to_string("/proc/cpuinfo").ok()?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("model name") {
                return value
                    .split_once(':')
                    .map(|(_, model)| model.trim().to_string());
            }
        }
    }
    None
}

fn total_ram_bytes() -> Option<u64> {
    if cfg!(target_os = "macos") {
        let text = command_output(Path::new("sysctl"), &["-n", "hw.memsize"])?;
        return text.trim().parse::<u64>().ok();
    }
    if cfg!(target_os = "linux") {
        let text = fs::read_to_string("/proc/meminfo").ok()?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kilobytes = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())?;
                return kilobytes.checked_mul(1024);
            }
        }
    }
    None
}

fn storage_info(repo_root: &Path) -> Value {
    let output = Command::new("df").arg("-Pk").arg(repo_root).output();
    let Ok(output) = output else {
        return json!({
            "supported": false,
            "reason": "df -Pk could not be executed",
        });
    };
    if !output.status.success() {
        return json!({
            "supported": false,
            "reason": "df -Pk exited unsuccessfully",
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let data_line = text.lines().skip(1).find(|line| !line.trim().is_empty());
    let Some(line) = data_line else {
        return json!({
            "supported": false,
            "reason": "df -Pk produced no data row",
        });
    };
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 6 {
        return json!({
            "supported": false,
            "reason": "df -Pk output did not match the POSIX -P layout",
        });
    }
    let blocks = fields[1].parse::<u64>().ok();
    let available = fields[3].parse::<u64>().ok();
    match (blocks, available) {
        (Some(blocks), Some(available)) => json!({
            "supported": true,
            "method": "df -Pk (1024-byte blocks)",
            "filesystem": fields[0],
            "mountPoint": fields[5..].join(" "),
            "totalBytes": blocks.saturating_mul(1024),
            "freeBytes": available.saturating_mul(1024),
            "capacity": fields[4],
        }),
        _ => json!({
            "supported": false,
            "reason": "df -Pk numeric fields could not be parsed",
        }),
    }
}

// ---------------------------------------------------------------------------
// Provenance and private source archive
// ---------------------------------------------------------------------------

struct SourceSnapshot {
    revision: String,
    dirty: bool,
    digest: String,
}

/// Python oracle sources archived for exact baseline reconstruction.
const ORACLE_SOURCE_PATHS: &[&str] = &[
    "tools/apk-lab/apk_lab",
    "tools/apk-lab/tests",
    "tools/apk-lab/pyproject.toml",
    "tools/apk-lab/uv.lock",
    "tools/apk-lab/tools.lock.json",
    "tools/apk-lab/.python-version",
    "rust-toolchain.toml",
];

/// Legacy Python generators plus the active invocation/config/lock files that
/// determine their behavior. The root `patches-list.json` release metadata is
/// included because the Morphe oracle reports its bundle version from it.
/// Plans 006/007 delete the generators before plan
/// 008 reconstructs the baseline for asset/README comparison, so they must be
/// archived with the oracle rather than inferred from a later revision.
/// Credential-bearing local files (local.properties, `.env`, caches) are never
/// listed here.
const ANCILLARY_SOURCE_PATHS: &[&str] = &[
    "patches-list.json",
    ".github/scripts",
    ".github/workflows/release.yml",
    ".github/workflows/sync-app-icons.yml",
    ".github/workflows/apk-lab-tests.yml",
    ".releaserc",
    ".pre-commit-config.yaml",
    "scripts/audit.ts",
    "patches/build.gradle.kts",
    "gradle/libs.versions.toml",
    "package.json",
    "bun.lock",
    "package-lock.json",
    "site/package.json",
    "site/bun.lock",
];

/// Source files that plan 001 archived for plan-008 reconstruction but that
/// this harness never invokes and that their owners may already have deleted.
/// They are collected only when still present; an absent path is expected and
/// is reported explicitly in the archive manifest. The frozen plan-001 archive
/// remains authoritative for reconstructing them.
const HISTORICAL_SOURCE_PATHS: &[&str] = &[
    ".github/scripts/generate_patches_readme.py",
    "extensions/adobescan-pdf-native/tests/fixtures/generate_fixtures.py",
];

/// True for a path in either the active ancillary set or the historical
/// provenance set. Historical entries do not imply an active consumer.
fn is_ancillary_source_path(relative: &str) -> bool {
    ANCILLARY_SOURCE_PATHS
        .iter()
        .chain(HISTORICAL_SOURCE_PATHS.iter())
        .any(|candidate| {
            relative == *candidate
                || relative
                    .strip_prefix(candidate)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// Changed/untracked detection covers the whole oracle tree plus the ancillary
/// generators and their invocation inputs. A missing historical path is normal
/// after its owner removes it; git reports nothing for an absent pathspec.
fn source_git_paths() -> Vec<&'static str> {
    let mut paths = vec!["tools/apk-lab"];
    paths.extend_from_slice(ANCILLARY_SOURCE_PATHS);
    paths.extend_from_slice(HISTORICAL_SOURCE_PATHS);
    paths
}

fn collect_source(repo_root: &Path) -> (SourceSnapshot, Vec<String>) {
    let mut reasons = Vec::new();
    let revision = git_output(repo_root, &["rev-parse", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            reasons.push("git revision could not be determined".to_string());
            "unknown".to_string()
        });
    let mut status_args = vec!["status", "--porcelain", "--"];
    status_args.extend(source_git_paths());
    let status = git_output(repo_root, &status_args).unwrap_or_default();
    let dirty = !status.trim().is_empty();
    let dirty_bytes = if dirty {
        let mut diff_args = vec!["diff", "--binary", "HEAD", "--"];
        diff_args.extend(source_git_paths());
        git_bytes(repo_root, &diff_args).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut dirty_hasher = Sha256::new();
    dirty_hasher.update(&dirty_bytes);
    if dirty {
        let mut untracked_args = vec!["ls-files", "--others", "--exclude-standard", "--"];
        untracked_args.extend(source_git_paths());
        let untracked = git_output(repo_root, &untracked_args).unwrap_or_default();
        let mut paths: Vec<&str> = untracked.lines().filter(|line| !line.is_empty()).collect();
        paths.sort_unstable();
        for relative in paths {
            dirty_hasher.update(relative.as_bytes());
            dirty_hasher.update(b"\0");
            if let Ok(bytes) = fs::read(repo_root.join(relative)) {
                dirty_hasher.update(sha256_bytes(&bytes).as_bytes());
            }
            dirty_hasher.update(b"\n");
        }
    }
    let dirty_digest = hex(&dirty_hasher.finalize());

    let mut hasher = Sha256::new();
    hasher.update(revision.as_bytes());
    hasher.update(b"\0");
    hasher.update(if dirty { b"dirty" } else { b"clean" });
    hasher.update(b"\0");
    hasher.update(dirty_digest.as_bytes());
    let digest = hex(&hasher.finalize());
    (
        SourceSnapshot {
            revision,
            dirty,
            digest,
        },
        reasons,
    )
}

/// Archives the executed oracle plus the legacy Python asset/README generators
/// and their active invocation/config/lock inputs (see
/// `ANCILLARY_SOURCE_PATHS`). Directory walking skips VCS/build caches and
/// bytecode, and no credential-bearing local config is in the inventory.
fn archive_source(
    repo_root: &Path,
    private_root: &Path,
    source: &SourceSnapshot,
) -> Result<SourceArchiveInfo, String> {
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut source_paths: Vec<&str> = Vec::new();
    source_paths.extend_from_slice(ORACLE_SOURCE_PATHS);
    source_paths.extend_from_slice(ANCILLARY_SOURCE_PATHS);
    source_paths.extend_from_slice(HISTORICAL_SOURCE_PATHS);
    collect_archive_entries(repo_root, &source_paths, &mut entries)?;
    if entries.is_empty() {
        return Err("no baseline source files were found to archive".to_string());
    }

    let dirty_patch = if source.dirty {
        let mut diff_args = vec!["diff", "--binary", "HEAD", "--"];
        diff_args.extend(source_git_paths());
        git_bytes(repo_root, &diff_args).filter(|bytes| !bytes.is_empty())
    } else {
        None
    };

    let mut file_hashes: Vec<(String, String, u64)> = Vec::new();
    let mut digest_hasher = Sha256::new();
    for (relative, path) in &entries {
        let bytes =
            fs::read(path).map_err(|e| format!("cannot read a source file for archiving: {e}"))?;
        let hash = sha256_bytes(&bytes);
        digest_hasher.update(relative.as_bytes());
        digest_hasher.update(b"\0");
        digest_hasher.update(hash.as_bytes());
        digest_hasher.update(b"\n");
        file_hashes.push((relative.clone(), hash, bytes.len() as u64));
    }
    if let Some(patch) = &dirty_patch {
        let hash = sha256_bytes(patch);
        digest_hasher.update(b"dirty.patch\0");
        digest_hasher.update(hash.as_bytes());
        digest_hasher.update(b"\n");
        file_hashes.push(("dirty.patch".to_string(), hash, patch.len() as u64));
    }
    let digest = hex(&digest_hasher.finalize());
    let short_revision: String = source.revision.chars().take(12).collect();
    let directory_name = format!("{short_revision}-{}", &digest[..12]);
    let target = private_root.join(&directory_name);
    let manifest_path = target.join("manifest.json");

    if manifest_path.is_file() {
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            if let Ok(existing) = serde_json::from_str::<Value>(&text) {
                if existing.get("digest").and_then(Value::as_str) == Some(digest.as_str()) {
                    return Ok(SourceArchiveInfo {
                        location: format!("baseline-source/{directory_name}"),
                        digest,
                        file_count: file_hashes.len(),
                    });
                }
            }
        }
    }

    fs::create_dir_all(&target)
        .map_err(|e| format!("cannot create private source archive directory: {e}"))?;
    restrict_directory(&target)?;
    for (relative, path) in &entries {
        let destination = target.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create a private source archive subdirectory: {e}"))?;
        }
        fs::copy(path, &destination)
            .map_err(|e| format!("cannot copy a source file into the private archive: {e}"))?;
        restrict_file(&destination)?;
    }
    if let Some(patch) = &dirty_patch {
        let destination = target.join("dirty.patch");
        fs::write(&destination, patch)
            .map_err(|e| format!("cannot write the dirty source patch: {e}"))?;
        restrict_file(&destination)?;
    }

    let files: Vec<Value> = file_hashes
        .iter()
        .map(|(path, hash, size)| {
            json!({
                "path": path,
                "sha256": hash,
                "sizeBytes": size,
            })
        })
        .collect();
    let ancillary_file_count = file_hashes
        .iter()
        .filter(|(path, _, _)| is_ancillary_source_path(path))
        .count();
    let absent_historical: Vec<&str> = HISTORICAL_SOURCE_PATHS
        .iter()
        .copied()
        .filter(|relative| !repo_root.join(relative).exists())
        .collect();
    let manifest = json!({
        "schemaVersion": 1,
        "kind": "apk-lab-baseline-source-archive",
        "revision": source.revision,
        "dirty": source.dirty,
        "digest": digest,
        "fileCount": file_hashes.len(),
        "files": files,
        "ancillarySourcePaths": ANCILLARY_SOURCE_PATHS,
        "ancillaryFileCount": ancillary_file_count,
        "historicalSourcePaths": HISTORICAL_SOURCE_PATHS,
        "absentHistoricalSourcePaths": absent_historical,
        "reconstruction": "The archived files are the exact executed working tree, including the legacy Python asset/README generators and their active invocation/config/lock inputs that still existed when archived. Historical-only paths are never invoked by this harness; when their owners already removed them they are listed in absentHistoricalSourcePaths and the earlier frozen plan-001 archive remains authoritative for reconstruction. Verify every file sha256; dirty.patch is provenance evidence of the difference from the revision and must not be applied on top of the archive. This archive intentionally contains no APK/APKM/APKS bytes, credentials, caches, or local env/config files.",
    });
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| format!("cannot write the private source archive manifest: {e}"))?;
    restrict_file(&manifest_path)?;

    Ok(SourceArchiveInfo {
        location: format!("baseline-source/{directory_name}"),
        digest,
        file_count: file_hashes.len(),
    })
}

/// Collect every existing path from `source_paths`.
///
/// A listed path that no longer exists is expected for historical provenance
/// entries (plan 001 kept them for plan 008, while plan 006/007 owners delete
/// the files). Absence is therefore not an error; the archive manifest records
/// which historical paths were absent.
fn collect_archive_entries(
    repo_root: &Path,
    source_paths: &[&str],
    entries: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    for relative in source_paths {
        let path = repo_root.join(relative);
        if path.is_dir() {
            collect_source_entries(repo_root, &path, entries)?;
        } else if path.is_file() {
            entries.insert((*relative).to_string(), path);
        }
    }
    Ok(())
}

fn collect_source_entries(
    repo_root: &Path,
    directory: &Path,
    entries: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    let mut children: Vec<PathBuf> = fs::read_dir(directory)
        .map_err(|e| format!("cannot read a source directory for archiving: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if matches!(name, ".venv" | "__pycache__" | ".pytest_cache" | "target")
            || name.ends_with(".pyc")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&child)
            .map_err(|e| format!("cannot stat a source file for archiving: {e}"))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_source_entries(repo_root, &child, entries)?;
        } else if metadata.is_file() {
            let relative = child
                .strip_prefix(repo_root)
                .map_err(|_| "source file is outside the repository".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            entries.insert(relative, child);
        }
    }
    Ok(())
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_bytes(repo_root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot restrict private archive directory permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict private archive file permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Paths and small utilities
// ---------------------------------------------------------------------------

fn resolve_repo_root(opts: &Options, manifest: &BenchmarkManifest) -> Result<PathBuf, String> {
    if let Some(path) = &opts.repo_root {
        return canonical_or_absolute(path);
    }
    if let Some(path) = &manifest.repository_root {
        if Path::new(path).is_absolute() {
            return canonical_or_absolute(Path::new(path));
        }
        let manifest_dir = opts.manifest.parent().unwrap_or_else(|| Path::new("."));
        return canonical_or_absolute(&manifest_dir.join(path));
    }
    find_repo_root(&None)
}

fn find_repo_root(explicit: &Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return canonical_or_absolute(path);
    }
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !value.is_empty() {
                return canonical_or_absolute(Path::new(&value));
            }
        }
    }
    canonical_or_absolute(&std::env::current_dir().map_err(|e| {
        format!("cannot determine repository root and current directory is unavailable: {e}")
    })?)
}

fn canonical_or_absolute(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|e| format!("cannot resolve path {}: {e}", path.display()))
    }
}

fn resolve_against(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn display_path(path: &Path, repo_root: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(repo_root) {
        relative.to_string_lossy().replace('\\', "/")
    } else {
        "<private>".to_string()
    }
}

fn display_cwd(path: &Path, repo_root: &Path) -> String {
    let shown = display_path(path, repo_root);
    if shown.is_empty() {
        ".".to_string()
    } else {
        shown
    }
}

pub(crate) fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub(crate) fn write_json_atomic(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "cannot create benchmark output directory {}: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| format!("cannot serialize benchmark report: {e}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, format!("{text}\n"))
        .map_err(|e| format!("cannot write benchmark report {}: {e}", temporary.display()))?;
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&temporary, path)
            .map_err(|e| format!("cannot publish benchmark report {}: {e}", path.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Result sanitization (`bench --sanitize`)
// ---------------------------------------------------------------------------

const SANITIZE_REPO: &str = "<repo>";
const SANITIZE_CORPUS_PREFIX: &str = "<corpus>/";
const SANITIZE_ARCHIVE_LOCATION: &str = "<private-source-archive>";
const SANITIZE_RECONSTRUCTION: &str = "<private-reconstruction-recipe>";
const SANITIZE_HOST_PREFIX: &str = "<host:";
const SANITIZE_STORAGE_MOUNT: &str = "<storage-mount>";
const SANITIZE_STORAGE_DEVICE: &str = "<storage-device>";

/// Pure JSON transform: read one private benchmark result and publish a
/// sanitized copy. Nothing is executed, timed, or resolved against a manifest;
/// the transform either produces a fully validated public document or fails
/// without writing `out`.
fn sanitize_result(input: &Path, out: &Path) -> Result<(), String> {
    let bytes = fs::read(input)
        .map_err(|e| format!("cannot read benchmark result {}: {e}", input.display()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "benchmark result {} is not valid JSON: {e}",
            input.display()
        )
    })?;
    let sanitized = sanitize_result_value(&value)?;
    write_json_atomic(out, &sanitized)
}

/// Sanitize an already-parsed private result. Every rejection happens before
/// the caller serializes, so a rejected input can never leave a public file.
fn sanitize_result_value(value: &Value) -> Result<Value, String> {
    validate_result_shape(value)?;
    let declared_paths = collect_declared_input_paths(value);

    let mut sanitized = value.clone();
    let object = sanitized
        .as_object_mut()
        .ok_or_else(|| "benchmark result must be a JSON object".to_string())?;

    {
        let engine = object
            .get_mut("engine")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                "benchmark result engine must be an object with executable and cwd".to_string()
            })?;
        let executable = engine
            .get("executable")
            .and_then(Value::as_str)
            .ok_or_else(|| "benchmark result engine.executable must be a string".to_string())?;
        let file_name = engine_file_name(executable)?;
        engine.insert("executable".to_string(), Value::String(file_name));
        if engine.get("cwd").and_then(Value::as_str).is_none() {
            return Err("benchmark result engine.cwd must be a string".to_string());
        }
        engine.insert("cwd".to_string(), Value::String(SANITIZE_REPO.to_string()));
    }

    {
        // Host identity must stay publishable without exposing the machine name
        // or a storage path that can legitimately contain a path marker such as
        // `/System/Volumes/Data`.
        let environment = object
            .get_mut("environment")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "benchmark result environment must be an object".to_string())?;
        if let Some(hostname) = environment.get("hostname").and_then(Value::as_str) {
            if hostname.is_empty() {
                return Err("benchmark result environment.hostname must not be empty".to_string());
            }
            let digest = sha256_bytes(hostname.as_bytes());
            environment.insert(
                "hostname".to_string(),
                Value::String(format!("{SANITIZE_HOST_PREFIX}{}>", &digest[..12])),
            );
        }
        if let Some(executable) = environment.get("oracleExecutable").and_then(Value::as_str) {
            let file_name = engine_file_name(executable)?;
            environment.insert("oracleExecutable".to_string(), Value::String(file_name));
        }
        if let Some(storage) = environment
            .get_mut("storage")
            .and_then(Value::as_object_mut)
        {
            if storage.get("mountPoint").map(Value::is_string) == Some(true) {
                storage.insert(
                    "mountPoint".to_string(),
                    Value::String(SANITIZE_STORAGE_MOUNT.to_string()),
                );
            }
            if storage.get("filesystem").map(Value::is_string) == Some(true) {
                storage.insert(
                    "filesystem".to_string(),
                    Value::String(SANITIZE_STORAGE_DEVICE.to_string()),
                );
            }
        }
    }

    {
        let source = object
            .get_mut("source")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "benchmark result source must be an object".to_string())?;
        match source.get_mut("archive") {
            Some(Value::Null) => {}
            Some(Value::Object(archive)) => {
                if archive.get("location").and_then(Value::as_str).is_none() {
                    return Err(
                        "benchmark result source.archive.location must be a string".to_string()
                    );
                }
                if archive
                    .get("reconstruction")
                    .and_then(Value::as_str)
                    .is_none()
                {
                    return Err(
                        "benchmark result source.archive.reconstruction must be a string"
                            .to_string(),
                    );
                }
                archive.insert(
                    "location".to_string(),
                    Value::String(SANITIZE_ARCHIVE_LOCATION.to_string()),
                );
                archive.insert(
                    "reconstruction".to_string(),
                    Value::String(SANITIZE_RECONSTRUCTION.to_string()),
                );
                if archive.get("digest").and_then(Value::as_str).is_none()
                    || archive.get("fileCount").and_then(Value::as_u64).is_none()
                    || archive.get("kind").and_then(Value::as_str).is_none()
                {
                    return Err(
                        "benchmark result source.archive must keep kind, digest, and fileCount"
                            .to_string(),
                    );
                }
            }
            Some(_) => {
                return Err("benchmark result source.archive must be an object or null".to_string())
            }
            None => return Err(
                "benchmark result source.archive is missing; use null when no archive was written"
                    .to_string(),
            ),
        }
    }

    {
        let workloads = object
            .get_mut("workloads")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "benchmark result workloads must be an array".to_string())?;
        if workloads.is_empty() {
            return Err("benchmark result workloads must not be empty".to_string());
        }
        for (index, workload) in workloads.iter_mut().enumerate() {
            sanitize_workload(workload, index)?;
        }
    }

    validate_sanitized_result(&sanitized, &declared_paths)?;
    Ok(sanitized)
}

/// Keep only the executable's file name; a path is never echoed.
fn engine_file_name(executable: &str) -> Result<String, String> {
    let trimmed = executable.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!(
            "benchmark result engine.executable has no usable file name: '{executable}'"
        ));
    }
    Ok(name.to_string())
}

/// Private paths that must never survive into the public document. Only the
/// declared input locations are collected; those exact strings are checked
/// after sanitization as a fail-closed leak detector.
fn collect_declared_input_paths(value: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let Some(workloads) = value.get("workloads").and_then(Value::as_array) else {
        return paths;
    };
    for workload in workloads {
        let mut statuses: Vec<&Value> = workload
            .get("inputs")
            .and_then(Value::as_array)
            .map(|inputs| inputs.iter().collect())
            .unwrap_or_default();
        if let Some(mpp) = workload.get("mpp").filter(|mpp| mpp.is_object()) {
            statuses.push(mpp);
        }
        for status in statuses {
            for key in ["displayPath", "path"] {
                if let Some(path) = status.get(key).and_then(Value::as_str) {
                    let path = path.trim();
                    if path.len() > 1 && !path.starts_with('<') && !paths.iter().any(|p| p == path)
                    {
                        paths.push(path.to_string());
                    }
                }
            }
        }
    }
    paths
}

fn sanitize_workload(workload: &mut Value, index: usize) -> Result<(), String> {
    let id = workload
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("workload {index} id must be a string"))?
        .to_string();
    let label = format!("workload {index} ('{id}')");
    let object = workload
        .as_object_mut()
        .ok_or_else(|| format!("{label} must be an object"))?;

    let inputs = object
        .get_mut("inputs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("{label} inputs must be an array"))?;
    for (position, input) in inputs.iter_mut().enumerate() {
        sanitize_input_status(input, &format!("{label} input {position}"))?;
    }

    match object.get_mut("mpp") {
        Some(Value::Null) => {}
        Some(mpp) => sanitize_input_status(mpp, &format!("{label} mpp"))?,
        None => {
            return Err(format!(
                "{label} mpp is missing; use null when the workload has no mpp"
            ))
        }
    }

    let argv_template = object.get("argvTemplate").cloned();
    match argv_template {
        Some(Value::Array(argv)) => {
            let mut tokens = Vec::with_capacity(argv.len());
            for token in &argv {
                let token = token
                    .as_str()
                    .ok_or_else(|| format!("{label} argvTemplate entries must be strings"))?;
                tokens.push(token.to_string());
            }
            object.insert("command".to_string(), sanitized_command(&tokens));
            let _ = object.remove("argvTemplate");
        }
        Some(_) => return Err(format!("{label} argvTemplate must be an array of strings")),
        None => {
            let command = object
                .get("command")
                .ok_or_else(|| format!("{label} has neither argvTemplate nor command"))?
                .clone();
            validate_command(&command, &label)?;
        }
    }

    if object.get("samples").map(Value::is_array) != Some(true) {
        return Err(format!("{label} samples must be an array"));
    }
    Ok(())
}

/// Replace the input location with `<corpus>/<declaredSha256 first 12>`.
/// Only the declared digest is used, never any segment of the private path.
fn sanitize_input_status(status: &mut Value, label: &str) -> Result<(), String> {
    let object = status
        .as_object_mut()
        .ok_or_else(|| format!("{label} must be an object"))?;
    let declared = object
        .get("declaredSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} declaredSha256 is missing"))?;
    if declared.len() != 64 || !declared.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "{label} declaredSha256 is not a 64-character hex digest"
        ));
    }
    let placeholder = format!("{SANITIZE_CORPUS_PREFIX}{}", &declared[..12]);
    let mut replaced = false;
    for key in ["displayPath", "path"] {
        if object.get(key).map(Value::is_string) == Some(true) {
            object.insert(key.to_string(), Value::String(placeholder.clone()));
            replaced = true;
        }
    }
    if !replaced {
        return Err(format!("{label} has neither displayPath nor path"));
    }
    Ok(())
}

/// Derive a command summary from an argv template. The first non-flag token is
/// the subcommand (null when the argv has none); every token starting with '-'
/// is a flag, and flag names strip any `=` value. No semantics are inferred.
fn sanitized_command(argv: &[String]) -> Value {
    let subcommand = argv
        .iter()
        .find(|token| !token.starts_with('-'))
        .cloned()
        .map(Value::String)
        .unwrap_or(Value::Null);
    let flags: Vec<String> = argv
        .iter()
        .filter(|token| token.starts_with('-'))
        .cloned()
        .collect();
    let flag_names: Vec<String> = flags
        .iter()
        .map(|flag| flag.split('=').next().unwrap_or(flag).to_string())
        .collect();
    json!({
        "subcommand": subcommand,
        "flags": flags,
        "flagNames": flag_names,
    })
}

fn validate_command(command: &Value, label: &str) -> Result<(), String> {
    let object = command
        .as_object()
        .ok_or_else(|| format!("{label} command must be an object"))?;
    let expected = ["subcommand", "flags", "flagNames"];
    if object.len() != expected.len() || !expected.iter().all(|key| object.contains_key(*key)) {
        return Err(format!(
            "{label} command must contain exactly subcommand, flags, and flagNames"
        ));
    }
    if !object["subcommand"].is_null() && object["subcommand"].as_str().is_none() {
        return Err(format!(
            "{label} command.subcommand must be a string or null"
        ));
    }
    for key in ["flags", "flagNames"] {
        let values = object[key]
            .as_array()
            .ok_or_else(|| format!("{label} command.{key} must be an array"))?;
        if values.iter().any(|item| item.as_str().is_none()) {
            return Err(format!("{label} command.{key} entries must be strings"));
        }
    }
    Ok(())
}

/// Known host roots. `/proc`, `/dev`, `/sys`, and `/usr` are deliberately
/// absent so documented metric methods (`/proc/<pid>/stat`) stay publishable.
const PRIVATE_PATH_MARKERS: &[&str] = &[
    "/Users/",
    "/home/",
    "/root/",
    "/tmp/",
    "/var/",
    "/private/",
    "/Volumes/",
    "/opt/",
    "/mnt/",
    "/srv/",
    "/etc/",
    "/Library/",
    "~/",
    ".apk-lab-migration",
    "/apk/",
    "\\apk\\",
    "\\Users\\",
];

fn private_path_markers() -> Vec<String> {
    let mut markers: Vec<String> = PRIVATE_PATH_MARKERS
        .iter()
        .map(|marker| marker.to_string())
        .collect();
    for key in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(key) {
            let text = home
                .to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .to_string();
            if text.len() > 1 && text != "/" && !markers.iter().any(|m| m == &text) {
                markers.push(text);
            }
        }
    }
    markers
}

fn first_private_path_marker<'a>(
    text: &str,
    markers: &'a [String],
    declared: &'a [String],
) -> Option<&'a str> {
    if let Some(marker) = markers.iter().find(|marker| text.contains(marker.as_str())) {
        return Some(marker.as_str());
    }
    declared
        .iter()
        .find(|path| text.contains(path.as_str()))
        .map(String::as_str)
}

fn visit_strings(
    value: &Value,
    visit: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => {
            for item in items {
                visit_strings(item, visit)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for item in map.values() {
                visit_strings(item, visit)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Post-transform guard. Re-derives the expected placeholders from the
/// sanitized document and rejects any surviving host or declared private path.
fn validate_sanitized_result(value: &Value, declared_paths: &[String]) -> Result<(), String> {
    let engine = value
        .get("engine")
        .and_then(Value::as_object)
        .ok_or_else(|| "sanitized result is missing engine".to_string())?;
    let executable = engine
        .get("executable")
        .and_then(Value::as_str)
        .ok_or_else(|| "sanitized result is missing engine.executable".to_string())?;
    if executable.contains('/') || executable.contains('\\') {
        return Err("sanitized engine.executable still contains a path separator".to_string());
    }
    if engine.get("cwd").and_then(Value::as_str) != Some(SANITIZE_REPO) {
        return Err(format!(
            "sanitized engine.cwd must be exactly {SANITIZE_REPO}"
        ));
    }
    let source = value
        .get("source")
        .and_then(Value::as_object)
        .ok_or_else(|| "sanitized result is missing source".to_string())?;
    match source.get("archive") {
        Some(Value::Null) => {}
        Some(Value::Object(archive)) => {
            if archive.get("location").and_then(Value::as_str) != Some(SANITIZE_ARCHIVE_LOCATION) {
                return Err(format!(
                    "sanitized source.archive.location must be exactly {SANITIZE_ARCHIVE_LOCATION}"
                ));
            }
            if archive.get("reconstruction").and_then(Value::as_str)
                != Some(SANITIZE_RECONSTRUCTION)
            {
                return Err(format!(
                    "sanitized source.archive.reconstruction must be exactly {SANITIZE_RECONSTRUCTION}"
                ));
            }
        }
        _ => return Err("sanitized source.archive must be an object or null".to_string()),
    }

    let workloads = value
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "sanitized result is missing workloads".to_string())?;
    for (index, workload) in workloads.iter().enumerate() {
        let id = workload
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("sanitized workload {index} is missing id"))?;
        if workload.get("argvTemplate").is_some() {
            return Err(format!("sanitized workload '{id}' still has argvTemplate"));
        }
        validate_command(
            workload
                .get("command")
                .ok_or_else(|| format!("sanitized workload '{id}' is missing command"))?,
            &format!("sanitized workload '{id}'"),
        )?;
        let mut statuses: Vec<&Value> = workload
            .get("inputs")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("sanitized workload '{id}' is missing inputs"))?
            .iter()
            .collect();
        match workload.get("mpp") {
            Some(Value::Null) => {}
            Some(mpp) if mpp.is_object() => statuses.push(mpp),
            _ => {
                return Err(format!(
                    "sanitized workload '{id}' mpp must be an object or null"
                ))
            }
        }
        for status in statuses {
            let declared = status
                .get("declaredSha256")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!("sanitized workload '{id}' input is missing declaredSha256")
                })?;
            let expected = format!("{SANITIZE_CORPUS_PREFIX}{}", &declared[..12]);
            let mut checked = false;
            for key in ["displayPath", "path"] {
                if let Some(path) = status.get(key).and_then(Value::as_str) {
                    if path != expected {
                        return Err(format!(
                            "sanitized workload '{id}' {key} must be exactly '{expected}'"
                        ));
                    }
                    checked = true;
                }
            }
            if !checked {
                return Err(format!(
                    "sanitized workload '{id}' input has no sanitized path"
                ));
            }
        }
    }

    let markers = private_path_markers();
    visit_strings(value, &mut |text| {
        if let Some(marker) = first_private_path_marker(text, &markers, declared_paths) {
            return Err(format!(
                "sanitized result still contains a private path marker '{marker}'"
            ));
        }
        Ok(())
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Baseline verification helpers (used by tests; deliberately independent of run)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn load_baseline(path: &Path) -> Result<Value, String> {
    if !path.is_file() {
        return Err(format!("baseline file does not exist: {}", path.display()));
    }
    let bytes =
        fs::read(path).map_err(|e| format!("cannot read baseline {}: {e}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("baseline {} is not valid JSON: {e}", path.display()))?;
    if value.get("kind").and_then(Value::as_str) != Some("apk-lab-benchmark-result") {
        return Err(format!(
            "baseline {} is not an apk-lab-benchmark-result",
            path.display()
        ));
    }
    validate_result_shape(&value)?;
    Ok(value)
}

/// Compares observable baseline fields and ignores approved volatile fields
/// (`generatedAtEpochSeconds`, `durationMs`, environment, and raw wall times).
#[allow(dead_code)]
fn diff_baseline_results(expected: &Value, actual: &Value) -> Vec<String> {
    let mut differences = Vec::new();
    for field in ["schemaVersion", "kind", "status", "profile"] {
        if expected.get(field) != actual.get(field) {
            differences.push(format!("top-level field '{field}' differs"));
        }
    }
    if expected
        .get("acceptance")
        .and_then(|value| value.get("valid"))
        != actual
            .get("acceptance")
            .and_then(|value| value.get("valid"))
    {
        differences.push("acceptance.valid differs".to_string());
    }

    let expected_workloads = workloads_by_id(expected);
    let actual_workloads = workloads_by_id(actual);
    let mut ids: BTreeSet<&String> = expected_workloads.keys().collect();
    ids.extend(actual_workloads.keys());
    for id in ids {
        let expected_workload = expected_workloads.get(id);
        let actual_workload = actual_workloads.get(id);
        match (expected_workload, actual_workload) {
            (Some(expected), Some(actual)) => {
                for field in ["status", "samplesCompleted", "warmupsCompleted"] {
                    if expected.get(field) != actual.get(field) {
                        differences.push(format!("workload '{id}' field '{field}' differs"));
                    }
                }
                let expected_samples = expected
                    .get("samples")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let actual_samples = actual
                    .get("samples")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if expected_samples.len() != actual_samples.len() {
                    differences.push(format!(
                        "workload '{id}' sample count differs: expected {}, actual {}",
                        expected_samples.len(),
                        actual_samples.len()
                    ));
                }
                for (index, (expected_sample, actual_sample)) in expected_samples
                    .iter()
                    .zip(actual_samples.iter())
                    .enumerate()
                {
                    for field in [
                        "kind",
                        "cacheState",
                        "exitCode",
                        "stdoutSha256",
                        "outputValid",
                    ] {
                        if expected_sample.get(field) != actual_sample.get(field) {
                            differences.push(format!(
                                "workload '{id}' sample {index} field '{field}' differs"
                            ));
                        }
                    }
                }
            }
            (Some(_), None) => differences.push(format!("workload '{id}' is missing")),
            (None, Some(_)) => differences.push(format!("workload '{id}' was added")),
            (None, None) => {}
        }
    }
    differences
}

#[allow(dead_code)]
fn workloads_by_id(value: &Value) -> BTreeMap<String, Value> {
    value
        .get("workloads")
        .and_then(Value::as_array)
        .map(|workloads| {
            workloads
                .iter()
                .filter_map(|workload| {
                    workload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|id| (id.to_string(), workload.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Unit tests: schema, sample-count, hash, missing baseline, and mutation checks
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PerformancePolicy {
        serde_json::from_str(include_str!("../../migration/performance-policy.json"))
            .expect("policy file must be valid JSON")
    }

    fn manifest_json() -> Value {
        json!({
            "schemaVersion": 1,
            "profile": "local",
            "engine": {
                "name": "python",
                "executable": "tools/apk-lab/.venv/bin/python",
                "cwd": "tools/apk-lab",
                "argsPrefix": ["-B", "-m", "apk_lab.cli"],
                "env": {"PYTHONDONTWRITEBYTECODE": "1"}
            },
            "workloads": [{
                "id": "startup.help",
                "category": "startup",
                "size": "small",
                "costly": false,
                "argv": ["--help"],
                "expected": {
                    "exitCode": 0,
                    "stdoutSha256": "0000000000000000000000000000000000000000000000000000000000000000"
                }
            }]
        })
    }

    fn parse_manifest(value: Value) -> BenchmarkManifest {
        serde_json::from_value(value).expect("manifest must parse")
    }

    #[test]
    fn checked_in_policy_is_valid_and_preregisters_thresholds() {
        let policy = policy();
        validate_policy(&policy).expect("checked-in policy must validate");
    }

    fn capability(cpu: bool, rss: bool, io: bool) -> MetricCapability {
        MetricCapability {
            cpu_supported: cpu,
            rss_supported: rss,
            io_supported: io,
            units: None,
            reason: Some("host has no tested sampler for this metric".to_string()),
        }
    }

    #[test]
    fn policy_blocking_io_makes_unsupported_io_fatal() {
        let mut policy = policy();
        policy.budgets.get_mut("io").unwrap().block_on_worse = Some(true);
        let evaluation = evaluate_metric_policy(&policy, &capability(true, true, false));
        assert_eq!(evaluation.limitations.len(), 1);
        assert_eq!(evaluation.limitations[0].metric, "io");
        assert!(evaluation.limitations[0].blocking);
        assert!(evaluation
            .blockers
            .iter()
            .any(|reason| reason.contains("process IO")));
    }

    #[test]
    fn policy_nonblocking_io_keeps_null_with_explanation_without_blocker() {
        let policy = policy();
        let evaluation = evaluate_metric_policy(&policy, &capability(true, true, false));
        assert!(
            evaluation.blockers.is_empty(),
            "a non-blocking unsupported metric must not fail acceptance"
        );
        let io = evaluation
            .limitations
            .iter()
            .find(|limitation| limitation.metric == "io")
            .expect("unsupported io must be recorded as a limitation");
        assert!(!io.blocking);
        assert!(io.requires_explanation);
        assert!(io.reason.contains("no tested sampler"));

        let value = metric_value(
            false,
            "bytes",
            IO_METRIC_METHOD,
            Some("no non-FFI per-process byte counter on this host"),
        );
        assert_eq!(value["supported"], json!(false));
        assert!(value["unit"].is_null());
        assert!(value.get("method").unwrap().is_null());
        assert_eq!(
            value["reason"],
            json!("no non-FFI per-process byte counter on this host")
        );
        let metrics = ProcessMetrics::unsupported();
        assert!(metrics.read_bytes.is_none());
        assert!(metrics.write_bytes.is_none());
        assert!(metrics.cpu_ms.is_none());
    }

    #[test]
    fn policy_blocking_cpu_with_unsupported_cpu_is_fatal() {
        let policy = policy();
        let evaluation = evaluate_metric_policy(&policy, &capability(false, true, true));
        let cpu = evaluation
            .limitations
            .iter()
            .find(|limitation| limitation.metric == "cpu")
            .expect("unsupported cpu must be recorded");
        assert!(cpu.blocking);
        assert!(evaluation
            .blockers
            .iter()
            .any(|reason| reason.contains("CPU-time")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_cpu_method_reports_ps_sampling_without_exactness_claim() {
        assert_eq!(HOST_SAMPLER, "macos-ps");
        assert!(CPU_METRIC_METHOD.contains("not exact per-sample"));
        let value = metric_value(true, "milliseconds", CPU_METRIC_METHOD, None);
        assert_eq!(value["sampler"], json!("macos-ps"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_metrics_declare_proc_sampling() {
        assert_eq!(HOST_SAMPLER, "linux-proc");
        let value = metric_value(true, "bytes", IO_METRIC_METHOD, None);
        assert_eq!(value["sampler"], json!("linux-proc"));
    }

    #[test]
    fn policy_rejects_drifted_sample_counts() {
        let mut value: Value =
            serde_json::from_str(include_str!("../../migration/performance-policy.json")).unwrap();
        value["sampleClasses"]["local"]["samples"] = json!(29);
        let policy: PerformancePolicy = serde_json::from_value(value).unwrap();
        assert!(validate_policy(&policy).is_err());
    }

    #[test]
    fn manifest_rejects_wrong_schema_version() {
        let mut value = manifest_json();
        value["schemaVersion"] = json!(2);
        let manifest = parse_manifest(value);
        assert!(validate_manifest(&manifest, &policy(), "local").is_err());
    }

    #[test]
    fn manifest_rejects_sample_counts_below_policy() {
        let mut value = manifest_json();
        value["workloads"][0]["samples"] = json!(29);
        let manifest = parse_manifest(value);
        let error = validate_manifest(&manifest, &policy(), "local").unwrap_err();
        assert!(error.contains("pre-registered minimum"));
    }

    #[test]
    fn manifest_rejects_costly_sample_counts_below_policy() {
        let mut value = manifest_json();
        value["workloads"][0]["costly"] = json!(true);
        value["workloads"][0]["samples"] = json!(9);
        let manifest = parse_manifest(value);
        assert!(validate_manifest(&manifest, &policy(), "local").is_err());
    }

    #[test]
    fn manifest_rejects_malformed_input_hash() {
        let mut value = manifest_json();
        value["workloads"][0]["inputs"] = json!([{
            "role": "artifact",
            "path": "corpus/app.apk",
            "sha256": "abc"
        }]);
        let manifest = parse_manifest(value);
        let error = validate_manifest(&manifest, &policy(), "local").unwrap_err();
        assert!(error.contains("64 hexadecimal"));
    }

    #[test]
    fn environment_allowlist_rejects_secrets_and_injection_keys() {
        assert!(check_env_key("PYTHONDONTWRITEBYTECODE").is_ok());
        assert!(check_env_key("ANDROID_HOME").is_ok());
        assert!(check_env_key("LD_PRELOAD").is_err());
        assert!(check_env_key("PLAY_TOKEN").is_err());
        assert!(check_env_key("GITHUB_TOKEN").is_err());
    }

    #[test]
    fn manifest_rejects_uv_execution() {
        let mut value = manifest_json();
        value["engine"]["executable"] = json!("uv");
        let manifest = parse_manifest(value);
        let error = validate_manifest(&manifest, &policy(), "local").unwrap_err();
        assert!(error.contains("uv"));
    }

    #[test]
    fn input_hash_verification_detects_mutation() {
        let directory = test_directory("hash");
        let path = directory.join("artifact.apk");
        fs::write(&path, b"original bytes").unwrap();
        let expected = sha256_bytes(b"original bytes");
        assert!(verify_file_hash(&path, &expected).is_ok());
        fs::write(&path, b"mutated bytes").unwrap();
        let error = verify_file_hash(&path, &expected).unwrap_err();
        assert!(error.contains("sha256 mismatch"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_baseline_is_an_error_not_an_empty_result() {
        let directory = test_directory("missing-baseline");
        let missing = directory.join("baseline.json");
        let error = load_baseline(&missing).unwrap_err();
        assert!(error.contains("does not exist"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn baseline_mutations_are_detected() {
        let expected = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-result",
            "status": "complete",
            "profile": "local",
            "acceptance": {"valid": true, "reasons": []},
            "source": {},
            "workloads": [{
                "id": "startup.help",
                "status": "measured",
                "samplesCompleted": 1,
                "warmupsCompleted": 1,
                "samples": [{
                    "exitCode": 0,
                    "stdoutSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "outputValid": true
                }]
            }]
        });
        let mut actual = expected.clone();
        assert!(diff_baseline_results(&expected, &actual).is_empty());

        actual["workloads"][0]["samples"][0]["stdoutSha256"] =
            json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert!(!diff_baseline_results(&expected, &actual).is_empty());

        let mut status_changed = expected.clone();
        status_changed["workloads"][0]["status"] = json!("invalid-output");
        assert!(!diff_baseline_results(&expected, &status_changed).is_empty());

        let mut exit_changed = expected.clone();
        exit_changed["workloads"][0]["samples"][0]["exitCode"] = json!(2);
        assert!(!diff_baseline_results(&expected, &exit_changed).is_empty());
    }

    #[test]
    fn result_shape_requires_acceptance_and_workloads() {
        let bad = json!({"schemaVersion": 1, "kind": "apk-lab-benchmark-result"});
        assert!(validate_result_shape(&bad).is_err());
    }

    #[test]
    fn ancillary_inventory_covers_legacy_generators_and_invocation_inputs() {
        for generator in [
            ".github/scripts/generate_patches_readme.py",
            ".github/scripts/prepare_emoji_font.py",
            ".github/scripts/prepare_ocr_model.py",
            ".github/scripts/prepare_pdfbox_assets.py",
            ".github/scripts/sync_app_icons.py",
            "extensions/adobescan-pdf-native/tests/fixtures/generate_fixtures.py",
        ] {
            assert!(is_ancillary_source_path(generator), "{generator}");
        }
        for input in [
            "patches-list.json",
            ".releaserc",
            "patches/build.gradle.kts",
            "gradle/libs.versions.toml",
            "site/package.json",
            "scripts/audit.ts",
        ] {
            assert!(is_ancillary_source_path(input), "{input}");
        }
        for excluded in [
            "local.properties",
            ".env",
            "tools/apk-lab/apk_lab/cli.py",
            "site/public/icons/example.png",
            ".gradle/caches/modules.bin",
        ] {
            assert!(!is_ancillary_source_path(excluded), "{excluded}");
        }
    }

    #[test]
    fn archive_entry_collection_tolerates_absent_historical_paths() {
        let directory = test_directory("archive-absent");
        fs::create_dir_all(directory.join(".github/scripts")).unwrap();
        fs::write(
            directory.join(".github/scripts/present.py"),
            b"print('x')\n",
        )
        .unwrap();
        fs::write(directory.join("patches-list.json"), b"{}").unwrap();

        let mut entries = BTreeMap::new();
        collect_archive_entries(
            &directory,
            &[
                ".github/scripts",
                "patches-list.json",
                ".github/scripts/generate_patches_readme.py",
                "extensions/adobescan-pdf-native/tests/fixtures/generate_fixtures.py",
            ],
            &mut entries,
        )
        .expect("absent historical provenance paths must not abort archiving");

        assert!(entries.contains_key(".github/scripts/present.py"));
        assert!(entries.contains_key("patches-list.json"));
        assert!(!entries.contains_key(".github/scripts/generate_patches_readme.py"));
        assert!(!entries
            .contains_key("extensions/adobescan-pdf-native/tests/fixtures/generate_fixtures.py"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn macos_ps_cpu_time_parses_tested_formats() {
        assert_eq!(parse_macos_ps_cpu_time("0:00.17"), Some(0.17));
        assert_eq!(parse_macos_ps_cpu_time("2:28.50"), Some(148.5));
        assert_eq!(parse_macos_ps_cpu_time("7:39.39"), Some(459.39));
        assert_eq!(parse_macos_ps_cpu_time("10:00"), Some(600.0));
        assert_eq!(parse_macos_ps_cpu_time("00:00.00"), Some(0.0));
        // Live macOS output for long-running processes accumulates minutes and
        // omits the hours field; rejecting these broke every sampled metric.
        assert_eq!(parse_macos_ps_cpu_time("71:24.25"), Some(4_284.25));
        assert_eq!(parse_macos_ps_cpu_time("190:44.36"), Some(11_444.36));
        assert_eq!(parse_macos_ps_cpu_time("238:02.23"), Some(14_282.23));
        assert_eq!(
            parse_macos_ps_cpu_time("1-02:03:04"),
            Some(86_400.0 + 2.0 * 3_600.0 + 3.0 * 60.0 + 4.0)
        );
        for invalid in [
            "", "abc", "1:2:3:4", "1:60:00", "1:00:60", "-1:00", "1:00:nan", "0:60",
        ] {
            assert_eq!(parse_macos_ps_cpu_time(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn macos_ps_row_uses_kib_rss_and_centisecond_cpu() {
        let row = parse_macos_ps_row("  123    1   123  22576   2:28.50 ").unwrap();
        assert_eq!(
            row,
            MacPsRow {
                pid: 123,
                ppid: 1,
                pgid: 123,
                rss_kib: 22_576,
                cpu_seconds: 148.5,
            }
        );
        for invalid in [
            "",
            "123 1 123 22576",
            "123 1 123 22576 2:28.50 extra",
            "abc 1 123 22576 2:28.50",
            "123 1 123 -1 2:28.50",
        ] {
            assert_eq!(parse_macos_ps_row(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn macos_tree_snapshot_selects_group_and_parent_descendants() {
        let rows: Vec<MacPsRow> = [
            "100 1 100 1000 0:00.50",
            "101 100 100 500 0:00.25",
            "102 101 100 250 0:00.13",
            "103 1 100 100 0:00.10",
            "104 102 999 400 0:00.01",
            "200 1 200 999 9:00.00",
        ]
        .iter()
        .map(|line| parse_macos_ps_row(line).unwrap())
        .collect();
        let snapshot = macos_tree_snapshot(&rows, 100).unwrap();
        assert_eq!(snapshot.cpu_by_pid.len(), 5);
        let cpu_seconds: f64 = snapshot.cpu_by_pid.values().sum();
        assert!((cpu_seconds - 0.99).abs() < 1e-9, "{cpu_seconds}");
        assert!(!snapshot.cpu_by_pid.contains_key(&200));
        assert_eq!(snapshot.rss_bytes, 2_250 * MACOS_PS_RSS_UNIT_BYTES);
    }

    #[test]
    fn macos_tree_snapshot_fails_closed_without_a_matching_process() {
        assert!(macos_tree_snapshot(&[], 100).is_none());
        let unrelated = parse_macos_ps_row("200 1 200 999 0:01.00").unwrap();
        assert!(macos_tree_snapshot(&[unrelated], 100).is_none());
        let root = parse_macos_ps_row("100 1 100 10 0:00.01").unwrap();
        assert!(macos_tree_snapshot(&[root, root], 100).is_none());
    }

    #[test]
    fn manifest_rejects_required_tools_without_a_tool_cache_source() {
        let mut value = manifest_json();
        value["workloads"][0]["requiredTools"] = json!(["morphe"]);
        let manifest = parse_manifest(value);
        let error = validate_manifest(&manifest, &policy(), "local").unwrap_err();
        assert!(error.contains("toolCache"));
    }

    #[test]
    fn manifest_accepts_tool_cache_with_strict_semantics() {
        let mut value = manifest_json();
        value["toolCache"] = json!({
            "source": "~/.cache/aidans-patches/apk-lab/tools",
            "strict": true
        });
        value["workloads"][0]["requiredTools"] = json!(["morphe"]);
        let manifest = parse_manifest(value);
        validate_manifest(&manifest, &policy(), "local").expect("manifest must validate");
    }

    #[test]
    fn tool_cache_preparation_stages_only_required_pinned_tools() {
        let directory = test_directory("tool-cache");
        let engine_cwd = directory.join("engine");
        fs::create_dir_all(&engine_cwd).unwrap();
        let source = directory.join("cache");
        let payload = b"pinned morphe jar bytes";
        let sha = sha256_bytes(payload);
        let install = source.join("morphe").join("1.0.0").join(&sha);
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("morphe.jar"), payload).unwrap();
        let other_payload = b"unrequired jadx bytes";
        let other_sha = sha256_bytes(other_payload);
        let other_install = source.join("jadx").join("2.0.0").join(&other_sha);
        fs::create_dir_all(&other_install).unwrap();
        fs::write(other_install.join("jadx"), other_payload).unwrap();
        let lock = json!({
            "tools": {
                "morphe": {
                    "version": "1.0.0",
                    "sha256": sha.clone(),
                    "toolType": "jar",
                    "executablePath": "morphe.jar"
                },
                "jadx": {
                    "version": "2.0.0",
                    "sha256": other_sha,
                    "toolType": "binary",
                    "executablePath": "jadx"
                }
            }
        });
        fs::write(
            engine_cwd.join("tools.lock.json"),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        let spec = ToolCacheSpec {
            source: source.display().to_string(),
            strict: true,
        };
        let required = BTreeSet::from(["morphe".to_string()]);
        let prepared = prepare_tool_cache(&directory, &engine_cwd, Some(&spec), &required).unwrap();
        assert!(prepared.issues.is_empty());
        assert_eq!(prepared.tools.len(), 1);
        assert_eq!(prepared.tools[0].name, "morphe");
        let staged = prepared
            .cache_root()
            .unwrap()
            .join("morphe")
            .join("1.0.0")
            .join(&sha)
            .join("morphe.jar");
        assert_eq!(fs::read(&staged).unwrap(), payload);
        assert_eq!(fs::read(install.join("morphe.jar")).unwrap(), payload);
        assert!(!prepared.cache_root().unwrap().join("jadx").exists());
        drop(prepared);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn strict_tool_cache_preflight_fails_on_unpinned_required_tool() {
        let directory = test_directory("tool-cache-unpinned");
        let engine_cwd = directory.join("engine");
        fs::create_dir_all(&engine_cwd).unwrap();
        let source = directory.join("cache");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            engine_cwd.join("tools.lock.json"),
            serde_json::to_vec(&json!({"tools": {}})).unwrap(),
        )
        .unwrap();
        let spec = ToolCacheSpec {
            source: source.display().to_string(),
            strict: true,
        };
        let required = BTreeSet::from(["morphe".to_string()]);
        let error =
            prepare_tool_cache(&directory, &engine_cwd, Some(&spec), &required).unwrap_err();
        assert!(error.contains("unpinned"));
        let _ = fs::remove_dir_all(&directory);
    }

    fn morphe_report_fixture() -> &'static str {
        r#"{"packageName":"com.example","testCases":[{"patchName":"p","options":{"durationSeconds": 7,"nested":"caf\u00e9"},"success":true,"errorMessage":null,"appliedPatches":[],"injectedClasses":[],"durationSeconds":1.23},{"patchName":"q","options":{},"success":false,"errorMessage":"bad","appliedPatches":["a"],"injectedClasses":[],"durationSeconds":4.56}],"totalCases":2}"#
    }

    #[test]
    fn morphe_duration_normalization_replaces_only_top_level_values() {
        let original = morphe_report_fixture();
        let (normalized, replacements) = normalize_morphe_durations(original).unwrap();
        assert_eq!(replacements, 2);
        assert_eq!(normalized.matches("\"durationSeconds\":null").count(), 2);
        assert!(normalized.contains("\"durationSeconds\": 7"));
        assert!(normalized.contains("caf\\u00e9"));
        assert!(!normalized.contains(":1.23"));
        assert!(!normalized.contains(":4.56"));
        assert_eq!(normalized.len(), original.len());
    }

    #[test]
    fn morphe_duration_only_mutation_is_accepted() {
        let original = morphe_report_fixture();
        let mutated = original.replace("1.23", "9.99");
        let (normalized, _) = normalize_morphe_durations(original).unwrap();
        let (mutated_normalized, _) = normalize_morphe_durations(&mutated).unwrap();
        assert_eq!(normalized, mutated_normalized);
    }

    #[test]
    fn morphe_duration_normalization_rejects_other_mutations() {
        let original = morphe_report_fixture();
        let (baseline, _) = normalize_morphe_durations(original).unwrap();
        for mutated in [
            original.replace(
                "\"options\":{\"durationSeconds\": 7",
                "\"options\":{\"durationSeconds\": 8",
            ),
            original.replace("\"success\":true", "\"success\":false"),
            original.replace("caf\\u00e9", "caf\\u00e8"),
            original.replace(
                r#",{"patchName":"q","options":{},"success":false,"errorMessage":"bad","appliedPatches":["a"],"injectedClasses":[],"durationSeconds":4.56}"#,
                "",
            ),
        ] {
            let (normalized, _) = normalize_morphe_durations(&mutated).unwrap();
            assert_ne!(baseline, normalized);
        }
    }

    #[test]
    fn morphe_duration_normalization_rejects_missing_null_or_malformed_fields() {
        let original = morphe_report_fixture();
        for malformed in [
            original.replace("\"durationSeconds\":1.23", "\"durationSeconds\":null"),
            original.replace(r#","durationSeconds":4.56"#, ""),
            original
                .replace(r#","durationSeconds":4.56"#, "")
                .replace(r#","durationSeconds":1.23"#, ""),
            "{\"packageName\":\"com.example\"}".to_string(),
            "{\"testCases\":{}}".to_string(),
            "{\"testCases\":[{}]}".to_string(),
        ] {
            assert!(
                normalize_morphe_durations(&malformed).is_err(),
                "must reject {malformed}"
            );
        }
    }

    #[test]
    fn prepare_is_a_bare_flag_on_the_existing_bench_cli() {
        let args: Vec<String> = [
            "--prepare",
            "--engine",
            "python",
            "--profile",
            "local",
            "--manifest",
            "manifest.json",
            "--out",
            "pins.json",
        ]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
        let options = parse_args(&args).unwrap();
        assert!(options.prepare);
        assert_eq!(options.engine, "python");
        assert_eq!(options.profile, "local");

        let error = parse_args(
            &["--prepare=true"]
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap_err();
        assert!(error.contains("does not take a value"));
    }

    #[test]
    fn pin_normalization_is_an_array_even_without_a_rule() {
        assert!(normalization_pins(&Value::Null).is_empty());
        let rule = json!({"rule": "morphe-test-case-duration-seconds", "applied": true});
        assert_eq!(normalization_pins(&rule), vec![rule.clone()]);
    }

    #[test]
    fn pin_validation_errors_redact_repository_and_private_roots() {
        let repo = Path::new("/tmp/example-repo");
        let private = Path::new("/tmp/example-repo/.private");
        let message = "failed /tmp/example-repo/.private/archive and /tmp/example-repo/corpus";
        let sanitised = sanitise_report_message(message, repo, private);
        assert_eq!(sanitised, "failed <private-dir>/archive and <repo>/corpus");
    }

    #[test]
    fn bounded_excerpt_is_utf8_safe_and_reports_truncation() {
        let short = "c0 03 5f d6\n";
        assert_eq!(bounded_excerpt(short), (short.to_string(), false));

        let long = "é".repeat(DIAGNOSTIC_EXCERPT_CHARS + 10);
        let (excerpt, truncated) = bounded_excerpt(&long);
        assert!(truncated);
        assert_eq!(excerpt.chars().count(), DIAGNOSTIC_EXCERPT_CHARS);
        assert_eq!(excerpt, "é".repeat(DIAGNOSTIC_EXCERPT_CHARS));
    }

    #[test]
    fn failed_output_diagnostics_redact_private_roots_before_truncation() {
        let directory = test_directory("diagnostics-redaction");
        let repo = directory.join("repo");
        let engine_cwd = directory.join("engine");
        let sandbox = directory.join("sandbox");
        let workspace = sandbox.join("workspace");
        for path in [&repo, &engine_cwd, &workspace] {
            fs::create_dir_all(path).unwrap();
        }
        let stdout = format!(
            "{}/oracle.py c0 03 5f d6 {}",
            sandbox.display(),
            "x".repeat(DIAGNOSTIC_EXCERPT_CHARS + 100)
        );
        let stderr = format!("{}/corpus.apk missing", repo.display());
        let diagnostics = output_diagnostics(
            &stdout,
            &stderr,
            &repo,
            &engine_cwd,
            &sandbox,
            &workspace,
            None,
        );
        assert!(diagnostics.stdout_truncated);
        assert!(diagnostics
            .stdout_excerpt
            .starts_with("<tmp>/oracle.py c0 03 5f d6"));
        assert!(!diagnostics
            .stdout_excerpt
            .contains(&sandbox.display().to_string()));
        assert!(diagnostics.stdout_excerpt.chars().count() <= DIAGNOSTIC_EXCERPT_CHARS);
        assert!(!diagnostics.stderr_truncated);
        assert_eq!(diagnostics.stderr_excerpt, "<repo>/corpus.apk missing");
        let _ = fs::remove_dir_all(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn prepare_failed_row_keeps_asm_output_but_withholds_pins() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_directory("failed-asm-diagnostics");
        let engine_cwd = directory.join("engine");
        fs::create_dir_all(&engine_cwd).unwrap();
        let engine_exe = directory.join("oracle");
        fs::write(&engine_exe, "#!/bin/sh\nprintf 'c0 03 5f d6\\n'\n").unwrap();
        let mut permissions = fs::metadata(&engine_exe).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&engine_exe, permissions).unwrap();

        let workload: WorkloadSpec = serde_json::from_value(json!({
            "id": "asm.ret.hex",
            "category": "code",
            "size": "small",
            "costly": false,
            "argv": ["asm", "ret", "--format", "hex"],
            "expected": {
                "exitCode": 0,
                "stdoutContains": ["token-that-the-oracle-never-prints"]
            },
            "timeoutSeconds": 30
        }))
        .unwrap();
        let pins = prepare_workload(
            &directory,
            &engine_exe,
            &engine_cwd,
            &[],
            &BTreeMap::new(),
            &workload,
            &detect_metric_capability(),
            true,
            &BTreeSet::new(),
            None,
            &directory.join("private"),
        );
        assert_eq!(pins.status, "failed");
        assert_eq!(pins.exit_code, Some(0));
        assert!(pins.stdout_sha256.is_none());
        assert!(pins.stderr_sha256.is_none());
        assert!(pins.raw_stdout_sha256.is_none());
        assert!(pins.raw_stderr_sha256.is_none());
        assert!(pins
            .validation_errors
            .iter()
            .any(|error| error.contains("stdout")));
        let diagnostics = pins
            .diagnostics
            .expect("a failed row must retain diagnostics");
        assert_eq!(diagnostics.stdout_excerpt, "c0 03 5f d6\n");
        assert!(!diagnostics.stdout_truncated);
        assert!(!diagnostics.stderr_truncated);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn pins_shape_requires_an_array_of_truthful_rows() {
        let mut value = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-output-pins",
            "status": "complete",
            "engine": "python",
            "profile": "local",
            "manifestSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "source": {},
            "workloads": [{
                "id": "startup.help",
                "status": "prepared",
                "exitCode": 0,
                "stdoutSha256": null,
                "stderrSha256": null,
                "rawStdoutSha256": null,
                "rawStderrSha256": null,
                "outputNormalization": [],
                "validationErrors": []
            }],
            "acceptance": {"valid": true, "reasons": []}
        });
        validate_pins_shape(&value).unwrap();

        value["workloads"][0]["outputNormalization"] = json!(null);
        assert!(validate_pins_shape(&value).is_err());

        let mut with_diagnostics = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-output-pins",
            "status": "incomplete",
            "engine": "python",
            "profile": "local",
            "manifestSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "source": {},
            "workloads": [{
                "id": "asm.ret.hex",
                "status": "failed",
                "exitCode": 0,
                "stdoutSha256": null,
                "stderrSha256": null,
                "rawStdoutSha256": null,
                "rawStderrSha256": null,
                "outputNormalization": [],
                "validationErrors": ["stdout does not contain required text"],
                "diagnostics": {
                    "stdoutExcerpt": "c0 03 5f d6\n",
                    "stderrExcerpt": "",
                    "stdoutTruncated": false,
                    "stderrTruncated": false
                }
            }],
            "acceptance": {"valid": false, "reasons": ["failed"]}
        });
        validate_pins_shape(&with_diagnostics).unwrap();
        with_diagnostics["workloads"][0]["status"] = json!("prepared");
        assert!(
            validate_pins_shape(&with_diagnostics).is_err(),
            "diagnostics must not ride on an accepted row"
        );
    }

    fn private_result() -> Value {
        let apk_sha = "a1b2c3d4e5f60718293a4b5c6d7e8f901a2b3c4d5e6f708192a3b4c5d6e7f801";
        let mpp_sha = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let revision = "0123456789abcdef0123456789abcdef01234567";
        json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-result",
            "status": "complete",
            "profile": "local",
            "generatedAtEpochSeconds": 1_760_000_000u64,
            "durationMs": 1234.5,
            "engine": {
                "name": "python",
                "executable": "tools/apk-lab/.venv/bin/python",
                "version": "3.12.7",
                "cwd": "tools/apk-lab",
                "argsPrefix": ["-B", "-m", "apk_lab.cli"],
                "directExecution": true,
                "uvInstallPerformed": false
            },
            "policy": {"id": "local-baseline", "sha256": "b".repeat(64)},
            "source": {
                "revision": revision,
                "dirty": true,
                "worktreeDigest": "c".repeat(64),
                "headIsExecutedSource": false,
                "warning": "HEAD is not the executed source",
                "archive": {
                    "kind": "private-source-archive",
                    "location": "baseline-source/0123456789ab-dddddddddddd",
                    "digest": "d".repeat(64),
                    "fileCount": 42,
                    "reconstruction": "Restore the archived files exactly."
                }
            },
            "environment": {
                "pythonVersion": "3.12.7",
                "hostname": "Example-Host.local",
                "oracleExecutable": "tools/apk-lab/.venv/bin/python",
                "storage": {
                    "supported": true,
                    "method": "df -Pk (1024-byte blocks)",
                    "mountPoint": "/System/Volumes/Data",
                    "filesystem": "/dev/disk3s5",
                    "totalBytes": 1000,
                    "freeBytes": 500
                }
            },
            "settings": {"order": "fixed-manifest-order (single engine)"},
            "measurements": {
                "wall": {"supported": true, "unit": "milliseconds", "method": "Instant"},
                "cpu": {"supported": true, "unit": "milliseconds", "method": "ps"},
                "descendantRss": {"supported": true, "unit": "bytes", "method": "ps"},
                "io": {"supported": false, "unit": null, "method": null, "reason": "unsupported"}
            },
            "workloads": [{
                "id": "split.sample",
                "category": "splits",
                "size": "large",
                "costly": false,
                "status": "measured",
                "sampleClass": "local",
                "warmupsExpected": 5,
                "samplesExpected": 30,
                "warmupsCompleted": 5,
                "samplesCompleted": 30,
                "reasons": [],
                "inputs": [{
                    "role": "apk",
                    "displayPath": "corpus/team-alpha/com.example.apk",
                    "declaredSha256": apk_sha,
                    "actualSha256": apk_sha,
                    "sizeBytes": 123,
                    "available": true,
                    "reason": null
                }],
                "mpp": {
                    "role": "mpp",
                    "displayPath": "corpus/team-alpha/com.example.mpp",
                    "declaredSha256": mpp_sha,
                    "actualSha256": mpp_sha,
                    "sizeBytes": 456,
                    "available": true,
                    "reason": null
                },
                "requiredTools": [],
                "argvTemplate": ["asm", "ret", "--format", "hex", "--limit=10"],
                "environment": {},
                "samples": [{
                    "kind": "warmup",
                    "cacheState": "warmup",
                    "index": 0,
                    "wallMs": 10.5,
                    "cpuMs": 9.0,
                    "peakChildRssBytes": 1024,
                    "readBytes": null,
                    "writeBytes": null,
                    "exitCode": 0,
                    "timedOut": false,
                    "stdoutSha256": "e".repeat(64),
                    "stderrSha256": "f".repeat(64),
                    "rawStdoutSha256": "e".repeat(64),
                    "rawStderrSha256": "f".repeat(64),
                    "stdoutBytes": 12,
                    "stderrBytes": 0,
                    "outputValid": true,
                    "outputsPinned": true,
                    "outputNormalization": null,
                    "validationErrors": []
                }, {
                    "kind": "measured",
                    "cacheState": "warm-file-cache",
                    "index": 1,
                    "wallMs": 8.25,
                    "cpuMs": 7.5,
                    "peakChildRssBytes": 2048,
                    "readBytes": null,
                    "writeBytes": null,
                    "exitCode": 0,
                    "timedOut": false,
                    "stdoutSha256": "e".repeat(64),
                    "stderrSha256": "f".repeat(64),
                    "rawStdoutSha256": "e".repeat(64),
                    "rawStderrSha256": "f".repeat(64),
                    "stdoutBytes": 12,
                    "stderrBytes": 0,
                    "outputValid": true,
                    "outputsPinned": true,
                    "outputNormalization": null,
                    "validationErrors": []
                }],
                "summary": {"count": 30, "minMs": 8.0, "medianMs": 8.25, "meanMs": 8.3, "p95Ms": 9.0, "maxMs": 10.0},
                "outputValidation": {"stdoutHashPinned": true, "stderrHashPinned": false, "allOutputsValid": true},
                "outputNormalization": null
            }],
            "acceptance": {"valid": true, "reasons": []},
            "provenance": {"schemaVersion": 1, "policySha256": "b".repeat(64)}
        })
    }

    #[test]
    fn sanitize_private_result_keeps_measurements_and_replaces_paths() {
        let raw = private_result();
        let sanitized = sanitize_result_value(&raw).expect("private result must sanitize");
        let apk_sha = &raw["workloads"][0]["inputs"][0]["declaredSha256"]
            .as_str()
            .unwrap()[..12];
        let mpp_sha = &raw["workloads"][0]["mpp"]["declaredSha256"]
            .as_str()
            .unwrap()[..12];

        assert_eq!(sanitized["engine"]["executable"], json!("python"));
        assert_eq!(sanitized["engine"]["cwd"], json!("<repo>"));
        assert_eq!(
            sanitized["engine"]["argsPrefix"], raw["engine"]["argsPrefix"],
            "argsPrefix carries no private path and must be preserved"
        );
        assert_eq!(
            sanitized["source"]["archive"]["location"],
            json!("<private-source-archive>")
        );
        assert_eq!(
            sanitized["source"]["archive"]["reconstruction"],
            json!("<private-reconstruction-recipe>")
        );
        assert_eq!(
            sanitized["source"]["archive"]["digest"],
            raw["source"]["archive"]["digest"]
        );
        assert_eq!(sanitized["source"]["archive"]["fileCount"], json!(42));
        assert_eq!(sanitized["source"]["revision"], raw["source"]["revision"]);
        assert_eq!(sanitized["source"]["dirty"], json!(true));
        assert_eq!(
            sanitized["source"]["worktreeDigest"],
            raw["source"]["worktreeDigest"]
        );

        let workload = &sanitized["workloads"][0];
        assert_eq!(
            workload["inputs"][0]["displayPath"],
            json!(format!("<corpus>/{apk_sha}"))
        );
        assert_eq!(
            workload["mpp"]["displayPath"],
            json!(format!("<corpus>/{mpp_sha}"))
        );
        assert_eq!(
            workload["inputs"][0]["declaredSha256"],
            raw["workloads"][0]["inputs"][0]["declaredSha256"]
        );
        assert_eq!(
            workload["inputs"][0]["role"],
            raw["workloads"][0]["inputs"][0]["role"]
        );
        assert!(
            workload.get("acceptance").is_none(),
            "workloads do not carry acceptance"
        );
        assert!(workload.get("argvTemplate").is_none());
        assert_eq!(
            workload["command"],
            json!({
                "subcommand": "asm",
                "flags": ["--format", "--limit=10"],
                "flagNames": ["--format", "--limit"],
            })
        );
        assert_eq!(workload["samples"], raw["workloads"][0]["samples"]);
        assert_eq!(workload["samples"][1]["wallMs"], json!(8.25));
        assert_eq!(workload["id"], json!("split.sample"));
        assert_eq!(workload["category"], json!("splits"));
        assert_eq!(workload["size"], json!("large"));
        assert_eq!(workload["status"], json!("measured"));
        assert_eq!(workload["summary"], raw["workloads"][0]["summary"]);
        assert_eq!(sanitized["acceptance"], raw["acceptance"]);
        assert_eq!(sanitized["measurements"], raw["measurements"]);
        assert_eq!(sanitized["environment"]["pythonVersion"], json!("3.12.7"));
        assert_eq!(
            sanitized["environment"]["oracleExecutable"],
            json!("python")
        );
        assert_eq!(
            sanitized["environment"]["storage"]["mountPoint"],
            json!("<storage-mount>")
        );
        assert_eq!(
            sanitized["environment"]["storage"]["filesystem"],
            json!("<storage-device>")
        );
        assert_eq!(
            sanitized["environment"]["storage"]["totalBytes"],
            raw["environment"]["storage"]["totalBytes"]
        );
        let host = sanitized["environment"]["hostname"]
            .as_str()
            .expect("sanitized hostname must be a string");
        assert!(
            host.starts_with("<host:")
                && host.ends_with('>')
                && host.len() == "<host:".len() + 12 + 1,
            "host identity must be a stable placeholder, got {host:?}"
        );
        assert_ne!(
            sanitized["environment"]["hostname"],
            raw["environment"]["hostname"]
        );
        assert_eq!(sanitized["settings"], raw["settings"]);
        assert_eq!(sanitized["policy"], raw["policy"]);
        assert_eq!(sanitized["provenance"], raw["provenance"]);
    }

    #[test]
    fn sanitize_rejects_leaked_absolute_path_in_unrelated_string() {
        let mut raw = private_result();
        raw["acceptance"]["reasons"] =
            json!(["could not read /Users/aidan/Projects/aidans-patches/corpus/team-alpha.apk"]);
        raw["acceptance"]["valid"] = json!(false);
        raw["status"] = json!("incomplete");
        let error = sanitize_result_value(&raw).unwrap_err();
        assert!(
            error.contains("private path marker"),
            "absolute host path must be rejected, got: {error}"
        );
    }

    #[test]
    fn sanitize_rejects_declared_input_path_echoed_elsewhere() {
        let mut raw = private_result();
        raw["workloads"][0]["reasons"] =
            json!(["failed while reading corpus/team-alpha/com.example.apk"]);
        let error = sanitize_result_value(&raw).unwrap_err();
        assert!(
            error.contains("private path marker 'corpus/team-alpha/com.example.apk'"),
            "declared private path must be rejected, got: {error}"
        );
    }

    #[test]
    fn sanitize_rejects_private_migration_and_apk_paths() {
        for leak in [".apk-lab-migration/archive.zip", "/srv/apk/private.apk"] {
            let mut raw = private_result();
            raw["provenance"]
                .as_object_mut()
                .unwrap()
                .insert("note".to_string(), json!(leak));
            let error = sanitize_result_value(&raw).unwrap_err();
            assert!(
                error.contains("private path marker"),
                "must reject {leak}, got: {error}"
            );
        }
    }

    #[test]
    fn sanitize_rejects_missing_or_empty_workloads() {
        let mut missing = private_result();
        let _ = missing.as_object_mut().unwrap().remove("workloads");
        let error = sanitize_result_value(&missing).unwrap_err();
        assert!(error.contains("workloads"), "got: {error}");

        let mut empty = private_result();
        empty["workloads"] = json!([]);
        let error = sanitize_result_value(&empty).unwrap_err();
        assert!(
            error.contains("workloads must not be empty"),
            "got: {error}"
        );
    }

    #[test]
    fn sanitize_rejects_missing_private_fields() {
        let mut without_engine = private_result();
        let _ = without_engine.as_object_mut().unwrap().remove("engine");
        assert!(sanitize_result_value(&without_engine).is_err());

        let mut without_declared_hash = private_result();
        let _ = without_declared_hash["workloads"][0]["inputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("declaredSha256");
        assert!(sanitize_result_value(&without_declared_hash).is_err());

        let mut without_template = private_result();
        let _ = without_template["workloads"][0]
            .as_object_mut()
            .unwrap()
            .remove("argvTemplate");
        let error = sanitize_result_value(&without_template).unwrap_err();
        assert!(
            error.contains("neither argvTemplate nor command"),
            "got: {error}"
        );
    }

    #[test]
    fn sanitize_writes_nothing_on_rejection() {
        let directory = test_directory("sanitize-rejected");
        let input = directory.join("raw.json");
        let output = directory.join("public.json");
        let mut broken = private_result();
        broken["workloads"] = json!([]);
        fs::write(&input, serde_json::to_vec_pretty(&broken).unwrap()).unwrap();
        let error = sanitize_result(&input, &output).unwrap_err();
        assert!(error.contains("workloads"), "got: {error}");
        assert!(!output.exists(), "a rejected input must not write output");
        assert!(
            !directory.join("public.json.tmp").exists(),
            "a rejected input must not leave a temp file"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn sanitize_writes_public_result_atomically() {
        let directory = test_directory("sanitize-accepted");
        let input = directory.join("raw.json");
        let output = directory.join("public.json");
        fs::write(
            &input,
            serde_json::to_vec_pretty(&private_result()).unwrap(),
        )
        .unwrap();
        sanitize_result(&input, &output).expect("valid private result must sanitize");
        let text = fs::read_to_string(&output).unwrap();
        assert!(!text.contains("corpus/team-alpha"));
        assert!(!text.contains("tools/apk-lab"));
        assert!(!text.contains("baseline-source/"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["engine"]["executable"], json!("python"));
        assert_eq!(value["workloads"][0]["command"]["subcommand"], json!("asm"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn sanitize_cli_needs_only_input_and_out() {
        let args: Vec<String> = ["--sanitize", "--input", "raw.json", "--out", "public.json"]
            .iter()
            .map(|arg| arg.to_string())
            .collect();
        let options = parse_args(&args).unwrap();
        assert!(options.sanitize);
        assert_eq!(options.input.as_deref(), Some(Path::new("raw.json")));
        assert_eq!(options.out, PathBuf::from("public.json"));

        let error = parse_args(
            &[
                "--sanitize",
                "--input",
                "raw.json",
                "--out",
                "public.json",
                "--manifest",
                "m.json",
            ]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>(),
        )
        .unwrap_err();
        assert!(error.contains("cannot be combined"), "got: {error}");

        let error = parse_args(
            &["--sanitize", "--out", "public.json"]
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap_err();
        assert!(error.contains("requires --input"), "got: {error}");
    }

    #[test]
    fn paired_cli_accepts_engine_both_and_rust_bin() {
        let args: Vec<String> = [
            "--engine",
            "both",
            "--profile",
            "local",
            "--manifest",
            "manifest.json",
            "--rust-bin",
            "tools/apk-lab/target/release/apk-lab",
            "--out",
            "paired.json",
        ]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
        let options = parse_args(&args).unwrap();
        assert_eq!(options.engine, "both");
        assert_eq!(
            options.rust_bin.as_deref(),
            Some(Path::new("tools/apk-lab/target/release/apk-lab"))
        );

        let error = parse_args(
            &[
                "--prepare",
                "--engine",
                "both",
                "--profile",
                "local",
                "--manifest",
                "manifest.json",
                "--out",
                "paired.json",
            ]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(error.prepare);
        assert!(run(&[
            "--prepare".to_string(),
            "--engine".to_string(),
            "both".to_string(),
            "--profile".to_string(),
            "local".to_string(),
            "--manifest".to_string(),
            "missing.json".to_string(),
            "--out".to_string(),
            "out.json".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn paired_order_alternates_from_a_seeded_start() {
        let mut rng = SplitMix64::new(order_seed(20261120, "inspect.small"));
        let rust_first = rng.next_u64() & 1 == 1;
        let first = pair_order(rust_first, 0);
        let second = pair_order(rust_first, 1);
        let third = pair_order(rust_first, 2);
        assert_eq!(first, third);
        assert_ne!(first, second);
        if rust_first {
            assert_eq!(first, vec!["rust", "python"]);
        } else {
            assert_eq!(first, vec!["python", "rust"]);
        }
        // A different workload gets an independent seeded start.
        let other = order_seed(20261120, "inspect.large");
        assert_ne!(order_seed(20261120, "inspect.small"), other);
    }

    #[test]
    fn paired_result_shape_accepts_only_both_engines() {
        let workload = json!({
            "id": "startup.help",
            "category": "startup",
            "size": "small",
            "costly": false,
            "sampleClass": "local",
            "status": "measured-paired",
            "warmupsExpected": 5,
            "samplesExpected": 30,
            "measuredPairs": 0,
            "reasons": [],
            "inputs": [],
            "mpp": null,
            "requiredTools": [],
            "argvTemplate": ["--help"],
            "environment": {},
            "engines": {
                "python": {"samples": []},
                "rust": {"samples": []}
            },
            "pairs": [],
            "correctness": {"matched": false}
        });
        let mut value = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-paired-benchmark-result",
            "status": "incomplete",
            "profile": "local",
            "engines": {
                "python": {"directExecution": true},
                "rust": {"directExecution": true, "compilationIncluded": false}
            },
            "corpus": {"identitySha256": "c".repeat(64)},
            "workloads": [workload],
            "acceptance": {"valid": false, "reasons": ["not measured"]}
        });
        validate_paired_result_shape(&value).unwrap();
        value["workloads"][0]["engines"]
            .as_object_mut()
            .unwrap()
            .remove("rust");
        assert!(validate_paired_result_shape(&value).is_err());
    }

    fn test_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "apk-lab-bench-test-{}-{}-{}",
            name,
            std::process::id(),
            unique_counter()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }
}
