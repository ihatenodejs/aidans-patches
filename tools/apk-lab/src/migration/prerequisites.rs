//! Prerequisite inventory and acceptance checks for the APK Lab Rust migration.
//!
//! This module deliberately performs only non-destructive local probes:
//! process versions, filesystem paths, SHA-256 identities, and archive content
//! classification through the production native archive inspector. It never talks
//! to Google Play, Cloudflare, a device, or any other network service. Device,
//! account, cloud, and CI resources require separate pinned authorization and
//! availability evidence records instead of an asserted boolean; authorization
//! never implies that the resource exists.
//!
//! Public JSON reports are sanitized: no private paths, device serials,
//! accounts, bucket names, or evidence paths are serialized.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: u32 = 1;
const MANIFEST_KIND: &str = "apk-lab-prerequisites-manifest";
const REPORT_KIND: &str = "apk-lab-prerequisites-report";
const READ_CHUNK: usize = 64 * 1024;

/// One row of the fixed prerequisite catalog.
struct CatalogEntry {
    id: &'static str,
    kind: &'static str,
    owner: &'static str,
    consuming_plan: &'static str,
    description: &'static str,
    probes_required: bool,
    authorization_required: bool,
}

/// Required resources. Ordering is stable and is also the report ordering.
const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "host.jdk",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "JDK 21+ with java and keytool",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.android-sdk",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "Android SDK with Build Tools 36.0.0 and platform-tools",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.android-ndk",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "Pinned Android NDK revision",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.cmake",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "CMake for native build verification",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.ninja",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "Ninja for native build verification",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.bun",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "Bun used by root repository orchestration",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "host.rust",
        kind: "host-tool",
        owner: "toolchain-operator",
        consuming_plan: "002",
        description: "Stable Rust toolchain with rustc and cargo",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "corpus.single-apk",
        kind: "artifact-corpus",
        owner: "benchmark-owner",
        consuming_plan: "001,002",
        description: "Hash-pinned private single signed APK for inspection/archive coverage; not a pristine Morphe baseline",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "corpus.split-apk",
        kind: "artifact-corpus",
        owner: "benchmark-owner",
        consuming_plan: "001,002",
        description: "Hash-pinned private split APK/APKM input for normalization coverage",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "corpus.mpp",
        kind: "artifact-corpus",
        owner: "benchmark-owner",
        consuming_plan: "001,002",
        description: "Hash-pinned Morphe patch bundle used by compatibility workloads",
        probes_required: true,
        authorization_required: false,
    },
    CatalogEntry {
        id: "device.disposable",
        kind: "device-authorization",
        owner: "device-operator",
        consuming_plan: "004",
        description:
            "Authorized disposable Android device/emulator with install and harmless UI-click scope",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.latest",
        kind: "play-delivery",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Authorized Play account/package for latest-version acquisition",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.older",
        kind: "play-delivery",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Authorized Play retrieval of an older version",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.direct",
        kind: "play-delivery",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Authorized direct authenticated Play delivery path",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.dispenser",
        kind: "play-delivery",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Authorized public Aurora device-dispenser fallback path",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.oauth",
        kind: "play-oauth",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Authorized OAuth token-exchange flow for disposable test accounts",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "play.hosts",
        kind: "play-network-hosts",
        owner: "release-operator",
        consuming_plan: "010",
        description: "Approved Play endpoint and host-mapping evidence",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "cloud.r2",
        kind: "cloud-storage",
        owner: "infra-operator",
        consuming_plan: "005",
        description: "Isolated R2 test bucket/prefix with fixture read/write scope",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "cloud.worker-callback",
        kind: "worker-endpoint",
        owner: "infra-operator",
        consuming_plan: "005",
        description: "Isolated test Worker callback endpoint and status secret scope",
        probes_required: false,
        authorization_required: true,
    },
    CatalogEntry {
        id: "ci.compatibility-workflow",
        kind: "ci-workflow",
        owner: "repo-admin",
        consuming_plan: "005",
        description: "Authorized test compatibility workflow and repository dispatch permissions",
        probes_required: false,
        authorization_required: true,
    },
];

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    kind: String,
    generated_at: String,
    host: HostInfo,
    resources: Vec<ResourceSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HostInfo {
    profile: String,
    os: String,
    arch: String,
    cpu_count: u32,
    memory_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ResourceSpec {
    id: String,
    kind: String,
    owner: String,
    #[serde(default)]
    probes: Vec<ProbeSpec>,
    #[serde(default)]
    authorization: Option<AuthorizationSpec>,
    #[serde(default)]
    availability: Option<EvidencePin>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProbeSpec {
    label: String,
    #[serde(rename = "type")]
    probe_type: String,
    #[serde(default)]
    program: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    expected_output_contains: Option<String>,
    #[serde(default)]
    expected_version: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    expected_contains: Option<String>,
    #[serde(default)]
    container_type: Option<String>,
    #[serde(default)]
    oracle: Option<OracleSpec>,
}

/// Historical oracle identity retained for manifest schema compatibility.
/// Container classification is native; these fields are validated but never
/// executed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OracleSpec {
    program: String,
    sha256: String,
    module_root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuthorizationSpec {
    required: bool,
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    identity: Option<String>,
    #[serde(default)]
    evidence: Option<EvidencePin>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EvidencePin {
    path: String,
    sha256: String,
}

/// Structured authorization evidence. A bare `authorized: true` is not
/// expressible here; the record must name the resource, identity, decision,
/// scope, and validity window.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuthorizationEvidence {
    schema_version: u32,
    resource_id: String,
    decision: String,
    identity: String,
    issued_at: String,
    expires_at: String,
    scope: String,
}

/// Structured, independently pinned availability observation. Authorization
/// never implies availability: the record must name a safe probe/scenario,
/// its outcome, the observed identity, and a bounded validity window.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AvailabilityEvidence {
    schema_version: u32,
    resource_id: String,
    identity: String,
    probe: String,
    outcome: String,
    observed_at: String,
    expires_at: String,
    scenario: String,
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    kind: &'static str,
    generated_at: String,
    mode: &'static str,
    host: ReportHost,
    summary: Summary,
    resources: Vec<ResourceReport>,
    blockers: Vec<Blocker>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportHost {
    profile: String,
    os: String,
    arch: String,
    cpu_count: u32,
    memory_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    total: usize,
    ready: usize,
    blocked: usize,
    missing: usize,
    acceptance_passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceReport {
    id: String,
    kind: String,
    owner: String,
    consuming_plan: String,
    description: String,
    required: bool,
    status: &'static str,
    verification: &'static str,
    probes: Vec<ProbeResult>,
    availability: AvailabilityReport,
    authorization: AuthorizationReport,
    blockers: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeResult {
    label: String,
    #[serde(rename = "type")]
    probe_type: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    container_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AvailabilityReport {
    source: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationReport {
    required: bool,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Blocker {
    resource_id: String,
    owner: String,
    reason: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Inventory,
    Check,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Inventory => "inventory",
            Mode::Check => "check",
        }
    }
}

struct Options {
    mode: Mode,
    manifest: PathBuf,
    out: Option<PathBuf>,
}

enum Invocation {
    Help,
    Run(Options),
}

/// `args` are the CLI arguments after the `prerequisites` subcommand.
pub fn run(args: &[String]) -> Result<(), String> {
    let invocation = parse_args(args)?;
    let options = match invocation {
        Invocation::Help => {
            print_help();
            return Ok(());
        }
        Invocation::Run(options) => options,
    };

    let manifest = load_manifest(&options.manifest)?;
    validate_manifest(&manifest)?;
    let now = unix_now()?;
    let report = evaluate(&manifest, options.mode, now)?;
    write_report(&report, options.out.as_deref())?;
    print_summary(&report);

    if options.mode == Mode::Check && !report.summary.acceptance_passed {
        return Err(format!(
            "prerequisites check failed: {} of {} required resources are not ready",
            report.summary.total - report.summary.ready,
            report.summary.total
        ));
    }
    Ok(())
}

fn print_help() {
    println!(
        "Usage: apk-lab-migrate prerequisites [--check] --manifest PATH [--out PATH|-]\n\
         \n\
         Non-destructively probes host tools, file hashes, and pinned authorization\n\
         evidence. It never contacts a device, Play, Cloudflare, or CI service.\n\
         \n\
         --check             acceptance mode: exit nonzero unless every required\n\
                             resource is available/authorized and current\n\
         --manifest PATH     private prerequisite manifest\n\
         --out PATH|-        write the sanitized report (default: stdout)\n\
         --report PATH|-     alias for --out"
    );
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut mode = Mode::Inventory;
    let mut manifest: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "--check" => mode = Mode::Check,
            "--manifest" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--manifest requires a path".to_string())?;
                set_once(&mut manifest, value, "--manifest")?;
            }
            "--out" | "--report" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{arg} requires a path or '-'"))?;
                set_once(&mut out, value, arg)?;
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
                set_once(&mut out, value, "--report")?;
            }
            _ => return Err(format!("unexpected argument: {arg}")),
        }
        index += 1;
    }

    let manifest = manifest.ok_or_else(|| "missing required --manifest PATH".to_string())?;
    Ok(Invocation::Run(Options {
        mode,
        manifest,
        out,
    }))
}

fn set_once<T: From<String>>(slot: &mut Option<T>, value: &str, flag: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{flag} requires a non-empty value"));
    }
    if slot.is_some() {
        return Err(format!("{flag} specified more than once"));
    }
    *slot = Some(T::from(value.to_string()));
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest loading and validation
// ---------------------------------------------------------------------------

fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let content =
        fs::read_to_string(path).map_err(|_| "cannot read prerequisite manifest".to_string())?;
    serde_json::from_str(&content)
        .map_err(|error| format!("invalid prerequisite manifest: {error}"))
}

fn catalog_entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.id == id)
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported prerequisite manifest schemaVersion {}",
            manifest.schema_version
        ));
    }
    if manifest.kind != MANIFEST_KIND {
        return Err(format!(
            "prerequisite manifest kind must be {MANIFEST_KIND}"
        ));
    }
    if parse_utc(&manifest.generated_at).is_none() {
        return Err("generatedAt must be a UTC RFC 3339 timestamp".to_string());
    }
    validate_token("host.profile", &manifest.host.profile, 64)?;
    if manifest.host.os != env::consts::OS {
        return Err(format!(
            "manifest host.os '{}' does not match this host '{}'",
            manifest.host.os,
            env::consts::OS
        ));
    }
    if manifest.host.arch != env::consts::ARCH {
        return Err(format!(
            "manifest host.arch '{}' does not match this host '{}'",
            manifest.host.arch,
            env::consts::ARCH
        ));
    }
    if manifest.host.cpu_count == 0 {
        return Err("host.cpuCount must be greater than zero".to_string());
    }
    if manifest.host.memory_bytes == 0 {
        return Err("host.memoryBytes must be greater than zero".to_string());
    }

    let mut seen = BTreeSet::new();
    for resource in &manifest.resources {
        let catalog = catalog_entry(&resource.id)
            .ok_or_else(|| format!("unknown resource id '{}'", resource.id))?;
        if !seen.insert(resource.id.as_str()) {
            return Err(format!("duplicate resource id '{}'", resource.id));
        }
        if resource.kind != catalog.kind {
            return Err(format!(
                "resource '{}' has kind '{}', expected '{}'",
                resource.id, resource.kind, catalog.kind
            ));
        }
        validate_token("resource owner", &resource.owner, 64)?;

        if !catalog.probes_required {
            if !resource.probes.is_empty() {
                return Err(format!(
                    "resource '{}' must not declare local probes; its availability is evidence-gated",
                    resource.id
                ));
            }
            if !catalog.authorization_required {
                return Err(format!(
                    "resource '{}' has no probe and no authorization requirement",
                    resource.id
                ));
            }
        }
        if catalog.probes_required && resource.probes.is_empty() {
            return Err(format!(
                "resource '{}' must declare at least one non-destructive probe",
                resource.id
            ));
        }

        let mut labels = BTreeSet::new();
        for probe in &resource.probes {
            if !labels.insert(probe.label.as_str()) {
                return Err(format!(
                    "resource '{}' has duplicate probe label '{}'",
                    resource.id, probe.label
                ));
            }
            validate_probe(&resource.id, probe)?;
        }
        validate_corpus_container_identity(&resource.id, &resource.probes)?;

        if catalog.authorization_required {
            let pin = resource.availability.as_ref().ok_or_else(|| {
                format!(
                    "resource '{}' requires independently pinned availability evidence",
                    resource.id
                )
            })?;
            if pin.path.trim().is_empty() {
                return Err(format!(
                    "resource '{}' availability evidence path is empty",
                    resource.id
                ));
            }
            validate_sha256("availability evidence sha256", &pin.sha256)?;
        } else if resource.availability.is_some() {
            return Err(format!(
                "resource '{}' is probe-verified and must not pin availability evidence",
                resource.id
            ));
        }

        match (&resource.authorization, catalog.authorization_required) {
            (None, true) => {
                return Err(format!(
                    "resource '{}' requires pinned authorization evidence",
                    resource.id
                ));
            }
            (Some(auth), true) => validate_authorization(&resource.id, auth)?,
            (Some(auth), false) => {
                if auth.required {
                    return Err(format!(
                        "resource '{}' is probe-verified and must not require live authorization",
                        resource.id
                    ));
                }
                if auth.evidence.is_some() {
                    return Err(format!(
                        "resource '{}' must not pin authorization evidence",
                        resource.id
                    ));
                }
            }
            (None, false) => {}
        }
    }
    Ok(())
}

fn validate_probe(resource_id: &str, probe: &ProbeSpec) -> Result<(), String> {
    validate_token("probe label", &probe.label, 64)?;
    match probe.probe_type.as_str() {
        "executable" => {
            let program = probe.program.as_deref().ok_or_else(|| {
                format!("resource '{resource_id}' executable probe needs program")
            })?;
            if program.trim().is_empty() {
                return Err(format!("resource '{resource_id}' has an empty program"));
            }
            if probe.path.is_some()
                || probe.size_bytes.is_some()
                || probe.expected_contains.is_some()
                || probe.container_type.is_some()
                || probe.oracle.is_some()
            {
                return Err(format!(
                    "resource '{resource_id}' executable probe '{}' mixes file fields",
                    probe.label
                ));
            }
            if probe.args.len() > 32 {
                return Err(format!(
                    "resource '{resource_id}' executable probe '{}' has too many args",
                    probe.label
                ));
            }
            if probe.expected_output_contains.is_none() && probe.expected_version.is_none() {
                return Err(format!(
                    "resource '{resource_id}' executable probe '{}' needs an expected marker",
                    probe.label
                ));
            }
            if let Some(hash) = &probe.sha256 {
                validate_sha256("probe sha256", hash)?;
            }
        }
        "file" => {
            let path = probe
                .path
                .as_deref()
                .ok_or_else(|| format!("resource '{resource_id}' file probe needs path"))?;
            if path.trim().is_empty() {
                return Err(format!("resource '{resource_id}' has an empty file path"));
            }
            if probe.program.is_some()
                || !probe.args.is_empty()
                || probe.expected_output_contains.is_some()
                || probe.container_type.is_some()
                || probe.oracle.is_some()
            {
                return Err(format!(
                    "resource '{resource_id}' file probe '{}' mixes executable fields",
                    probe.label
                ));
            }
            let hash = probe
                .sha256
                .as_deref()
                .ok_or_else(|| format!("resource '{resource_id}' file probe needs sha256"))?;
            validate_sha256("probe sha256", hash)?;
        }
        "apk-container" => {
            let path = probe.path.as_deref().ok_or_else(|| {
                format!("resource '{resource_id}' apk-container probe needs path")
            })?;
            if path.trim().is_empty() {
                return Err(format!(
                    "resource '{resource_id}' apk-container probe has an empty path"
                ));
            }
            if probe.program.is_some()
                || !probe.args.is_empty()
                || probe.expected_output_contains.is_some()
                || probe.expected_contains.is_some()
                || probe.expected_version.is_some()
            {
                return Err(format!(
                    "resource '{resource_id}' apk-container probe '{}' mixes other probe fields",
                    probe.label
                ));
            }
            let hash = probe.sha256.as_deref().ok_or_else(|| {
                format!("resource '{resource_id}' apk-container probe needs sha256")
            })?;
            validate_sha256("probe sha256", hash)?;

            let expected = probe.container_type.as_deref().ok_or_else(|| {
                format!("resource '{resource_id}' apk-container probe needs containerType")
            })?;
            if !is_container_type(expected) {
                return Err(format!(
                    "resource '{resource_id}' apk-container probe has unsupported containerType"
                ));
            }

            let oracle = probe.oracle.as_ref().ok_or_else(|| {
                format!("resource '{resource_id}' apk-container probe needs an oracle")
            })?;
            if oracle.program.trim().is_empty() {
                return Err(format!(
                    "resource '{resource_id}' apk-container probe oracle program is empty"
                ));
            }
            if oracle.module_root.trim().is_empty() {
                return Err(format!(
                    "resource '{resource_id}' apk-container probe oracle module root is empty"
                ));
            }
            validate_sha256("oracle sha256", &oracle.sha256)?;
        }
        other => {
            return Err(format!(
                "resource '{resource_id}' has unsupported probe type '{other}'"
            ));
        }
    }
    Ok(())
}

fn validate_authorization(resource_id: &str, auth: &AuthorizationSpec) -> Result<(), String> {
    if !auth.required {
        return Err(format!(
            "resource '{resource_id}' must mark authorization required"
        ));
    }
    let reference = auth
        .reference
        .as_deref()
        .ok_or_else(|| format!("resource '{resource_id}' authorization needs a reference"))?;
    if reference.is_empty() || reference.contains('@') || reference.len() > 128 {
        return Err(format!(
            "resource '{resource_id}' authorization reference must be a non-account opaque identifier"
        ));
    }
    if !reference
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
    {
        return Err(format!(
            "resource '{resource_id}' authorization reference has unsupported characters"
        ));
    }
    let identity = auth
        .identity
        .as_deref()
        .ok_or_else(|| format!("resource '{resource_id}' authorization needs an identity alias"))?;
    if identity.trim().is_empty() || identity.len() > 256 {
        return Err(format!(
            "resource '{resource_id}' authorization identity is invalid"
        ));
    }
    let evidence = auth.evidence.as_ref().ok_or_else(|| {
        format!("resource '{resource_id}' asserts authorization without pinned evidence")
    })?;
    if evidence.path.trim().is_empty() {
        return Err(format!(
            "resource '{resource_id}' authorization evidence path is empty"
        ));
    }
    validate_sha256("authorization evidence sha256", &evidence.sha256)?;
    Ok(())
}

fn validate_corpus_container_identity(
    resource_id: &str,
    probes: &[ProbeSpec],
) -> Result<(), String> {
    if resource_id != "corpus.single-apk" && resource_id != "corpus.split-apk" {
        return Ok(());
    }
    let identities: Vec<&ProbeSpec> = probes
        .iter()
        .filter(|probe| probe.probe_type == "apk-container")
        .collect();
    if identities.len() != 1 {
        return Err(format!(
            "resource '{resource_id}' must declare exactly one apk-container identity probe"
        ));
    }
    let expected = identities[0].container_type.as_deref().unwrap_or("");
    match resource_id {
        "corpus.single-apk" if expected != "APK" => Err(format!(
            "resource '{resource_id}' must pin containerType APK; extension alone is not content identity"
        )),
        "corpus.split-apk" if expected == "APK" => Err(format!(
            "resource '{resource_id}' must pin a split container type, not APK"
        )),
        _ => Ok(()),
    }
}

fn is_container_type(value: &str) -> bool {
    matches!(value, "APK" | "APKM" | "XAPK" | "APKS")
}

fn validate_token(field: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max_len {
        return Err(format!("{field} must be 1..={max_len} characters"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
    {
        return Err(format!("{field} contains unsupported characters"));
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{field} must be 64 hexadecimal characters"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

fn evaluate(manifest: &Manifest, mode: Mode, now: i64) -> Result<Report, String> {
    let mut resources = Vec::with_capacity(CATALOG.len());
    let mut blockers = Vec::new();
    let mut ready = 0usize;
    let mut missing = 0usize;

    for catalog in CATALOG {
        let spec = manifest
            .resources
            .iter()
            .find(|resource| resource.id == catalog.id);

        let report = match spec {
            Some(spec) => evaluate_resource(catalog, spec, now),
            None => missing_resource(catalog),
        };

        if report.status == "ready" {
            ready += 1;
        } else if report.status == "missing" {
            missing += 1;
        }
        for reason in &report.blockers {
            blockers.push(Blocker {
                resource_id: report.id.clone(),
                owner: report.owner.clone(),
                reason: reason.clone(),
            });
        }
        resources.push(report);
    }

    let total = CATALOG.len();
    let report = Report {
        schema_version: SCHEMA_VERSION,
        kind: REPORT_KIND,
        generated_at: format_utc(now),
        mode: mode.as_str(),
        host: ReportHost {
            profile: manifest.host.profile.clone(),
            os: manifest.host.os.clone(),
            arch: manifest.host.arch.clone(),
            cpu_count: manifest.host.cpu_count,
            memory_bytes: manifest.host.memory_bytes,
        },
        summary: Summary {
            total,
            ready,
            blocked: total - ready - missing,
            missing,
            acceptance_passed: ready == total,
        },
        resources,
        blockers,
    };
    Ok(report)
}

fn missing_resource(catalog: &CatalogEntry) -> ResourceReport {
    ResourceReport {
        id: catalog.id.to_string(),
        kind: catalog.kind.to_string(),
        owner: catalog.owner.to_string(),
        consuming_plan: catalog.consuming_plan.to_string(),
        description: catalog.description.to_string(),
        required: true,
        status: "missing",
        verification: "missing",
        probes: Vec::new(),
        availability: AvailabilityReport {
            source: "missing",
            status: "missing",
            evidence_sha256: None,
            observed_at: None,
            expires_at: None,
            reason: Some("required resource row missing from manifest".to_string()),
        },
        authorization: AuthorizationReport {
            required: catalog.authorization_required,
            status: "missing",
            reference: None,
            evidence_sha256: None,
            identity_fingerprint: None,
            expires_at: None,
            reason: Some("required resource row missing from manifest".to_string()),
        },
        blockers: vec!["required resource row missing from manifest".to_string()],
    }
}

fn evaluate_resource(catalog: &CatalogEntry, spec: &ResourceSpec, now: i64) -> ResourceReport {
    let mut probes = Vec::with_capacity(spec.probes.len());
    let mut blockers = Vec::new();

    for probe in &spec.probes {
        let result = match probe.probe_type.as_str() {
            "executable" => probe_executable(probe),
            "file" => probe_file(probe),
            "apk-container" => probe_apk_container(probe),
            _ => unreachable!("manifest validation rejects unknown probe types"),
        };
        if result.status != "passed" {
            let reason = result
                .reason
                .clone()
                .unwrap_or_else(|| "probe failed".to_string());
            blockers.push(format!("{}: {}", result.label, reason));
        }
        probes.push(result);
    }

    let authorization = evaluate_authorization(&spec.id, spec.authorization.as_ref(), now);
    if let Some(reason) = &authorization.reason {
        if authorization.status != "not-required" && authorization.status != "authorized" {
            blockers.push(reason.clone());
        }
    }

    let identity = spec
        .authorization
        .as_ref()
        .and_then(|auth| auth.identity.as_deref())
        .unwrap_or("");
    let availability = if catalog.authorization_required {
        evaluate_availability(&spec.id, spec.availability.as_ref(), identity, now)
    } else {
        probe_availability(&probes)
    };
    if availability.status != "available" {
        if let Some(reason) = &availability.reason {
            blockers.push(reason.clone());
        }
    }

    let ready = probes.iter().all(|probe| probe.status == "passed")
        && availability.status == "available"
        && (!authorization.required || authorization.status == "authorized")
        && (!catalog.authorization_required || authorization.status == "authorized");

    let verification = if catalog.authorization_required {
        "authorization-and-availability-evidence"
    } else {
        "probed"
    };
    let status = if ready { "ready" } else { "blocked" };

    ResourceReport {
        id: catalog.id.to_string(),
        kind: catalog.kind.to_string(),
        owner: spec.owner.clone(),
        consuming_plan: catalog.consuming_plan.to_string(),
        description: catalog.description.to_string(),
        required: true,
        status,
        verification,
        probes,
        availability,
        authorization,
        blockers,
    }
}

fn probe_executable(probe: &ProbeSpec) -> ProbeResult {
    let label = probe.label.clone();
    let program = probe.program.as_deref().unwrap_or("");
    let path = match resolve_program(program) {
        Some(path) => path,
        None => return failed_probe(&label, "executable", "executable not found"),
    };

    let actual_hash = match sha256_file(&path) {
        Ok(hash) => hash,
        Err(_) => return failed_probe(&label, "executable", "probe read failed"),
    };
    if let Some(expected) = &probe.sha256 {
        if !expected.eq_ignore_ascii_case(&actual_hash) {
            return ProbeResult {
                label,
                probe_type: "executable".to_string(),
                status: "failed",
                sha256: Some(actual_hash),
                size_bytes: None,
                container_type: None,
                version: None,
                reason: Some("executable hash mismatch".to_string()),
            };
        }
    }

    let output = match Command::new(path).args(&probe.args).output() {
        Ok(output) => output,
        Err(_) => return failed_probe(&label, "executable", "probe execution failed"),
    };
    if !output.status.success() {
        return failed_probe_with_hash(&label, "executable", "unexpected exit status", actual_hash);
    }

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    if let Some(marker) = &probe.expected_output_contains {
        if !combined.contains(marker) {
            return failed_probe_with_hash(
                &label,
                "executable",
                "expected output marker not found",
                actual_hash,
            );
        }
    }
    if let Some(version) = &probe.expected_version {
        if !combined.contains(version) {
            return failed_probe_with_hash(
                &label,
                "executable",
                "expected version not found",
                actual_hash,
            );
        }
    }

    ProbeResult {
        label,
        probe_type: "executable".to_string(),
        status: "passed",
        sha256: Some(actual_hash),
        size_bytes: None,
        container_type: None,
        version: probe.expected_version.clone(),
        reason: None,
    }
}

fn probe_file(probe: &ProbeSpec) -> ProbeResult {
    let label = probe.label.clone();
    let path = match probe.path.as_deref() {
        Some(path) => Path::new(path),
        None => return failed_probe(&label, "file", "file path missing"),
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return failed_probe(&label, "file", "file not found"),
    };
    let actual_hash = match sha256_file(path) {
        Ok(hash) => hash,
        Err(_) => return failed_probe(&label, "file", "probe read failed"),
    };

    if let Some(expected) = &probe.sha256 {
        if !expected.eq_ignore_ascii_case(&actual_hash) {
            return ProbeResult {
                label,
                probe_type: "file".to_string(),
                status: "failed",
                sha256: Some(actual_hash),
                size_bytes: Some(metadata.len()),
                container_type: None,
                version: None,
                reason: Some("file hash mismatch".to_string()),
            };
        }
    }
    if let Some(expected_size) = probe.size_bytes {
        if expected_size != metadata.len() {
            return ProbeResult {
                label,
                probe_type: "file".to_string(),
                status: "failed",
                sha256: Some(actual_hash),
                size_bytes: Some(metadata.len()),
                container_type: None,
                version: None,
                reason: Some("file size mismatch".to_string()),
            };
        }
    }

    let version = probe.expected_version.clone();
    if probe.expected_contains.is_some() || probe.expected_version.is_some() {
        let mut content = String::new();
        match File::open(path).and_then(|mut file| file.read_to_string(&mut content)) {
            Ok(_) => {}
            Err(_) => {
                return ProbeResult {
                    label,
                    probe_type: "file".to_string(),
                    status: "failed",
                    sha256: Some(actual_hash),
                    size_bytes: Some(metadata.len()),
                    container_type: None,
                    version: None,
                    reason: Some("probe read failed".to_string()),
                }
            }
        }
        if let Some(marker) = &probe.expected_contains {
            if !content.contains(marker) {
                return failed_file(
                    &label,
                    "expected file content marker not found",
                    actual_hash,
                    metadata.len(),
                );
            }
        }
        if let Some(expected) = &probe.expected_version {
            if !content.contains(expected) {
                return failed_file(
                    &label,
                    "expected version not found",
                    actual_hash,
                    metadata.len(),
                );
            }
        }
    }

    ProbeResult {
        label,
        probe_type: "file".to_string(),
        status: "passed",
        sha256: Some(actual_hash),
        size_bytes: Some(metadata.len()),
        container_type: None,
        version,
        reason: None,
    }
}

fn probe_apk_container(probe: &ProbeSpec) -> ProbeResult {
    let label = probe.label.clone();
    let path = match probe.path.as_deref() {
        Some(path) => Path::new(path),
        None => return failed_probe(&label, "apk-container", "file path missing"),
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return failed_probe(&label, "apk-container", "file not found"),
    };
    let actual_hash = match sha256_file(path) {
        Ok(hash) => hash,
        Err(_) => return failed_probe(&label, "apk-container", "probe read failed"),
    };
    let expected_hash = probe.sha256.as_deref().unwrap_or("");
    if !expected_hash.eq_ignore_ascii_case(&actual_hash) {
        return failed_container(
            &label,
            "file hash mismatch",
            actual_hash,
            metadata.len(),
            None,
        );
    }
    if let Some(expected_size) = probe.size_bytes {
        if expected_size != metadata.len() {
            return failed_container(
                &label,
                "file size mismatch",
                actual_hash,
                metadata.len(),
                None,
            );
        }
    }

    let expected_type = match probe.container_type.as_deref() {
        Some(value) => value,
        None => return failed_probe(&label, "apk-container", "expected container type missing"),
    };
    if probe.oracle.is_none() {
        return failed_probe(&label, "apk-container", "oracle configuration missing");
    }

    let observed = match classify_container_identity(path) {
        Ok(observed) => observed,
        Err(reason) => return failed_container(&label, reason, actual_hash, metadata.len(), None),
    };
    if observed != expected_type {
        return failed_container(
            &label,
            "container type mismatch",
            actual_hash,
            metadata.len(),
            Some(observed.to_string()),
        );
    }

    ProbeResult {
        label,
        probe_type: "apk-container".to_string(),
        status: "passed",
        sha256: Some(actual_hash),
        size_bytes: Some(metadata.len()),
        container_type: Some(observed.to_string()),
        version: None,
        reason: None,
    }
}

fn classify_container_identity(archive: &Path) -> Result<&'static str, &'static str> {
    let report = crate::archives::inspect_safe_zip_path(archive)
        .map_err(|_| "container identity classification failed")?;
    Ok(match report.container_type {
        crate::models::ContainerType::Apk => "APK",
        crate::models::ContainerType::Apkm => "APKM",
        crate::models::ContainerType::Xapk => "XAPK",
        crate::models::ContainerType::Apks => "APKS",
        crate::models::ContainerType::Unknown => "UNKNOWN",
    })
}

fn failed_container(
    label: &str,
    reason: &str,
    sha256: String,
    size: u64,
    container_type: Option<String>,
) -> ProbeResult {
    ProbeResult {
        label: label.to_string(),
        probe_type: "apk-container".to_string(),
        status: "failed",
        sha256: Some(sha256),
        size_bytes: Some(size),
        container_type,
        version: None,
        reason: Some(reason.to_string()),
    }
}

fn failed_probe(label: &str, probe_type: &str, reason: &str) -> ProbeResult {
    ProbeResult {
        label: label.to_string(),
        probe_type: probe_type.to_string(),
        status: "failed",
        sha256: None,
        size_bytes: None,
        container_type: None,
        version: None,
        reason: Some(reason.to_string()),
    }
}

fn failed_probe_with_hash(
    label: &str,
    probe_type: &str,
    reason: &str,
    sha256: String,
) -> ProbeResult {
    ProbeResult {
        label: label.to_string(),
        probe_type: probe_type.to_string(),
        status: "failed",
        sha256: Some(sha256),
        size_bytes: None,
        container_type: None,
        version: None,
        reason: Some(reason.to_string()),
    }
}

fn failed_file(label: &str, reason: &str, sha256: String, size: u64) -> ProbeResult {
    ProbeResult {
        label: label.to_string(),
        probe_type: "file".to_string(),
        status: "failed",
        sha256: Some(sha256),
        size_bytes: Some(size),
        container_type: None,
        version: None,
        reason: Some(reason.to_string()),
    }
}

fn probe_availability(probes: &[ProbeResult]) -> AvailabilityReport {
    let available = probes.iter().all(|probe| probe.status == "passed");
    AvailabilityReport {
        source: "probe",
        status: if available {
            "available"
        } else {
            "unavailable"
        },
        evidence_sha256: None,
        observed_at: None,
        expires_at: None,
        reason: None,
    }
}

fn evaluate_availability(
    resource_id: &str,
    pin: Option<&EvidencePin>,
    identity: &str,
    now: i64,
) -> AvailabilityReport {
    let Some(pin) = pin else {
        return availability_failure(
            "missing",
            "availability evidence pin missing",
            None,
            None,
            None,
        );
    };
    let content = match fs::read(&pin.path) {
        Ok(content) => content,
        Err(_) => {
            return availability_failure(
                "missing",
                "availability evidence missing",
                None,
                None,
                None,
            )
        }
    };
    let actual_hash = sha256_hex(&content);
    if !pin.sha256.eq_ignore_ascii_case(&actual_hash) {
        return availability_failure(
            "stale",
            "availability evidence hash mismatch",
            Some(actual_hash),
            None,
            None,
        );
    }
    let parsed: AvailabilityEvidence = match serde_json::from_slice(&content) {
        Ok(parsed) => parsed,
        Err(_) => {
            return availability_failure(
                "malformed",
                "availability evidence malformed",
                Some(actual_hash),
                None,
                None,
            )
        }
    };
    if parsed.schema_version != SCHEMA_VERSION
        || parsed.probe.trim().is_empty()
        || parsed.scenario.trim().is_empty()
        || parsed.resource_id.trim().is_empty()
        || parsed.identity.trim().is_empty()
    {
        return availability_failure(
            "malformed",
            "availability evidence malformed",
            Some(actual_hash),
            None,
            None,
        );
    }
    if parsed.resource_id != resource_id {
        return availability_failure(
            "resource-mismatch",
            "availability evidence names a different resource",
            Some(actual_hash),
            None,
            None,
        );
    }
    if parsed.identity != identity {
        return availability_failure(
            "identity-mismatch",
            "availability evidence identity mismatch",
            Some(actual_hash),
            None,
            None,
        );
    }

    let observed = parse_utc(&parsed.observed_at);
    let expires = parse_utc(&parsed.expires_at);
    let (observed, expires) = match (observed, expires) {
        (Some(observed), Some(expires)) if observed < expires => (observed, expires),
        _ => {
            return availability_failure(
                "malformed",
                "availability evidence validity window malformed",
                Some(actual_hash),
                None,
                None,
            )
        }
    };
    if now < observed {
        return availability_failure(
            "not-yet-valid",
            "availability evidence is not yet valid",
            Some(actual_hash),
            Some(parsed.observed_at),
            Some(parsed.expires_at),
        );
    }
    if now >= expires {
        return availability_failure(
            "expired",
            "availability evidence is expired",
            Some(actual_hash),
            Some(parsed.observed_at),
            Some(parsed.expires_at),
        );
    }

    let mut report = AvailabilityReport {
        source: "evidence",
        status: "invalid",
        evidence_sha256: Some(actual_hash),
        observed_at: Some(parsed.observed_at),
        expires_at: Some(parsed.expires_at),
        reason: None,
    };
    match parsed.outcome.as_str() {
        "available" => report.status = "available",
        "unavailable" => {
            report.status = "unavailable";
            report.reason = Some("availability observation reports unavailable".to_string());
        }
        "pending" => {
            report.status = "pending";
            report.reason = Some("availability observation pending".to_string());
        }
        _ => {
            report.status = "invalid";
            report.reason = Some("availability outcome is not recognized".to_string());
        }
    }
    report
}

fn availability_failure(
    status: &'static str,
    reason: &str,
    evidence_sha256: Option<String>,
    observed_at: Option<String>,
    expires_at: Option<String>,
) -> AvailabilityReport {
    AvailabilityReport {
        source: "evidence",
        status,
        evidence_sha256,
        observed_at,
        expires_at,
        reason: Some(reason.to_string()),
    }
}

fn evaluate_authorization(
    resource_id: &str,
    spec: Option<&AuthorizationSpec>,
    now: i64,
) -> AuthorizationReport {
    let Some(spec) = spec else {
        return AuthorizationReport {
            required: false,
            status: "not-required",
            reference: None,
            evidence_sha256: None,
            identity_fingerprint: None,
            expires_at: None,
            reason: None,
        };
    };
    if !spec.required {
        return AuthorizationReport {
            required: false,
            status: "not-required",
            reference: None,
            evidence_sha256: None,
            identity_fingerprint: None,
            expires_at: None,
            reason: None,
        };
    }

    let reference = spec.reference.clone();
    let identity = spec.identity.clone().unwrap_or_default();
    let fingerprint = Some(identity_fingerprint(&identity));
    let evidence = match spec.evidence.as_ref() {
        Some(evidence) => evidence,
        None => {
            return AuthorizationReport {
                required: true,
                status: "missing",
                reference,
                evidence_sha256: None,
                identity_fingerprint: fingerprint,
                expires_at: None,
                reason: Some("authorization evidence pin missing".to_string()),
            }
        }
    };

    let content = match fs::read(&evidence.path) {
        Ok(content) => content,
        Err(_) => {
            return AuthorizationReport {
                required: true,
                status: "missing",
                reference,
                evidence_sha256: None,
                identity_fingerprint: fingerprint,
                expires_at: None,
                reason: Some("authorization evidence missing".to_string()),
            }
        }
    };
    let actual_hash = sha256_hex(&content);
    if !evidence.sha256.eq_ignore_ascii_case(&actual_hash) {
        return AuthorizationReport {
            required: true,
            status: "stale",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: None,
            reason: Some("authorization evidence hash mismatch".to_string()),
        };
    }

    let parsed: AuthorizationEvidence = match serde_json::from_slice(&content) {
        Ok(parsed) => parsed,
        Err(_) => {
            return AuthorizationReport {
                required: true,
                status: "malformed",
                reference,
                evidence_sha256: Some(actual_hash),
                identity_fingerprint: fingerprint,
                expires_at: None,
                reason: Some("authorization evidence malformed".to_string()),
            }
        }
    };

    if parsed.schema_version != SCHEMA_VERSION
        || parsed.scope.trim().is_empty()
        || parsed.has_empty_identity_material()
    {
        return AuthorizationReport {
            required: true,
            status: "malformed",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: None,
            reason: Some("authorization evidence malformed".to_string()),
        };
    }
    if parsed.resource_id != resource_id {
        return AuthorizationReport {
            required: true,
            status: "resource-mismatch",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: None,
            reason: Some("authorization evidence names a different resource".to_string()),
        };
    }
    if parsed.identity != identity {
        return AuthorizationReport {
            required: true,
            status: "identity-mismatch",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: None,
            reason: Some("authorization evidence identity mismatch".to_string()),
        };
    }

    let issued = parse_utc(&parsed.issued_at);
    let expires = parse_utc(&parsed.expires_at);
    let (issued, expires) = match (issued, expires) {
        (Some(issued), Some(expires)) if issued < expires => (issued, expires),
        _ => {
            return AuthorizationReport {
                required: true,
                status: "malformed",
                reference,
                evidence_sha256: Some(actual_hash),
                identity_fingerprint: fingerprint,
                expires_at: None,
                reason: Some("authorization evidence validity window malformed".to_string()),
            }
        }
    };
    if now < issued {
        return AuthorizationReport {
            required: true,
            status: "not-yet-valid",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: Some("authorization evidence is not yet valid".to_string()),
        };
    }
    if now >= expires {
        return AuthorizationReport {
            required: true,
            status: "expired",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: Some("authorization evidence is expired".to_string()),
        };
    }

    match parsed.decision.as_str() {
        "authorized" => AuthorizationReport {
            required: true,
            status: "authorized",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: None,
        },
        "denied" => AuthorizationReport {
            required: true,
            status: "denied",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: Some("authorization decision is denied".to_string()),
        },
        "pending" => AuthorizationReport {
            required: true,
            status: "pending",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: Some("authorization decision is pending".to_string()),
        },
        _ => AuthorizationReport {
            required: true,
            status: "invalid",
            reference,
            evidence_sha256: Some(actual_hash),
            identity_fingerprint: fingerprint,
            expires_at: Some(parsed.expires_at),
            reason: Some("authorization decision is not recognized".to_string()),
        },
    }
}

impl AuthorizationEvidence {
    /// `deny_unknown_fields` already prevents a bare boolean, but this guards
    /// against whitespace-only identity material.
    fn has_empty_identity_material(&self) -> bool {
        self.resource_id.trim().is_empty() || self.identity.trim().is_empty()
    }
}

// ---------------------------------------------------------------------------
// Report output
// ---------------------------------------------------------------------------

fn write_report(report: &Report, out: Option<&Path>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("cannot serialize prerequisite report: {error}"))?;
    let json = format!("{json}\n");

    match out {
        Some(path) if path != Path::new("-") => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .map_err(|_| "cannot create prerequisite report directory".to_string())?;
                }
            }
            fs::write(path, json).map_err(|_| "cannot write prerequisite report".to_string())?;
        }
        _ => {
            let mut stdout = std::io::stdout().lock();
            stdout
                .write_all(json.as_bytes())
                .and_then(|_| stdout.flush())
                .map_err(|_| "cannot write prerequisite report to stdout".to_string())?;
        }
    }
    Ok(())
}

fn print_summary(report: &Report) {
    if report.blockers.is_empty() {
        eprintln!(
            "prerequisites: all {} required resources ready",
            report.summary.total
        );
        return;
    }

    eprintln!(
        "prerequisites: {}/{} required resources ready; {} blocker(s)",
        report.summary.ready,
        report.summary.total,
        report.blockers.len()
    );
    eprintln!("PREREQUISITE BLOCKERS:");
    for blocker in &report.blockers {
        eprintln!(
            "  BLOCKED {} [owner: {}]: {}",
            blocker.resource_id, blocker.owner, blocker.reason
        );
    }
}

// ---------------------------------------------------------------------------
// Probe helpers
// ---------------------------------------------------------------------------

fn resolve_program(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') || program.contains('\\') {
        return if path.is_file() {
            Some(path.to_path_buf())
        } else {
            None
        };
    }
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_bytes(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_bytes(&hasher.finalize())
}

fn identity_fingerprint(identity: &str) -> String {
    sha256_hex(identity.as_bytes()).chars().take(16).collect()
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
// Minimal UTC RFC 3339 timestamps (no chrono dependency)
// ---------------------------------------------------------------------------

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
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
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

/// Howard Hinnant's civil-from-days algorithm.
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

/// Inverse of `days_from_civil`.
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
    use serde_json::{json, Value};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "apk-lab-prerequisites-{}-{}-{name}",
                std::process::id(),
                nonce
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_json(path: &Path, value: &Value) {
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn sha_of(path: &Path) -> String {
        sha256_file(path).unwrap()
    }

    /// Keeps synthetic evidence internally consistent: the start of the
    /// validity window must precede its expiry. Past expiries use a past
    /// start so the fixture describes an expired window, not a malformed one.
    fn fake_window_start(expires_at: &str) -> &'static str {
        if expires_at <= "2026-01-01T00:00:00Z" {
            "2019-01-01T00:00:00Z"
        } else {
            "2026-01-01T00:00:00Z"
        }
    }

    fn fake_authorization(
        dir: &TempDir,
        resource_id: &str,
        decision: &str,
        identity: &str,
        expires_at: &str,
    ) -> (String, String) {
        let name = resource_id.replace('.', "-");
        let path = dir.join(&format!("{name}.json"));
        let evidence = json!({
            "schemaVersion": 1,
            "resourceId": resource_id,
            "decision": decision,
            "identity": identity,
            "issuedAt": fake_window_start(expires_at),
            "expiresAt": expires_at,
            "scope": "synthetic unit-test scope"
        });
        write_json(&path, &evidence);
        (path.to_string_lossy().to_string(), sha_of(&path))
    }

    fn fake_availability(
        dir: &TempDir,
        resource_id: &str,
        identity: &str,
        outcome: &str,
        expires_at: &str,
    ) -> (String, String) {
        let name = format!("{}-availability", resource_id.replace('.', "-"));
        let path = dir.join(&format!("{name}.json"));
        let evidence = json!({
            "schemaVersion": 1,
            "resourceId": resource_id,
            "identity": identity,
            "probe": "unit-test-probe",
            "outcome": outcome,
            "observedAt": fake_window_start(expires_at),
            "expiresAt": expires_at,
            "scenario": "synthetic unit-test availability observation"
        });
        write_json(&path, &evidence);
        (path.to_string_lossy().to_string(), sha_of(&path))
    }

    fn auth_identity(resource_id: &str) -> &'static str {
        match resource_id {
            "device.disposable" => "test-device",
            "cloud.r2" => "test-bucket-alias",
            "cloud.worker-callback" => "test-worker-alias",
            "ci.compatibility-workflow" => "test-workflow-alias",
            _ => "test-play-alias",
        }
    }

    /// Builds a complete manifest with passing host/corpus probes.
    fn base_manifest(
        dir: &TempDir,
        decision: &str,
        availability_outcome: &str,
        expires_at: &str,
    ) -> Value {
        let corpus_a = dir.join("single.apk");
        let corpus_b = dir.join("split.apkm");
        let corpus_mpp = dir.join("patches.mpp");
        write_test_zip(&corpus_a, &[("AndroidManifest.xml", b"<manifest/>")]);
        write_test_zip(&corpus_b, &[("base.apk", b"synthetic base apk")]);
        fs::write(&corpus_mpp, b"synthetic-mpp").unwrap();

        let tools: Vec<(&str, Vec<Value>)> = vec![
            (
                "host.jdk",
                vec![exec(
                    "java",
                    "/bin/sh",
                    &["-c", "echo java 21.0.12.1"],
                    "java 21.0.12.1",
                    "21.0.12.1",
                )],
            ),
            (
                "host.android-sdk",
                vec![exec(
                    "aapt2",
                    "/bin/sh",
                    &["-c", "echo aapt2 36.0.0"],
                    "aapt2 36.0.0",
                    "36.0.0",
                )],
            ),
            (
                "host.android-ndk",
                vec![file(
                    "ndk",
                    &dir.join("ndk.properties"),
                    b"Pkg.Revision = 28.2.13676358",
                    "28.2.13676358",
                    Some("Pkg.Revision = 28.2.13676358"),
                )],
            ),
            (
                "host.cmake",
                vec![exec(
                    "cmake",
                    "/bin/sh",
                    &["-c", "echo cmake version 4.5.0"],
                    "cmake version",
                    "4.5.0",
                )],
            ),
            (
                "host.ninja",
                vec![exec(
                    "ninja",
                    "/bin/sh",
                    &["-c", "echo 1.13.2"],
                    "1.13.2",
                    "1.13.2",
                )],
            ),
            (
                "host.bun",
                vec![exec(
                    "bun",
                    "/bin/sh",
                    &["-c", "echo 1.4.4"],
                    "1.4.4",
                    "1.4.4",
                )],
            ),
            (
                "host.rust",
                vec![exec(
                    "rustc",
                    "/bin/sh",
                    &["-c", "echo rustc 1.97.1"],
                    "rustc 1.97.1",
                    "1.97.1",
                )],
            ),
        ];

        let mut resources = Vec::new();
        for (id, probes) in tools {
            resources.push(json!({
                "id": id,
                "kind": "host-tool",
                "owner": "test-operator",
                "probes": probes
            }));
        }

        resources.push(json!({
            "id": "corpus.single-apk",
            "kind": "artifact-corpus",
            "owner": "test-benchmark-owner",
            "probes": [apk_container("single-apk", &corpus_a, "APK")]
        }));
        resources.push(json!({
            "id": "corpus.split-apk",
            "kind": "artifact-corpus",
            "owner": "test-benchmark-owner",
            "probes": [apk_container("split-apk", &corpus_b, "APKM")]
        }));
        resources.push(json!({
            "id": "corpus.mpp",
            "kind": "artifact-corpus",
            "owner": "test-benchmark-owner",
            "probes": [file("artifact", &corpus_mpp, b"", "", None)]
        }));

        let auth_resources = [
            ("device.disposable", "device-authorization"),
            ("play.latest", "play-delivery"),
            ("play.older", "play-delivery"),
            ("play.direct", "play-delivery"),
            ("play.dispenser", "play-delivery"),
            ("play.oauth", "play-oauth"),
            ("play.hosts", "play-network-hosts"),
            ("cloud.r2", "cloud-storage"),
            ("cloud.worker-callback", "worker-endpoint"),
            ("ci.compatibility-workflow", "ci-workflow"),
        ];
        for (id, kind) in auth_resources {
            let identity = auth_identity(id);
            let (path, hash) = fake_authorization(dir, id, decision, identity, expires_at);
            let (availability_path, availability_hash) =
                fake_availability(dir, id, identity, availability_outcome, expires_at);
            resources.push(json!({
                "id": id,
                "kind": kind,
                "owner": "test-owner",
                "availability": {"path": availability_path, "sha256": availability_hash},
                "authorization": {
                    "required": true,
                    "reference": format!("AUTH-{}", id.replace('.', "-")),
                    "identity": identity,
                    "evidence": {"path": path, "sha256": hash}
                }
            }));
        }

        json!({
            "schemaVersion": 1,
            "kind": MANIFEST_KIND,
            "generatedAt": "2026-11-20T00:00:00Z",
            "host": {
                "profile": "test-host",
                "os": env::consts::OS,
                "arch": env::consts::ARCH,
                "cpuCount": 8,
                "memoryBytes": 17179869184u64
            },
            "resources": resources
        })
    }

    fn exec(
        label: &str,
        program: &str,
        args: &[&str],
        expected_output_contains: &str,
        expected_version: &str,
    ) -> Value {
        json!({
            "label": label,
            "type": "executable",
            "program": program,
            "args": args,
            "expectedOutputContains": expected_output_contains,
            "expectedVersion": expected_version
        })
    }

    fn file(
        label: &str,
        path: &Path,
        content: &[u8],
        version: &str,
        expected_contains: Option<&str>,
    ) -> Value {
        if !path.exists() {
            fs::write(path, content).unwrap();
        }
        let mut value = json!({
            "label": label,
            "type": "file",
            "path": path.to_string_lossy(),
            "sha256": sha_of(path),
            "sizeBytes": fs::metadata(path).unwrap().len()
        });
        if !version.is_empty() {
            value["expectedVersion"] = json!(version);
        }
        if let Some(marker) = expected_contains {
            value["expectedContains"] = json!(marker);
        }
        value
    }

    fn apk_container(label: &str, path: &Path, expected: &str) -> Value {
        json!({
            "label": label,
            "type": "apk-container",
            "path": path.to_string_lossy(),
            "sha256": sha_of(path),
            "sizeBytes": fs::metadata(path).unwrap().len(),
            "containerType": expected,
            "oracle": {
                "program": "native",
                "sha256": "0".repeat(64),
                "moduleRoot": "."
            }
        })
    }

    fn container_spec(label: &str, path: &Path, expected: &str) -> ProbeSpec {
        ProbeSpec {
            label: label.to_string(),
            probe_type: "apk-container".to_string(),
            program: None,
            args: Vec::new(),
            expected_output_contains: None,
            expected_version: None,
            path: Some(path.to_string_lossy().to_string()),
            sha256: Some(sha_of(path)),
            size_bytes: Some(fs::metadata(path).unwrap().len()),
            expected_contains: None,
            container_type: Some(expected.to_string()),
            oracle: Some(OracleSpec {
                program: "native".to_string(),
                sha256: "0".repeat(64),
                module_root: ".".to_string(),
            }),
        }
    }

    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut body = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = body.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&crc.to_le_bytes());
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&(name.len() as u16).to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = body.len() as u32;
        let central_size = central.len() as u32;
        body.extend_from_slice(&central);
        body.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        body.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        body.extend_from_slice(&central_size.to_le_bytes());
        body.extend_from_slice(&central_offset.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        fs::write(path, body).unwrap();
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn manifest_path(dir: &TempDir, manifest: &Value) -> PathBuf {
        let path = dir.join("prerequisites.json");
        write_json(&path, manifest);
        path
    }

    fn read_report(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn args(manifest: &Path, out: &Path, check: bool) -> Vec<String> {
        let mut args = vec![
            "--manifest".to_string(),
            manifest.to_string_lossy().to_string(),
        ];
        if check {
            args.push("--check".to_string());
        }
        args.push("--out".to_string());
        args.push(out.to_string_lossy().to_string());
        args
    }

    #[test]
    fn utc_roundtrip() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-11-20T19:30:05Z"), Some(1_795_203_005));
        assert_eq!(parse_utc(&format_utc(1_795_203_005)), Some(1_795_203_005));
        assert_eq!(parse_utc("2026-02-29T00:00:00Z"), None);
        assert_eq!(parse_utc("2026-11-20 19:30:05Z"), None);
    }

    #[test]
    fn inventory_writes_sanitized_blocked_report_and_succeeds() {
        let dir = TempDir::new("inventory-blocked");
        let manifest = base_manifest(&dir, "pending", "available", "2099-11-20T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        run(&args(&manifest, &out, false)).unwrap();

        let report = read_report(&out);
        assert_eq!(report["schemaVersion"], json!(1));
        assert_eq!(report["kind"], REPORT_KIND);
        assert_eq!(report["mode"], "inventory");
        assert_eq!(
            report["summary"]["total"].as_u64().unwrap() as usize,
            CATALOG.len()
        );
        assert_eq!(report["summary"]["ready"], json!(10));
        assert_eq!(report["summary"]["blocked"], json!(10));
        assert_eq!(report["summary"]["missing"], json!(0));
        assert_eq!(report["summary"]["acceptancePassed"], false);
        assert_eq!(report["blockers"].as_array().unwrap().len(), 10);

        let text = String::from_utf8(fs::read(&out).unwrap()).unwrap();
        assert!(!text.contains(&dir.path.to_string_lossy().to_string()));
        assert!(!text.contains("test-play-alias"));
        assert!(!text.contains("authorization/"));
        assert!(!text.contains("synthetic-single-apk"));
        for resource in report["resources"].as_array().unwrap() {
            assert!(resource.get("probes").is_some());
            assert_eq!(resource["required"], true);
        }
    }

    #[test]
    fn check_accepts_all_ready_resources() {
        let dir = TempDir::new("check-ready");
        let manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        run(&args(&manifest, &out, true)).unwrap();

        let report = read_report(&out);
        assert_eq!(report["mode"], "check");
        assert_eq!(report["summary"]["acceptancePassed"], true);
        assert_eq!(
            report["summary"]["ready"].as_u64().unwrap() as usize,
            CATALOG.len()
        );
        assert_eq!(report["blockers"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn check_rejects_pending_authorization() {
        let dir = TempDir::new("pending-auth");
        let manifest = base_manifest(&dir, "pending", "available", "2099-11-20T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert_eq!(report["summary"]["acceptancePassed"], false);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"].as_str().unwrap().contains("pending")));
    }

    #[test]
    fn check_rejects_authorized_resource_without_availability_observation() {
        let dir = TempDir::new("authorized-unobserved");
        let manifest = base_manifest(&dir, "authorized", "pending", "2099-11-20T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        let device = report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        assert_eq!(device["authorization"]["status"], "authorized");
        assert_eq!(device["availability"]["status"], "pending");
        assert_eq!(device["status"], "blocked");
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("availability observation pending")));
    }

    #[test]
    fn check_rejects_unavailable_availability_observation() {
        let dir = TempDir::new("availability-unavailable");
        let manifest = base_manifest(&dir, "authorized", "unavailable", "2099-11-20T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("reports unavailable")));
    }

    #[test]
    fn check_rejects_stale_availability_evidence() {
        let dir = TempDir::new("availability-stale");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        device["availability"]["sha256"] = json!("0".repeat(64));
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("availability evidence hash mismatch")));
    }

    #[test]
    fn rejects_missing_availability_pin() {
        let dir = TempDir::new("availability-missing");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        let object = device.as_object_mut().unwrap();
        object.remove("availability");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, false)).unwrap_err();
        assert!(error.contains("availability evidence"));
    }

    #[test]
    fn inventory_succeeds_while_reporting_stale_hash() {
        let dir = TempDir::new("inventory-stale");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let corpus = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap();
        corpus["probes"][0]["sha256"] = json!("0".repeat(64));
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        run(&args(&manifest, &out, false)).unwrap();

        let report = read_report(&out);
        assert_eq!(report["summary"]["acceptancePassed"], false);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("file hash mismatch")));
    }

    #[test]
    fn check_rejects_missing_required_row() {
        let dir = TempDir::new("missing-row");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["resources"]
            .as_array_mut()
            .unwrap()
            .retain(|resource| resource["id"] != "corpus.mpp");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        let missing = report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["id"] == "corpus.mpp")
            .unwrap();
        assert_eq!(missing["status"], "missing");
        assert_eq!(report["summary"]["missing"], json!(1));

        // Inventory mode still reports the incomplete gate prominently without
        // pretending the row exists.
        run(&args(&manifest, &out, false)).unwrap();
    }

    #[test]
    fn check_rejects_unavailable_probe() {
        let dir = TempDir::new("unavailable");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["resources"][0]["probes"][0]["program"] = json!("/nonexistent/java");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");

        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert_eq!(report["resources"][0]["id"], "host.jdk");
        assert_eq!(report["resources"][0]["status"], "blocked");
        assert!(report["blockers"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("executable not found"));
    }

    #[test]
    fn check_rejects_authorization_without_evidence() {
        let dir = TempDir::new("fake-auth");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        device["authorization"] = json!({
            "required": true,
            "reference": "AUTH-device-disposable",
            "identity": "test-device"
        });

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("pinned evidence"));
    }

    #[test]
    fn rejects_asserted_boolean_authorization() {
        let dir = TempDir::new("boolean-auth");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        device["authorization"] = json!({
            "required": true,
            "authorized": true,
            "reference": "AUTH-device-disposable",
            "identity": "test-device"
        });

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(
            error.contains("unknown field") || error.contains("invalid prerequisite manifest"),
            "{error}"
        );
    }

    #[test]
    fn check_rejects_stale_input_hash() {
        let dir = TempDir::new("stale-hash");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let corpus = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap();
        corpus["probes"][0]["sha256"] = json!("0".repeat(64));

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("file hash mismatch")));
    }

    #[test]
    fn check_rejects_stale_authorization_evidence_hash() {
        let dir = TempDir::new("stale-evidence");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        device["authorization"]["evidence"]["sha256"] = json!("f".repeat(64));

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("evidence hash mismatch")));
    }

    #[test]
    fn check_rejects_expired_authorization() {
        let dir = TempDir::new("expired");
        let manifest = base_manifest(&dir, "authorized", "available", "2020-01-01T00:00:00Z");
        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"].as_str().unwrap().contains("expired")));
    }

    #[test]
    fn check_rejects_malformed_authorization_window() {
        let dir = TempDir::new("malformed-window");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        let evidence_path = PathBuf::from(
            device["authorization"]["evidence"]["path"]
                .as_str()
                .unwrap(),
        );
        let evidence = json!({
            "schemaVersion": 1,
            "resourceId": "device.disposable",
            "decision": "authorized",
            "identity": "test-device",
            "issuedAt": "2099-11-20T00:00:00Z",
            "expiresAt": "2099-11-19T00:00:00Z",
            "scope": "synthetic unit-test scope"
        });
        write_json(&evidence_path, &evidence);
        device["authorization"]["evidence"]["sha256"] = json!(sha_of(&evidence_path));

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("validity window malformed")));
    }

    #[test]
    fn check_rejects_identity_mismatch() {
        let dir = TempDir::new("identity");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let device = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "device.disposable")
            .unwrap();
        let evidence_path = PathBuf::from(
            device["authorization"]["evidence"]["path"]
                .as_str()
                .unwrap(),
        );
        let evidence = json!({
            "schemaVersion": 1,
            "resourceId": "device.disposable",
            "decision": "authorized",
            "identity": "someone-else",
            "issuedAt": "2026-01-01T00:00:00Z",
            "expiresAt": "2099-11-20T00:00:00Z",
            "scope": "synthetic unit-test scope"
        });
        write_json(&evidence_path, &evidence);
        device["authorization"]["evidence"]["sha256"] = json!(sha_of(&evidence_path));

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));
        let report = read_report(&out);
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["reason"]
                .as_str()
                .unwrap()
                .contains("identity mismatch")));
    }

    #[test]
    fn rejects_unknown_duplicate_and_mismatched_rows() {
        let dir = TempDir::new("invalid-rows");
        let base = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");

        let mut unknown = base.clone();
        unknown["resources"].as_array_mut().unwrap().push(json!({
            "id": "host.unknown",
            "kind": "host-tool",
            "owner": "test-operator",
            "probes": [exec("x", "/bin/sh", &["-c", "echo x"], "x", "x")]
        }));
        let path = manifest_path(&dir, &unknown);
        let out = dir.join("report.json");
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("unknown resource"));

        let mut duplicate = base.clone();
        let first = duplicate["resources"][0].clone();
        duplicate["resources"].as_array_mut().unwrap().push(first);
        let path = manifest_path(&dir, &duplicate);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("duplicate resource"));

        let mut mismatched = base;
        mismatched["resources"][0]["kind"] = json!("artifact-corpus");
        let path = manifest_path(&dir, &mismatched);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("expected"));
    }

    #[test]
    fn rejects_unsupported_schema_version_and_host_mismatch() {
        let dir = TempDir::new("bad-manifest");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["schemaVersion"] = json!(2);
        let path = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("schemaVersion"));

        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["host"]["arch"] = json!(if env::consts::ARCH == "aarch64" {
            "x86_64"
        } else {
            "aarch64"
        });
        let path = manifest_path(&dir, &manifest);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("host.arch"));
    }

    #[test]
    fn container_identity_uses_archive_members_not_filename() {
        let dir = TempDir::new("container-identity");

        let single = dir.join("looks-like-single.apk");
        write_test_zip(&single, &[("AndroidManifest.xml", b"<manifest/>")]);
        let apkm = dir.join("also-named-single.apk");
        write_test_zip(&apkm, &[("base.apk", b"base"), ("info.json", b"{}")]);
        let xapk = dir.join("still-named-single.apk");
        write_test_zip(&xapk, &[("base.apk", b"base"), ("manifest.json", b"{}")]);
        let apks = dir.join("named-single.apk");
        write_test_zip(&apks, &[("base.apk", b"base"), ("toc.pb", b"toc")]);

        for (path, expected) in [
            (&single, "APK"),
            (&apkm, "APKM"),
            (&xapk, "XAPK"),
            (&apks, "APKS"),
        ] {
            let result = probe_apk_container(&container_spec("identity", path, expected));
            assert_eq!(result.status, "passed", "{expected}: {:?}", result.reason);
            assert_eq!(result.container_type.as_deref(), Some(expected));
        }
    }

    #[test]
    fn check_rejects_mislabelled_apkm_as_single_apk() {
        let dir = TempDir::new("mislabelled-single");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let archive = dir.join("single.apk");
        write_test_zip(&archive, &[("base.apk", b"base"), ("info.json", b"{}")]);
        let corpus = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap();
        corpus["probes"][0]["sha256"] = json!(sha_of(&archive));
        corpus["probes"][0]["sizeBytes"] = json!(fs::metadata(&archive).unwrap().len());

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));

        let report = read_report(&out);
        let resource = report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap();
        assert_eq!(resource["status"], "blocked");
        assert_eq!(resource["probes"][0]["containerType"], "APKM");
        assert!(resource["probes"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("container type mismatch"));
        assert!(report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["resourceId"] == "corpus.single-apk"
                && blocker["reason"]
                    .as_str()
                    .unwrap()
                    .contains("container type mismatch")));
    }

    #[test]
    fn container_identity_verifies_hashes_before_classification() {
        let dir = TempDir::new("identity-hash-order");
        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        let corpus = manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap();
        corpus["probes"][0]["sha256"] = json!("0".repeat(64));

        let manifest = manifest_path(&dir, &manifest);
        let out = dir.join("report.json");
        let error = run(&args(&manifest, &out, true)).unwrap_err();
        assert!(error.contains("check failed"));

        let report = read_report(&out);
        let probe = &report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap()["probes"][0];
        assert_eq!(probe["status"], "failed");
        assert_eq!(probe["reason"], "file hash mismatch");
        assert!(probe.get("containerType").is_none());
    }

    #[test]
    fn rejects_corpus_rows_without_typed_container_identity() {
        let dir = TempDir::new("container-required");
        let out = dir.join("report.json");
        let single_file = dir.join("single.apk");

        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap()["probes"] = json!([file("artifact", &single_file, b"", "", None)]);
        let path = manifest_path(&dir, &manifest);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("exactly one apk-container"));

        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.single-apk")
            .unwrap()["probes"][0]["containerType"] = json!("APKM");
        let path = manifest_path(&dir, &manifest);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("must pin containerType APK"));

        let mut manifest = base_manifest(&dir, "authorized", "available", "2099-11-20T00:00:00Z");
        manifest["resources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|resource| resource["id"] == "corpus.split-apk")
            .unwrap()["probes"][0]["containerType"] = json!("APK");
        let path = manifest_path(&dir, &manifest);
        assert!(run(&args(&path, &out, false))
            .unwrap_err()
            .contains("must pin a split container type"));
    }

    #[test]
    fn help_is_side_effect_free() {
        assert!(run(&["--help".to_string()]).is_ok());
    }
}
