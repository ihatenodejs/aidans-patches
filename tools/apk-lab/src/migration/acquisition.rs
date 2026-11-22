//! Acquisition proof gate for the plan-002 entry gate.
//!
//! `preflight acquisition` evaluates the native Play delivery probe reports
//! written by `play` and cross-checks them against the capability map. It does
//! not run a probe, contact a provider, or inspect any device: it is an
//! evidence checker. Every report is untrusted input, so structural defects,
//! missing modes or hosts, and divergent hashes are reported as failures rather
//! than skipped.
//!
//! The sanitized report contains only package names, relative member paths
//! (through a digest), capability ids, and failure strings. Account labels are
//! used to prove more than one account path ran but are not republished, and no
//! download directory, token, or email address can reach the report.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const REPORT_SCHEMA_VERSION: u32 = 1;
const REPORT_KIND: &str = "apk-lab-acquisition-preflight";
const PROBE_KIND: &str = "apk-lab-play-delivery-probe";
const CAPABILITIES_KIND: &str = "apk-lab-native-play-capability-map";
const DEFAULT_PROOF_DIR: &str = ".apk-lab-migration/play-proof";
const DEFAULT_CAPABILITIES: &str = "tools/apk-lab/migration/capabilities.json";
const DEFAULT_APPROVALS: &str = ".apk-lab-migration/acquisition-approvals.json";

const REQUIRED_MODES: [&str; 3] = ["aas", "oauth", "dispenser"];
const MACOS_AARCH64: (&str, &str) = ("macos", "aarch64");
const LINUX_X86_64: (&str, &str) = ("linux", "x86_64");

/// Capability rows these delivery reports can actually prove. A row outside
/// this set (notably `provider-failure-fallback`, whose APKMirror path is an
/// external anti-bot exception) is unproven unless an explicit approval exists.
const PROVEN_CAPABILITIES: &[&str] = &[
    "latest-delivery",
    "version-name",
    "version-code-and-precedence",
    "base-and-splits",
    "direct-aas",
    "oauth-to-aas",
    "device-locale-timezone",
    "dispenser",
    "enabled-account-rotation",
    "wrong-version-rejection",
    "host-coverage",
    "secret-redaction",
];

struct Options {
    proof_dir: PathBuf,
    capabilities: PathBuf,
    approvals: PathBuf,
    report: Option<PathBuf>,
    check: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    kind: String,
    schema_version: u32,
    package: String,
    account_label: String,
    mode: String,
    requested_version_code: Option<i64>,
    observed: VersionInfo,
    host: HostInfo,
    files: Vec<FileEntry>,
    check: CheckOutcome,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfo {
    version_name: String,
    version_code: u64,
}

#[derive(Deserialize)]
struct HostInfo {
    os: String,
    arch: String,
}

#[derive(Deserialize)]
struct FileEntry {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckOutcome {
    ok: bool,
    base_apk: bool,
    file_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesFile {
    schema_version: u32,
    kind: String,
    rows: Vec<CapabilityRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityRow {
    id: String,
    mandatory: bool,
    implementation_route: String,
    /// A blocked row may explicitly record `null` while no independent
    /// baseline case exists; the key itself is still required.
    baseline_case_id: Value,
    live_proof: LiveProof,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveProof {
    required: bool,
    resource_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalsFile {
    schema_version: u32,
    approvals: Vec<Approval>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Approval {
    capability_id: String,
    reason: String,
    approved_by: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    kind: &'static str,
    check: bool,
    passed: bool,
    proof_count: usize,
    proofs: Vec<ProofSummary>,
    coverage: Coverage,
    capabilities: CapabilitiesSummary,
    failures: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProofSummary {
    file: String,
    package: String,
    mode: String,
    version_name: String,
    version_code: u64,
    host: String,
    files: usize,
    sha256: String,
}

#[derive(Serialize)]
struct Coverage {
    #[serde(rename = "macosAarch64")]
    macos_aarch64: bool,
    #[serde(rename = "linuxX86_64")]
    linux_x86_64: bool,
    modes: Vec<String>,
}

#[derive(Serialize)]
struct CapabilitiesSummary {
    total: usize,
    proven: usize,
    approved: usize,
    unproven: Vec<String>,
}

type VersionKey = (String, String, u64);
type MemberMap = BTreeMap<String, String>;
type MemberGroups = BTreeMap<VersionKey, (String, MemberMap)>;

struct LoadedProof {
    file: String,
    report: ProbeReport,
}

struct ProofOutcome {
    summaries: Vec<ProofSummary>,
    hosts: BTreeSet<(String, String)>,
    modes: BTreeSet<String>,
}

/// Entry point for `apk-lab-migrate preflight acquisition ...`.
pub fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_args(args)? else {
        print_help();
        return Ok(());
    };

    let report = evaluate(&options)?;
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialize the preflight report: {error}"))?;

    match options.report.as_deref() {
        Some(path) if path.as_os_str() != "-" => fs::write(path, format!("{json}\n"))
            .map_err(|error| format!("could not write report '{}': {error}", path.display()))?,
        _ => println!("{json}"),
    }

    if options.check && !report.passed {
        return Err(format!(
            "acquisition preflight failed: {} failure(s)",
            report.failures.len()
        ));
    }
    Ok(())
}

fn print_help() {
    println!(
        "Usage:\n\
         \x20 apk-lab-migrate preflight acquisition [options]\n\
         \n\
         Validates every native Play delivery probe report in the proof directory,\n\
         requires aas/oauth/dispenser and macos/aarch64 + linux/x86_64 coverage,\n\
         rejects divergent package/version member hashes, and cross-checks the\n\
         native capability map. Mandatory rows without live proof must be named in\n\
         the approvals file. A sanitized JSON summary is written to stdout or\n\
         --report; --check exits nonzero when any requirement fails.\n\
         \n\
         --proof-dir DIR      probe reports (default .apk-lab-migration/play-proof)\n\
         --capabilities PATH  capability map (default tools/apk-lab/migration/capabilities.json)\n\
         --approvals PATH     approvals (default .apk-lab-migration/acquisition-approvals.json)\n\
         --check              exit nonzero on any failed requirement\n\
         --report PATH        write the sanitized report ('-' for stdout)"
    );
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut proof_dir: Option<PathBuf> = None;
    let mut capabilities: Option<PathBuf> = None;
    let mut approvals: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut check = false;

    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--check" => {
                if check {
                    return Err("--check specified more than once".to_string());
                }
                check = true;
            }
            "--proof-dir" => {
                let value = take_value(args, &mut index, "--proof-dir")?;
                set_once(&mut proof_dir, &value, "--proof-dir")?;
            }
            "--capabilities" => {
                let value = take_value(args, &mut index, "--capabilities")?;
                set_once(&mut capabilities, &value, "--capabilities")?;
            }
            "--approvals" => {
                let value = take_value(args, &mut index, "--approvals")?;
                set_once(&mut approvals, &value, "--approvals")?;
            }
            "--report" => {
                let value = take_value(args, &mut index, "--report")?;
                set_once(&mut report, &value, "--report")?;
            }
            _ if arg.starts_with("--proof-dir=") => {
                set_once(&mut proof_dir, value_of(arg, "--proof-dir"), "--proof-dir")?;
            }
            _ if arg.starts_with("--capabilities=") => {
                set_once(
                    &mut capabilities,
                    value_of(arg, "--capabilities"),
                    "--capabilities",
                )?;
            }
            _ if arg.starts_with("--approvals=") => {
                set_once(&mut approvals, value_of(arg, "--approvals"), "--approvals")?;
            }
            _ if arg.starts_with("--report=") => {
                set_once(&mut report, value_of(arg, "--report"), "--report")?;
            }
            _ => return Err(format!("unexpected preflight acquisition argument '{arg}'")),
        }
        index += 1;
    }

    Ok(Some(Options {
        proof_dir: proof_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_PROOF_DIR)),
        capabilities: capabilities.unwrap_or_else(|| PathBuf::from(DEFAULT_CAPABILITIES)),
        approvals: approvals.unwrap_or_else(|| PathBuf::from(DEFAULT_APPROVALS)),
        report,
        check,
    }))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn value_of<'a>(arg: &'a str, flag: &str) -> &'a str {
    arg.strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix('='))
        .unwrap_or("")
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

fn evaluate(options: &Options) -> Result<Report, String> {
    let proofs = load_proofs(&options.proof_dir)?;
    let capabilities = load_capabilities(&options.capabilities)?;
    let approvals = load_approvals(&options.approvals)?;

    let mut failures = Vec::new();
    let outcome = validate_proofs(&proofs, &mut failures);
    let proof_ok = failures.is_empty();
    let capabilities = validate_capabilities(&capabilities, &approvals, proof_ok, &mut failures);

    failures.sort();

    Ok(Report {
        schema_version: REPORT_SCHEMA_VERSION,
        kind: REPORT_KIND,
        check: options.check,
        passed: failures.is_empty(),
        proof_count: proofs.len(),
        proofs: outcome.summaries,
        coverage: Coverage {
            macos_aarch64: outcome
                .hosts
                .contains(&(MACOS_AARCH64.0.to_string(), MACOS_AARCH64.1.to_string())),
            linux_x86_64: outcome
                .hosts
                .contains(&(LINUX_X86_64.0.to_string(), LINUX_X86_64.1.to_string())),
            modes: outcome.modes.into_iter().collect(),
        },
        capabilities,
        failures,
    })
}

fn load_proofs(directory: &Path) -> Result<Vec<LoadedProof>, String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "could not read proof directory '{}': {error}",
            directory.display()
        )
    })?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not read proof directory '{}': {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut proofs = Vec::with_capacity(paths.len());
    for path in paths {
        let file = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("proof report '{}' has no usable file name", path.display()))?
            .to_string();
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("could not read proof report '{file}': {error}"))?;
        let report: ProbeReport = serde_json::from_str(&raw)
            .map_err(|error| format!("proof report '{file}' is malformed: {error}"))?;
        proofs.push(LoadedProof { file, report });
    }
    Ok(proofs)
}

fn load_capabilities(path: &Path) -> Result<CapabilitiesFile, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("could not read capabilities '{}': {error}", path.display()))?;
    let capabilities: CapabilitiesFile = serde_json::from_str(&raw)
        .map_err(|error| format!("capabilities document is malformed: {error}"))?;
    if capabilities.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "capabilities schemaVersion is {}, expected 1",
            capabilities.schema_version
        ));
    }
    if capabilities.kind != CAPABILITIES_KIND {
        return Err(format!(
            "capabilities kind is '{}', expected '{CAPABILITIES_KIND}'",
            capabilities.kind
        ));
    }
    Ok(capabilities)
}

fn load_approvals(path: &Path) -> Result<Vec<Approval>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "could not read approvals '{}': {error}",
                path.display()
            ))
        }
    };
    let approvals: ApprovalsFile = serde_json::from_str(&raw)
        .map_err(|error| format!("approvals document is malformed: {error}"))?;
    if approvals.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "approvals schemaVersion is {}, expected 1",
            approvals.schema_version
        ));
    }
    Ok(approvals.approvals)
}

fn validate_proofs(proofs: &[LoadedProof], failures: &mut Vec<String>) -> ProofOutcome {
    let mut summaries = Vec::with_capacity(proofs.len());
    let mut hosts = BTreeSet::new();
    let mut modes = BTreeSet::new();
    let mut requested_version_match = false;

    if proofs.is_empty() {
        failures.push("proof directory contains no *.json reports".to_string());
    }

    for proof in proofs {
        let report = &proof.report;
        let file = &proof.file;

        if report.kind != PROBE_KIND {
            failures.push(format!(
                "{file}: kind is '{}', expected '{PROBE_KIND}'",
                report.kind
            ));
        }
        if report.schema_version != REPORT_SCHEMA_VERSION {
            failures.push(format!(
                "{file}: schemaVersion is {}, expected 1",
                report.schema_version
            ));
        }
        if report.package.trim().is_empty() {
            failures.push(format!("{file}: package is empty"));
        }
        if report.account_label.trim().is_empty() {
            failures.push(format!("{file}: accountLabel is empty"));
        }
        let mode_valid = REQUIRED_MODES.contains(&report.mode.as_str());
        if !mode_valid {
            failures.push(format!(
                "{file}: mode is '{}', expected aas, oauth, or dispenser",
                report.mode
            ));
        }
        if report.observed.version_name.trim().is_empty() {
            failures.push(format!("{file}: observed versionName is empty"));
        }
        if report.observed.version_code == 0 {
            failures.push(format!("{file}: observed versionCode is not positive"));
        }
        if report.host.os.trim().is_empty() {
            failures.push(format!("{file}: host os is empty"));
        }
        if report.host.arch.trim().is_empty() {
            failures.push(format!("{file}: host arch is empty"));
        }
        if !report.check.ok {
            failures.push(format!("{file}: check.ok is false"));
        }
        if !report.check.base_apk {
            failures.push(format!("{file}: check.baseApk is false"));
        }
        if report.check.file_count < 2 {
            failures.push(format!(
                "{file}: check.fileCount is {}, expected at least 2",
                report.check.file_count
            ));
        }
        if report.check.file_count != report.files.len() {
            failures.push(format!(
                "{file}: check.fileCount {} does not match {} file entries",
                report.check.file_count,
                report.files.len()
            ));
        }

        let mut has_apk = false;
        let mut seen_paths = BTreeSet::new();
        for entry in &report.files {
            if !is_relative_path(&entry.path) {
                failures.push(format!(
                    "{file}: file path '{}' is not a clean relative path",
                    entry.path
                ));
            }
            if !seen_paths.insert(entry.path.as_str()) {
                failures.push(format!("{file}: duplicate file path '{}'", entry.path));
            }
            if entry.size == 0 {
                failures.push(format!(
                    "{file}: file '{}' has a non-positive size",
                    entry.path
                ));
            }
            if !is_sha256(&entry.sha256) {
                failures.push(format!(
                    "{file}: file '{}' does not have a 64-hex sha256",
                    entry.path
                ));
            }
            if entry.path.ends_with(".apk") {
                has_apk = true;
            }
        }
        if !has_apk {
            failures.push(format!("{file}: no .apk member is present"));
        }

        if mode_valid {
            modes.insert(report.mode.clone());
        }
        if !report.host.os.trim().is_empty() && !report.host.arch.trim().is_empty() {
            hosts.insert((
                report.host.os.trim().to_ascii_lowercase(),
                report.host.arch.trim().to_ascii_lowercase(),
            ));
        }
        if report.mode == "aas"
            && report
                .requested_version_code
                .and_then(|requested| u64::try_from(requested).ok())
                == Some(report.observed.version_code)
        {
            requested_version_match = true;
        }

        let map = member_map(report);
        summaries.push(ProofSummary {
            file: file.clone(),
            package: report.package.clone(),
            mode: report.mode.clone(),
            version_name: report.observed.version_name.clone(),
            version_code: report.observed.version_code,
            host: format!("{}/{}", report.host.os, report.host.arch),
            files: report.files.len(),
            sha256: digest_members(&map),
        });
    }

    for mode in REQUIRED_MODES {
        if !modes.contains(mode) {
            failures.push(format!("no '{mode}' proof report"));
        }
    }
    if !hosts.contains(&(MACOS_AARCH64.0.to_string(), MACOS_AARCH64.1.to_string())) {
        failures.push("no macos/aarch64 proof report".to_string());
    }
    if !hosts.contains(&(LINUX_X86_64.0.to_string(), LINUX_X86_64.1.to_string())) {
        failures.push("no linux/x86_64 proof report".to_string());
    }
    if !requested_version_match {
        failures.push("no aas proof report requested the observed version code".to_string());
    }

    check_member_consistency(proofs, failures);

    ProofOutcome {
        summaries,
        hosts,
        modes,
    }
}

/// Every report for the same package and observed version must describe the
/// same set of members with the same hashes. A divergence is reported against
/// the first differing path in sorted order.
fn check_member_consistency(proofs: &[LoadedProof], failures: &mut Vec<String>) {
    let mut groups: MemberGroups = BTreeMap::new();

    for proof in proofs {
        let report = &proof.report;
        if report.package.trim().is_empty() {
            continue;
        }
        let key = (
            report.package.clone(),
            report.observed.version_name.clone(),
            report.observed.version_code,
        );
        let map = member_map(report);
        match groups.get_mut(&key) {
            None => {
                groups.insert(key, (proof.file.clone(), map));
            }
            Some((first_file, first_map)) => {
                if *first_map != map {
                    let path = first_differing_path(first_map, &map)
                        .unwrap_or_else(|| "<unknown>".to_string());
                    failures.push(format!(
                        "{} {} {}: member '{}' differs between '{}' and '{}'",
                        key.0, key.1, key.2, path, first_file, proof.file
                    ));
                }
            }
        }
    }
}

fn member_map(report: &ProbeReport) -> MemberMap {
    report
        .files
        .iter()
        .map(|entry| (entry.path.clone(), entry.sha256.clone()))
        .collect()
}

fn first_differing_path(left: &MemberMap, right: &MemberMap) -> Option<String> {
    let paths: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    paths
        .into_iter()
        .find(|path| left.get(*path) != right.get(*path))
        .cloned()
}

fn digest_members(files: &MemberMap) -> String {
    let mut hasher = Sha256::new();
    for (path, sha256) in files {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(sha256.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn is_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_capabilities(
    capabilities: &CapabilitiesFile,
    approvals: &[Approval],
    proof_ok: bool,
    failures: &mut Vec<String>,
) -> CapabilitiesSummary {
    let mut ids = BTreeSet::new();
    let mut rows: BTreeMap<&str, &CapabilityRow> = BTreeMap::new();

    for (index, row) in capabilities.rows.iter().enumerate() {
        if row.id.trim().is_empty() {
            failures.push(format!("capability row {} has an empty id", index + 1));
            continue;
        }
        if !ids.insert(row.id.as_str()) {
            failures.push(format!("duplicate capability id '{}'", row.id));
        } else {
            rows.insert(row.id.as_str(), row);
        }
        if row.implementation_route.trim().is_empty() {
            failures.push(format!(
                "capability '{}' has an empty implementationRoute",
                row.id
            ));
        }
        match &row.baseline_case_id {
            Value::String(value) if value.trim().is_empty() => failures.push(format!(
                "capability '{}' has an empty baselineCaseId",
                row.id
            )),
            Value::String(_) | Value::Null => {}
            _ => failures.push(format!(
                "capability '{}' has a non-string baselineCaseId",
                row.id
            )),
        }
        if row.status.trim().is_empty() {
            failures.push(format!("capability '{}' has an empty status", row.id));
        }
        if row.live_proof.required && row.live_proof.resource_ids.is_empty() {
            failures.push(format!(
                "capability '{}' requires live proof but declares no resourceIds",
                row.id
            ));
        }
        for resource in &row.live_proof.resource_ids {
            if resource.trim().is_empty() {
                failures.push(format!(
                    "capability '{}' has an empty liveProof resource id",
                    row.id
                ));
            }
        }
    }

    let mut approved = BTreeSet::new();
    for approval in approvals {
        let id = approval.capability_id.trim();
        if id.is_empty() {
            failures.push("approval has an empty capabilityId".to_string());
            continue;
        }
        if !ids.contains(id) {
            failures.push(format!("approval references unknown capability '{id}'"));
            continue;
        }
        if approval.reason.trim().is_empty() {
            failures.push(format!("approval for '{id}' has an empty reason"));
            continue;
        }
        if approval.approved_by.trim().is_empty() {
            failures.push(format!("approval for '{id}' has an empty approvedBy"));
            continue;
        }
        if !approved.insert(id) {
            failures.push(format!("duplicate approval for capability '{id}'"));
        }
    }

    let proven: BTreeSet<&str> = if proof_ok {
        ids.iter()
            .copied()
            .filter(|id| PROVEN_CAPABILITIES.contains(id))
            .collect()
    } else {
        BTreeSet::new()
    };

    for id in &ids {
        let Some(row) = rows.get(id) else {
            continue;
        };
        if !row.mandatory || proven.contains(*id) || approved.contains(*id) {
            continue;
        }
        failures.push(format!(
            "mandatory capability '{id}' has no live proof and is not approved"
        ));
    }

    let unproven: Vec<String> = ids
        .iter()
        .filter(|id| !proven.contains(*id) && !approved.contains(*id))
        .map(|id| (*id).to_string())
        .collect();
    let approved_count = ids
        .iter()
        .filter(|id| approved.contains(*id) && !proven.contains(*id))
        .count();

    CapabilitiesSummary {
        total: ids.len(),
        proven: proven.len(),
        approved: approved_count,
        unproven,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    const BASE: &str = "com.example.app/com.example.app.apk";
    const SPLIT: &str = "com.example.app/com.example.app.config.arm64_v8a.apk";
    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    const ALL_IDS: [&str; 13] = [
        "latest-delivery",
        "version-name",
        "version-code-and-precedence",
        "base-and-splits",
        "direct-aas",
        "oauth-to-aas",
        "device-locale-timezone",
        "dispenser",
        "enabled-account-rotation",
        "wrong-version-rejection",
        "provider-failure-fallback",
        "host-coverage",
        "secret-redaction",
    ];

    struct Fixture {
        _dir: TempDir,
        proof_dir: PathBuf,
        capabilities: PathBuf,
        approvals: PathBuf,
    }

    fn default_files() -> Vec<(&'static str, u64, &'static str)> {
        vec![(BASE, 1024, SHA_A), (SPLIT, 2048, SHA_B)]
    }

    fn write_probe(
        directory: &Path,
        name: &str,
        mode: &str,
        os: &str,
        arch: &str,
        requested: Option<i64>,
        files: &[(&str, u64, &str)],
    ) {
        let files: Vec<Value> = files
            .iter()
            .map(|(path, size, sha256)| json!({"path": path, "size": size, "sha256": sha256}))
            .collect();
        let report = json!({
            "kind": PROBE_KIND,
            "schemaVersion": 1,
            "package": "com.example.app",
            "accountLabel": "fixture-account",
            "mode": mode,
            "requestedVersionCode": requested,
            "observed": {"versionName": "1.0.0", "versionCode": 42},
            "host": {"os": os, "arch": arch},
            "downloadDir": "/private/secret/download-dir",
            "files": files,
            "check": {
                "ok": true,
                "baseApk": true,
                "fileCount": files.len()
            }
        });
        fs::write(
            directory.join(name),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    fn capability_row(id: &str) -> Value {
        json!({
            "id": id,
            "mandatory": true,
            "implementationRoute": "native",
            "baselineCaseId": "differential:fixture",
            "liveProof": {"required": true, "resourceIds": ["play.fixture"]},
            "status": "blocked"
        })
    }

    fn capabilities_value(rows: Vec<Value>) -> Value {
        json!({
            "schemaVersion": 1,
            "kind": CAPABILITIES_KIND,
            "rows": rows
        })
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let proof_dir = dir.path().join("proof");
        fs::create_dir(&proof_dir).unwrap();
        let capabilities = dir.path().join("capabilities.json");
        let approvals = dir.path().join("approvals.json");

        let rows = ALL_IDS.iter().map(|id| capability_row(id)).collect();
        fs::write(
            &capabilities,
            serde_json::to_string_pretty(&capabilities_value(rows)).unwrap(),
        )
        .unwrap();

        let fixture = Fixture {
            _dir: dir,
            proof_dir,
            capabilities,
            approvals,
        };

        let files = default_files();
        write_probe(
            &fixture.proof_dir,
            "aas.json",
            "aas",
            "macos",
            "aarch64",
            Some(42),
            &files,
        );
        write_probe(
            &fixture.proof_dir,
            "oauth.json",
            "oauth",
            "linux",
            "x86_64",
            None,
            &files,
        );
        write_probe(
            &fixture.proof_dir,
            "dispenser.json",
            "dispenser",
            "macos",
            "aarch64",
            None,
            &files,
        );
        fixture
    }

    fn write_approvals(fixture: &Fixture, approvals: &[(&str, &str, &str)]) {
        let approvals: Vec<Value> = approvals
            .iter()
            .map(|(id, reason, by)| json!({"capabilityId": id, "reason": reason, "approvedBy": by}))
            .collect();
        fs::write(
            &fixture.approvals,
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "approvals": approvals
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn evaluate_fixture(fixture: &Fixture, check: bool) -> Result<Report, String> {
        evaluate(&Options {
            proof_dir: fixture.proof_dir.clone(),
            capabilities: fixture.capabilities.clone(),
            approvals: fixture.approvals.clone(),
            report: None,
            check,
        })
    }

    #[test]
    fn valid_full_set_passes() {
        let fixture = fixture();
        write_approvals(
            &fixture,
            &[(
                "provider-failure-fallback",
                "maintainer-approved external exception",
                "maintainer",
            )],
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(report.passed, "failures: {:?}", report.failures);
        assert!(report.check);
        assert_eq!(report.proof_count, 3);
        assert_eq!(report.capabilities.total, 13);
        assert_eq!(report.capabilities.proven, 12);
        assert_eq!(report.capabilities.approved, 1);
        assert!(report.capabilities.unproven.is_empty());
        assert!(report.coverage.macos_aarch64);
        assert!(report.coverage.linux_x86_64);
        assert_eq!(report.coverage.modes, vec!["aas", "dispenser", "oauth"]);
    }

    #[test]
    fn missing_mode_fails() {
        let fixture = fixture();
        fs::remove_file(fixture.proof_dir.join("dispenser.json")).unwrap();
        write_approvals(
            &fixture,
            &[("provider-failure-fallback", "reason", "maintainer")],
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("no 'dispenser' proof report")));
    }

    #[test]
    fn hash_divergence_fails() {
        let fixture = fixture();
        let divergent = [(BASE, 1024, SHA_A), (SPLIT, 2048, SHA_A)];
        write_probe(
            &fixture.proof_dir,
            "oauth.json",
            "oauth",
            "linux",
            "x86_64",
            None,
            &divergent,
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains(SPLIT) && failure.contains("differs")),
            "failures: {:?}",
            report.failures
        );
    }

    #[test]
    fn malformed_sha_fails() {
        let fixture = fixture();
        let malformed = [(BASE, 1024, "not-a-sha"), (SPLIT, 2048, SHA_B)];
        write_probe(
            &fixture.proof_dir,
            "oauth.json",
            "oauth",
            "linux",
            "x86_64",
            None,
            &malformed,
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("64-hex sha256")));
    }

    #[test]
    fn missing_host_fails() {
        let fixture = fixture();
        let files = default_files();
        write_probe(
            &fixture.proof_dir,
            "oauth.json",
            "oauth",
            "macos",
            "aarch64",
            None,
            &files,
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("no linux/x86_64 proof report")));
        assert!(!report.coverage.linux_x86_64);
    }

    #[test]
    fn duplicate_capability_id_fails() {
        let fixture = fixture();
        let mut rows: Vec<Value> = ALL_IDS.iter().map(|id| capability_row(id)).collect();
        rows.push(capability_row("latest-delivery"));
        fs::write(
            &fixture.capabilities,
            serde_json::to_string(&capabilities_value(rows)).unwrap(),
        )
        .unwrap();

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("duplicate capability id 'latest-delivery'")));
    }

    #[test]
    fn unproven_mandatory_row_fails_without_approval() {
        let fixture = fixture();

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("provider-failure-fallback")));
        assert_eq!(
            report.capabilities.unproven,
            vec!["provider-failure-fallback"]
        );
    }

    #[test]
    fn approved_row_passes() {
        let fixture = fixture();
        write_approvals(
            &fixture,
            &[(
                "provider-failure-fallback",
                "APKMirror anti-bot wall",
                "maintainer",
            )],
        );

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(report.passed, "failures: {:?}", report.failures);
        assert_eq!(report.capabilities.approved, 1);
    }

    #[test]
    fn unknown_approval_fails() {
        let fixture = fixture();
        write_approvals(&fixture, &[("not-a-capability", "reason", "maintainer")]);

        let report = evaluate_fixture(&fixture, true).unwrap();
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("unknown capability 'not-a-capability'")));
    }

    #[test]
    fn report_is_deterministic_and_sanitized() {
        let fixture = fixture();
        write_approvals(
            &fixture,
            &[("provider-failure-fallback", "reason", "maintainer")],
        );

        let first = serde_json::to_string(&evaluate_fixture(&fixture, true).unwrap()).unwrap();
        let second = serde_json::to_string(&evaluate_fixture(&fixture, true).unwrap()).unwrap();
        assert_eq!(first, second);

        let private = fixture._dir.path().to_string_lossy();
        assert!(!first.contains(private.as_ref()));
        assert!(!first.contains("downloadDir"));
        assert!(!first.contains("download-dir"));
        assert!(!first.contains("fixture-account"));
        assert!(!first.contains("email"));
        assert!(!first.contains("token"));
    }
}
