//! Paired benchmark comparison and report verification.
//!
//! `compare` consumes the raw paired samples produced by
//! `bench --engine both`, recomputes every summary from those samples, applies
//! the pre-registered performance policy, and fails when a workload is missing,
//! invalid, inconclusive, or materially slower in Rust.
//!
//! `report-check` independently validates `TO-RUST.md` against the stored raw
//! data, recomputes the comparison summaries, and checks that every chart file
//! maps to stored workload identities and units.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::benchmark::{
    now_epoch_seconds, sha256_bytes, validate_policy, write_json_atomic, BudgetSpec,
    PerformancePolicy, POLICY_SCHEMA_VERSION, RSS_NOISE_FLOOR_BYTES, WALL_NOISE_FLOOR_MS,
};

const COMPARISON_SCHEMA_VERSION: u32 = 1;
const PAIRED_KIND: &str = "apk-lab-paired-benchmark-result";
const COMPARISON_KIND: &str = "apk-lab-benchmark-comparison";
const BOOTSTRAP_RESAMPLES: usize = 2_000;

/// Required report sections. The matcher is case-insensitive and treats any
/// Markdown heading level 2 or deeper as a candidate.
const REQUIRED_SECTIONS: &[&str] = &[
    "goals and non-goals",
    "architecture",
    "migration map",
    "clean-code decisions",
    "provenance",
    "methodology",
    "data and charts",
    "correctness and integration evidence",
    "rejected and inconclusive measurements",
    "performance acceptance verdict",
    "python retirement",
    "operational rollback and rebuild",
    "limitations",
];

const PLACEHOLDERS: &[&str] = &[
    "todo",
    "tbd",
    "fixme",
    "lorem ipsum",
    "{{",
    "}}",
    "<placeholder>",
];

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stats {
    pub(crate) count: usize,
    pub(crate) median: f64,
    pub(crate) p95: f64,
}

pub(crate) fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    let index = ((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

pub(crate) fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let count = sorted.len();
    let median = if count % 2 == 1 {
        sorted[count / 2]
    } else {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    };
    Some(Stats {
        count,
        median,
        p95: percentile(&sorted, 0.95),
    })
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let count = values.len();
    if count % 2 == 1 {
        values[count / 2]
    } else {
        (values[count / 2 - 1] + values[count / 2]) / 2.0
    }
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

    fn index(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Bootstrap the median of paired Rust/Python ratios. `rust` and `python` are
/// matched by pair index. The seed is derived from the policy seed, workload id,
/// and metric name so independent runs reproduce the same interval.
pub(crate) fn bootstrap_median_ratio_interval(
    rust: &[f64],
    python: &[f64],
    resamples: usize,
    seed: u64,
) -> Option<(f64, f64)> {
    if rust.len() != python.len() || rust.is_empty() || resamples == 0 {
        return None;
    }
    let ratios: Vec<f64> = rust
        .iter()
        .zip(python)
        .filter_map(|(rust, python)| {
            if *python > 0.0 && rust.is_finite() && python.is_finite() {
                Some(rust / python)
            } else {
                None
            }
        })
        .collect();
    if ratios.len() != rust.len() {
        return None;
    }
    let mut rng = SplitMix64::new(seed);
    let mut medians = Vec::with_capacity(resamples);
    let mut sample = vec![0.0; ratios.len()];
    for _ in 0..resamples {
        for slot in sample.iter_mut() {
            *slot = ratios[rng.index(ratios.len())];
        }
        medians.push(median(&mut sample));
    }
    medians.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    // Two-sided 95% percentile interval.
    Some((percentile(&medians, 0.025), percentile(&medians, 0.975)))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    Pass,
    MaterialSlowdown,
    Inconclusive,
    ExplanationRequired,
    Unavailable,
}

impl Verdict {
    fn as_str(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::MaterialSlowdown => "material-slowdown",
            Verdict::Inconclusive => "inconclusive",
            Verdict::ExplanationRequired => "explanation-required",
            Verdict::Unavailable => "unavailable",
        }
    }
}

/// Applies the pre-registered policy. A regression must exceed both the ratio
/// budget's lower confidence bound and the absolute floor to be material. An
/// interval that straddles `ratio_max` is inconclusive, not an improvement.
pub(crate) fn classify(
    ratio_point: f64,
    interval: (f64, f64),
    absolute_difference: f64,
    floor: Option<f64>,
    ratio_max: f64,
    blocks_on_worse: bool,
) -> Verdict {
    if !ratio_point.is_finite() || !interval.0.is_finite() || !interval.1.is_finite() {
        return Verdict::Unavailable;
    }
    let above_floor = floor
        .map(|floor| absolute_difference > floor)
        .unwrap_or(true);
    if !above_floor {
        return Verdict::Pass;
    }
    if interval.0 > ratio_max {
        if blocks_on_worse {
            Verdict::MaterialSlowdown
        } else {
            Verdict::ExplanationRequired
        }
    } else if interval.1 > ratio_max {
        if blocks_on_worse {
            Verdict::Inconclusive
        } else {
            Verdict::ExplanationRequired
        }
    } else {
        Verdict::Pass
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CompareOptions {
    input: PathBuf,
    policy: PathBuf,
    out: PathBuf,
}

fn parse_compare_args(args: &[String]) -> Result<CompareOptions, String> {
    let mut input = None;
    let mut policy = None;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        let take = |index: &mut usize| -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for '{flag}'"))
        };
        match flag {
            "--input" => input = Some(PathBuf::from(take(&mut index)?)),
            "--policy" => policy = Some(PathBuf::from(take(&mut index)?)),
            "--out" => out = Some(PathBuf::from(take(&mut index)?)),
            "-h" | "--help" => {
                return Err(
                    "usage: compare --input PAIRED.json --policy performance-policy.json --out COMPARISON.json"
                        .to_string(),
                )
            }
            other => return Err(format!("unknown compare option '{other}'")),
        }
        index += 1;
    }
    Ok(CompareOptions {
        input: input.ok_or_else(|| "compare requires --input".to_string())?,
        policy: policy.ok_or_else(|| "compare requires --policy".to_string())?,
        out: out.ok_or_else(|| "compare requires --out".to_string())?,
    })
}

fn load_policy(path: &Path) -> Result<(PerformancePolicy, String), String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("cannot read performance policy {}: {e}", path.display()))?;
    let policy: PerformancePolicy = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid performance policy {}: {e}", path.display()))?;
    validate_policy(&policy)?;
    Ok((policy, sha256_bytes(&bytes)))
}

fn metric_budget<'a>(
    policy: &'a PerformancePolicy,
    metric: &str,
) -> Result<&'a BudgetSpec, String> {
    policy
        .budgets
        .get(metric)
        .ok_or_else(|| format!("performance policy is missing budgets.{metric}"))
}

fn sample_values(samples: &[Value], field: &str) -> Option<Vec<f64>> {
    let mut values = Vec::new();
    for sample in samples {
        if sample.get("kind").and_then(Value::as_str) != Some("measured") {
            continue;
        }
        let value = sample.get(field).and_then(Value::as_f64)?;
        values.push(value);
    }
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn measured_samples<'a>(engine: &'a Value, id: &str, role: &str) -> Result<Vec<&'a Value>, String> {
    let samples = engine
        .get("samples")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("workload '{id}' engines.{role}.samples must be an array"))?;
    Ok(samples
        .iter()
        .filter(|sample| sample.get("kind").and_then(Value::as_str) == Some("measured"))
        .collect())
}

fn engine_expected(engine: &Value, id: &str, role: &str, key: &str) -> Result<u64, String> {
    engine
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("workload '{id}' engines.{role}.{key} must be an integer"))
}

/// Build one metric comparison from paired points. Values are re-read from the
/// raw samples; no stored summary is trusted.
#[allow(clippy::too_many_arguments)]
fn compare_metric(
    id: &str,
    metric: &str,
    unit: &str,
    paired: &[(f64, f64)],
    budget: &BudgetSpec,
    ratio_max: f64,
    floor: Option<f64>,
    policy_seed: u64,
) -> Value {
    let python: Vec<f64> = paired.iter().map(|(python, _)| *python).collect();
    let rust: Vec<f64> = paired.iter().map(|(_, rust)| *rust).collect();
    let python_stats = stats(&python);
    let rust_stats = stats(&rust);
    let (Some(python_stats), Some(rust_stats)) = (python_stats, rust_stats) else {
        return json!({
            "metric": metric,
            "unit": unit,
            "python": Value::Null,
            "rust": Value::Null,
            "ratio": Value::Null,
            "bootstrap": Value::Null,
            "budget": budget,
            "verdict": Verdict::Unavailable.as_str(),
        });
    };
    let ratio = if python_stats.median > 0.0 {
        rust_stats.median / python_stats.median
    } else {
        f64::INFINITY
    };
    let difference = rust_stats.median - python_stats.median;
    let seed = policy_seed ^ fnv1a(&format!("{id}:{metric}"));
    let interval = bootstrap_median_ratio_interval(&rust, &python, BOOTSTRAP_RESAMPLES, seed)
        .unwrap_or((f64::NAN, f64::NAN));
    let verdict = classify(
        ratio,
        interval,
        difference,
        floor,
        ratio_max,
        budget.block_on_worse.unwrap_or(false),
    );
    json!({
        "metric": metric,
        "unit": unit,
        "python": {
            "count": python_stats.count,
            "median": python_stats.median,
            "p95": python_stats.p95,
        },
        "rust": {
            "count": rust_stats.count,
            "median": rust_stats.median,
            "p95": rust_stats.p95,
        },
        "ratio": ratio,
        "absoluteDifference": difference,
        "bootstrap": {
            "resamples": BOOTSTRAP_RESAMPLES,
            "confidence": budget.bootstrap_confidence.unwrap_or(0.95),
            "seed": seed,
            "interval": {"low": interval.0, "high": interval.1},
        },
        "budget": {
            "ratioMax": ratio_max,
            "absoluteNoiseFloor": floor,
            "blockOnWorse": budget.block_on_worse.unwrap_or(false),
            "requiresExplanation": budget.requires_explanation.unwrap_or(false),
        },
        "verdict": verdict.as_str(),
    })
}

fn compare_workload(
    id: &str,
    workload: &Value,
    policy: &PerformancePolicy,
) -> (Value, Vec<String>) {
    let mut reasons = Vec::new();
    let sample_class = workload
        .get("sampleClass")
        .and_then(Value::as_str)
        .unwrap_or("local");
    let required = policy
        .sample_classes
        .get(sample_class)
        .cloned()
        .unwrap_or_else(|| {
            reasons.push(format!(
                "workload '{id}' has unknown sample class '{sample_class}'"
            ));
            super::benchmark::SampleClass {
                warmups: 0,
                samples: 0,
            }
        });
    let status = workload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("incomplete");
    if status != "measured-paired" {
        reasons.push(format!(
            "workload '{id}' status is '{status}', expected 'measured-paired'"
        ));
    }

    let engines = workload.get("engines").and_then(Value::as_object);
    let Some(engines) = engines else {
        reasons.push(format!("workload '{id}' has no engines object"));
        return (
            json!({"id": id, "status": "invalid", "reasons": reasons}),
            reasons,
        );
    };
    let (Some(python_engine), Some(rust_engine)) = (engines.get("python"), engines.get("rust"))
    else {
        reasons.push(format!(
            "workload '{id}' is missing a python or rust engine"
        ));
        return (
            json!({"id": id, "status": "invalid", "reasons": reasons}),
            reasons,
        );
    };

    let expected_samples = engine_expected(python_engine, id, "python", "samplesExpected")
        .and_then(|value| {
            engine_expected(rust_engine, id, "rust", "samplesExpected").map(|_| value)
        })
        .unwrap_or(0);
    if expected_samples < required.samples as u64 {
        reasons.push(format!(
            "workload '{id}' expects {expected_samples} samples, below the pre-registered minimum {}",
            required.samples
        ));
    }
    let expected_warmups =
        engine_expected(python_engine, id, "python", "warmupsExpected").unwrap_or(0);
    if expected_warmups < required.warmups as u64 {
        reasons.push(format!(
            "workload '{id}' expects {expected_warmups} warmups, below the pre-registered minimum {}",
            required.warmups
        ));
    }

    let python_samples = match measured_samples(python_engine, id, "python") {
        Ok(samples) => samples,
        Err(error) => {
            reasons.push(error);
            Vec::new()
        }
    };
    let rust_samples = match measured_samples(rust_engine, id, "rust") {
        Ok(samples) => samples,
        Err(error) => {
            reasons.push(error);
            Vec::new()
        }
    };
    let python_completed = engine_expected(python_engine, id, "python", "samplesCompleted")
        .unwrap_or(python_samples.len() as u64);
    let rust_completed = engine_expected(rust_engine, id, "rust", "samplesCompleted")
        .unwrap_or(rust_samples.len() as u64);
    let warmups_python =
        engine_expected(python_engine, id, "python", "warmupsCompleted").unwrap_or(0);
    let warmups_rust = engine_expected(rust_engine, id, "rust", "warmupsCompleted").unwrap_or(0);
    if python_samples.len() < expected_samples as usize
        || rust_samples.len() < expected_samples as usize
    {
        reasons.push(format!(
            "workload '{id}' has {} python and {} rust measured samples, expected {expected_samples}",
            python_samples.len(),
            rust_samples.len()
        ));
    }
    if python_completed < expected_samples || rust_completed < expected_samples {
        reasons.push(format!(
            "workload '{id}' engine sample counters are {python_completed}/{rust_completed}, expected {expected_samples}"
        ));
    }
    if warmups_python < expected_warmups || warmups_rust < expected_warmups {
        reasons.push(format!(
            "workload '{id}' warmup counters are {warmups_python}/{warmups_rust}, expected {expected_warmups}"
        ));
    }
    for (role, samples) in [("python", &python_samples), ("rust", &rust_samples)] {
        for sample in samples.iter() {
            if sample.get("outputValid").and_then(Value::as_bool) != Some(true) {
                let index = sample.get("index").and_then(Value::as_u64).unwrap_or(0);
                reasons.push(format!(
                    "workload '{id}' {role} sample {index} failed output validation"
                ));
            }
        }
    }

    // Verify the explicit pairing record and per-sample cross-engine checks.
    let pairs = workload
        .get("pairs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if pairs.len() < expected_samples as usize {
        reasons.push(format!(
            "workload '{id}' has {} recorded pairs, expected {expected_samples}",
            pairs.len()
        ));
    }
    for pair in &pairs {
        let index = pair.get("index").and_then(Value::as_u64).unwrap_or(0);
        if pair.get("correct").and_then(Value::as_bool) != Some(true) {
            reasons.push(format!("workload '{id}' pair {index} is not correct"));
        }
        for field in ["pythonOutputValid", "rustOutputValid"] {
            if pair.get(field).and_then(Value::as_bool) != Some(true) {
                reasons.push(format!("workload '{id}' pair {index} {field} is not true"));
            }
        }
        for field in ["exitCodesMatch", "stdoutSha256Match", "stderrSha256Match"] {
            if pair.get(field).and_then(Value::as_bool) != Some(true) {
                reasons.push(format!("workload '{id}' pair {index} {field} is not true"));
            }
        }
    }
    if workload
        .get("correctness")
        .and_then(|value| value.get("matched"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        reasons.push(format!("workload '{id}' correctness.matched is not true"));
    }

    let mut metrics = Map::new();
    let mut add_metric = |name: &str,
                          field: &str,
                          unit: &str,
                          floor: Option<f64>,
                          reasons: &mut Vec<String>| {
        let python_values = sample_values(
            &python_samples
                .iter()
                .map(|v| (*v).clone())
                .collect::<Vec<_>>(),
            field,
        );
        let rust_values = sample_values(
            &rust_samples
                .iter()
                .map(|v| (*v).clone())
                .collect::<Vec<_>>(),
            field,
        );
        let (Some(python_values), Some(rust_values)) = (python_values, rust_values) else {
            let budget = metric_budget(policy, name);
            let blocking = budget
                .as_ref()
                .map(|budget| budget.block_on_worse.unwrap_or(false))
                .unwrap_or(false);
            if blocking {
                reasons.push(format!(
                    "workload '{id}' has no complete {name} samples; a blocking metric may not stay null"
                ));
            }
            let budget_value = budget
                .map(|budget| serde_json::to_value(budget).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            metrics.insert(
                name.to_string(),
                json!({
                    "metric": name,
                    "unit": unit,
                    "python": Value::Null,
                    "rust": Value::Null,
                    "ratio": Value::Null,
                    "bootstrap": Value::Null,
                    "budget": budget_value,
                    "verdict": Verdict::Unavailable.as_str(),
                    "reason": "one or both engines reported null values for every measured sample",
                }),
            );
            return;
        };
        if python_values.len() != rust_values.len() {
            reasons.push(format!(
                "workload '{id}' {name} sample counts differ: {} python, {} rust",
                python_values.len(),
                rust_values.len()
            ));
        }
        let paired: Vec<(f64, f64)> = python_values
            .iter()
            .zip(&rust_values)
            .map(|(python, rust)| (*python, *rust))
            .collect();
        let budget = match metric_budget(policy, name) {
            Ok(budget) => budget,
            Err(error) => {
                reasons.push(error);
                return;
            }
        };
        let ratio_max = budget.ratio_max.unwrap_or(f64::INFINITY);
        let value = compare_metric(
            id,
            name,
            unit,
            &paired,
            budget,
            ratio_max,
            floor,
            policy.seed,
        );
        let verdict = value
            .get("verdict")
            .and_then(Value::as_str)
            .unwrap_or("unavailable");
        if budget.block_on_worse.unwrap_or(false)
            && matches!(
                verdict,
                "material-slowdown" | "inconclusive" | "unavailable"
            )
        {
            reasons.push(format!(
                "workload '{id}' {name} verdict is '{verdict}' under the pre-registered budget"
            ));
        }
        metrics.insert(name.to_string(), value);
    };

    // Wall time is always mandatory; the policy fixes blockOnWorse true.
    add_metric(
        "wallTime",
        "wallMs",
        "milliseconds",
        Some(
            policy
                .budgets
                .get("wallTime")
                .and_then(|budget| budget.absolute_noise_floor_ms)
                .unwrap_or(WALL_NOISE_FLOOR_MS),
        ),
        &mut reasons,
    );
    if policy.budgets.contains_key("cpu") {
        add_metric("cpu", "cpuMs", "milliseconds", None, &mut reasons);
    }
    if policy.budgets.contains_key("rss") {
        add_metric(
            "rss",
            "peakChildRssBytes",
            "bytes",
            Some(
                policy
                    .budgets
                    .get("rss")
                    .and_then(|budget| budget.absolute_noise_floor_bytes)
                    .unwrap_or(RSS_NOISE_FLOOR_BYTES) as f64,
            ),
            &mut reasons,
        );
    }

    let verdict = if reasons.is_empty() { "pass" } else { "fail" };
    let value = json!({
        "id": id,
        "category": workload.get("category"),
        "size": workload.get("size"),
        "costly": workload.get("costly"),
        "sampleClass": sample_class,
        "measuredPairs": pairs.len(),
        "metrics": metrics,
        "verdict": verdict,
        "reasons": reasons.clone(),
    });
    (value, reasons)
}

/// Runs `compare`.
pub fn run(args: &[String]) -> Result<(), String> {
    let opts = parse_compare_args(args)?;
    let (policy, policy_sha256) = load_policy(&opts.policy)?;
    let input_bytes = fs::read(&opts.input)
        .map_err(|e| format!("cannot read paired benchmark {}: {e}", opts.input.display()))?;
    let input: Value = serde_json::from_slice(&input_bytes)
        .map_err(|e| format!("invalid paired benchmark {}: {e}", opts.input.display()))?;
    let object = input
        .as_object()
        .ok_or_else(|| "paired benchmark must be a JSON object".to_string())?;
    if object.get("kind").and_then(Value::as_str) != Some(PAIRED_KIND) {
        return Err(format!(
            "paired benchmark kind must be '{PAIRED_KIND}', got {:?}",
            object.get("kind")
        ));
    }
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("paired benchmark schemaVersion must be 1".to_string());
    }
    let input_policy = object
        .get("policy")
        .and_then(Value::as_object)
        .ok_or_else(|| "paired benchmark policy must be an object".to_string())?;
    if input_policy.get("id").and_then(Value::as_str) != Some(policy.id.as_str()) {
        return Err(format!(
            "paired benchmark policy id {:?} does not match {}",
            input_policy.get("id"),
            policy.id
        ));
    }
    if input_policy.get("sha256").and_then(Value::as_str) != Some(policy_sha256.as_str()) {
        return Err(format!(
            "paired benchmark policy sha256 does not match {}",
            opts.policy.display()
        ));
    }
    if input_policy
        .get("requiredWorkloads")
        .and_then(Value::as_array)
        .map(|value| value.len())
        .unwrap_or(0)
        != policy.required_workloads.len()
    {
        return Err("paired benchmark requiredWorkloads do not match the policy".to_string());
    }
    let engines = object
        .get("engines")
        .and_then(Value::as_object)
        .ok_or_else(|| "paired benchmark engines must be an object".to_string())?;
    for role in ["python", "rust"] {
        let engine = engines
            .get(role)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("paired benchmark engines.{role} must be an object"))?;
        if engine.get("directExecution").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "paired benchmark engines.{role} must execute directly"
            ));
        }
    }
    if engines
        .get("rust")
        .and_then(|rust| rust.get("compilationIncluded"))
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(
            "paired benchmark rust engine must not include compilation in timed samples"
                .to_string(),
        );
    }

    let workloads = object
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "paired benchmark workloads must be an array".to_string())?;
    let mut reasons = Vec::new();
    if object
        .get("acceptance")
        .and_then(|value| value.get("valid"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        let detail = object
            .get("acceptance")
            .and_then(|value| value.get("reasons"))
            .and_then(Value::as_array)
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        reasons.push(format!(
            "paired benchmark did not report valid acceptance: {detail}"
        ));
    }
    if object
        .get("corpus")
        .and_then(|corpus| corpus.get("identitySha256"))
        .and_then(Value::as_str)
        .map(|value| value.len() == 64)
        != Some(true)
    {
        reasons.push("paired benchmark is missing a corpus identity sha256".to_string());
    }
    if !object.get("source").map(Value::is_object).unwrap_or(false) {
        reasons.push("paired benchmark is missing source provenance".to_string());
    }
    if object
        .get("provenance")
        .and_then(|value| value.get("toolchain"))
        .is_none()
    {
        reasons.push("paired benchmark is missing toolchain provenance".to_string());
    }
    let paired_measurements = object.get("measurements").and_then(Value::as_object);
    for metric in ["wall", "cpu", "descendantRss", "io"] {
        let Some(measurement) = paired_measurements.and_then(|value| value.get(metric)) else {
            reasons.push(format!(
                "paired benchmark is missing measurements.{metric} units/method"
            ));
            continue;
        };
        if measurement
            .get("supported")
            .and_then(Value::as_bool)
            .is_none()
        {
            reasons.push(format!(
                "paired benchmark measurements.{metric} is missing supported"
            ));
        }
        if measurement.get("supported").and_then(Value::as_bool) == Some(true)
            && measurement.get("unit").and_then(Value::as_str).is_none()
        {
            reasons.push(format!(
                "paired benchmark measurements.{metric} is supported without a unit"
            ));
        }
        if measurement.get("supported").and_then(Value::as_bool) == Some(false)
            && measurement.get("reason").and_then(Value::as_str).is_none()
        {
            reasons.push(format!(
                "paired benchmark measurements.{metric} is unsupported without a null reason"
            ));
        }
    }
    let present: BTreeSet<String> = workloads
        .iter()
        .filter_map(|workload| {
            workload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    for required in &policy.required_workloads {
        if !present.contains(required) {
            reasons.push(format!(
                "required workload '{required}' is missing from the paired results"
            ));
        }
    }
    let mut comparisons = Vec::with_capacity(workloads.len());
    for workload in workloads {
        let Some(id) = workload.get("id").and_then(Value::as_str) else {
            reasons.push("a paired workload is missing its id".to_string());
            continue;
        };
        let (comparison, mut workload_reasons) = compare_workload(id, workload, &policy);
        reasons.append(&mut workload_reasons);
        comparisons.push(comparison);
    }

    let acceptance = json!({
        "valid": reasons.is_empty(),
        "reasons": reasons,
    });
    let report = json!({
        "schemaVersion": COMPARISON_SCHEMA_VERSION,
        "kind": COMPARISON_KIND,
        "status": if reasons.is_empty() { "pass" } else { "fail" },
        "generatedAtEpochSeconds": now_epoch_seconds(),
        "input": {
            "kind": PAIRED_KIND,
            "sha256": sha256_bytes(&input_bytes),
        },
        "policy": {
            "schemaVersion": POLICY_SCHEMA_VERSION,
            "id": policy.id,
            "sha256": policy_sha256,
            "seed": policy.seed,
            "cachePolicy": policy.cache_policy,
            "order": policy.order.dual_engine,
        },
        "method": {
            "ratio": "rust median / python median; identical workloads, inputs, argv, options, jobs, and sandbox layout",
            "bootstrap": {
                "resamples": BOOTSTRAP_RESAMPLES,
                "confidence": 0.95,
                "statistic": "median of paired rust/python ratios",
                "seed": "policy seed XOR workload id/metric FNV-1a",
            },
            "noiseBudget": {
                "ratioMax": 1.05,
                "wallFloorMs": WALL_NOISE_FLOOR_MS,
                "rssFloorBytes": RSS_NOISE_FLOOR_BYTES,
            },
            "blockingRules": "A lower bound above ratioMax and an absolute difference above the floor blocks. An interval crossing ratioMax is inconclusive and blocks. Non-blocking metrics require explanation.",
        },
        "workloads": comparisons,
        "acceptance": acceptance,
    });
    let acceptance_valid = report["acceptance"]["valid"].as_bool().unwrap_or(false);
    write_json_atomic(&opts.out, &report)?;
    if acceptance_valid {
        Ok(())
    } else {
        let message = report["acceptance"]["reasons"]
            .as_array()
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        Err(format!("comparison failed: {message}"))
    }
}

// ---------------------------------------------------------------------------
// report-check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ReportCheckOptions {
    report: PathBuf,
    data_dir: PathBuf,
}

fn parse_report_check_args(args: &[String]) -> Result<ReportCheckOptions, String> {
    let mut report = None;
    let mut data_dir = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        let take = |index: &mut usize| -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for '{flag}'"))
        };
        match flag {
            "--report" => report = Some(PathBuf::from(take(&mut index)?)),
            "--data-dir" => data_dir = Some(PathBuf::from(take(&mut index)?)),
            "-h" | "--help" => {
                return Err(
                    "usage: report-check --report TO-RUST.md --data-dir docs/benchmarks/apk-lab"
                        .to_string(),
                )
            }
            other => return Err(format!("unknown report-check option '{other}'")),
        }
        index += 1;
    }
    Ok(ReportCheckOptions {
        report: report.ok_or_else(|| "report-check requires --report".to_string())?,
        data_dir: data_dir.ok_or_else(|| "report-check requires --data-dir".to_string())?,
    })
}

fn strip_code_fences(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            output.push('\n');
            continue;
        }
        if fenced {
            output.push('\n');
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

fn headings(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            if hashes < 2 {
                return None;
            }
            let heading = trimmed[hashes..].trim();
            if heading.is_empty() {
                None
            } else {
                Some(heading.to_lowercase())
            }
        })
        .collect()
}

fn markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find(')') {
            let target = after[..end].trim();
            let target = target.split_whitespace().next().unwrap_or("");
            if !target.is_empty() {
                links.push(target.to_string());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    links
}

fn expected_unit(metric: &str) -> Option<&'static str> {
    match metric {
        "wallTime" | "wall" | "cpu" => Some("milliseconds"),
        "rss" | "descendantRss" | "io" => Some("bytes"),
        _ => None,
    }
}

fn numeric(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(Value::as_f64)
}

fn matches(a: f64, b: f64) -> bool {
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= 1e-9 * scale
}

fn verify_charts(
    report_text: &str,
    data_dir: &Path,
    comparison: &Value,
    workload_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let manifest_path = data_dir.join("charts/manifest.json");
    let bytes = fs::read(&manifest_path)
        .map_err(|e| format!("missing chart manifest {}: {e}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid chart manifest {}: {e}", manifest_path.display()))?;
    let charts = manifest
        .get("charts")
        .and_then(Value::as_array)
        .ok_or_else(|| "chart manifest must contain a charts array".to_string())?;
    if charts.is_empty() {
        return Err("chart manifest must contain at least one chart".to_string());
    }
    let mut checked = Vec::new();
    for chart in charts {
        let file = chart
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| "chart entry is missing file".to_string())?;
        let relative = Path::new(file);
        if relative.is_absolute() || file.contains("..") {
            return Err(format!(
                "chart file '{file}' must be a relative path inside the data dir"
            ));
        }
        let path = data_dir.join(relative);
        if !path.is_file() {
            return Err(format!(
                "chart file '{}' does not exist under {}",
                file,
                data_dir.display()
            ));
        }
        let metadata = fs::metadata(&path).map_err(|e| format!("cannot stat '{file}': {e}"))?;
        if metadata.len() == 0 {
            return Err(format!("chart file '{file}' is empty"));
        }
        let head = fs::read(&path).map_err(|e| format!("cannot read '{file}': {e}"))?;
        let valid_magic = file.ends_with(".png") && head.starts_with(b"\x89PNG\r\n\x1a\n")
            || file.ends_with(".svg")
                && String::from_utf8_lossy(&head[..head.len().min(512)]).contains("<svg");
        if !valid_magic {
            return Err(format!(
                "chart file '{file}' is not a recognizable PNG or SVG"
            ));
        }
        let workloads = chart
            .get("workloads")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("chart '{file}' is missing a workloads array"))?;
        if workloads.is_empty() {
            return Err(format!(
                "chart '{file}' must reference at least one workload"
            ));
        }
        let metric = chart
            .get("metric")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("chart '{file}' is missing metric"))?;
        let Some(unit) = expected_unit(metric) else {
            return Err(format!("chart '{file}' has unknown metric '{metric}'"));
        };
        if chart.get("unit").and_then(Value::as_str) != Some(unit) {
            return Err(format!(
                "chart '{file}' metric '{metric}' must use unit '{unit}'"
            ));
        }
        for workload in workloads {
            let Some(id) = workload.as_str() else {
                return Err(format!("chart '{file}' workload ids must be strings"));
            };
            if !workload_ids.contains(id) {
                return Err(format!(
                    "chart '{file}' references unknown workload id '{id}'"
                ));
            }
            let comparison_workloads = comparison
                .get("workloads")
                .and_then(Value::as_array)
                .ok_or_else(|| "comparison workloads must be an array".to_string())?;
            let stored = comparison_workloads
                .iter()
                .find(|workload| workload.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| {
                    format!("chart '{file}' workload '{id}' is not in the comparison")
                })?;
            let stored_metric = stored
                .pointer(&format!("/metrics/{metric}"))
                .ok_or_else(|| {
                    format!("chart '{file}' metric '{metric}' is not stored for workload '{id}'")
                })?;
            if stored_metric.get("unit").and_then(Value::as_str) != Some(unit) {
                return Err(format!(
                    "chart '{file}' unit '{unit}' does not match stored workload '{id}' metric '{metric}'"
                ));
            }
        }
        if !report_text.contains(file) {
            return Err(format!("report does not reference chart file '{file}'"));
        }
        checked.push(file.to_string());
    }
    Ok(checked)
}

fn verify_comparison_against_paired(
    paired: &Value,
    comparison: &Value,
    paired_sha256: &str,
) -> Result<(), String> {
    if comparison.get("kind").and_then(Value::as_str) != Some(COMPARISON_KIND) {
        return Err(format!("comparison kind must be '{COMPARISON_KIND}'"));
    }
    if comparison.pointer("/input/sha256").and_then(Value::as_str) != Some(paired_sha256) {
        return Err(
            "comparison was not generated from this paired.json (sha256 mismatch)".to_string(),
        );
    }
    if comparison.get("status").and_then(Value::as_str) != Some("pass")
        || comparison
            .pointer("/acceptance/valid")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(
            "stored comparison did not pass; the report cannot claim acceptance".to_string(),
        );
    }
    let paired_workloads = paired
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "paired workloads must be an array".to_string())?;
    let comparisons = comparison
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| "comparison workloads must be an array".to_string())?;
    let paired_by_id: BTreeMap<&str, &Value> = paired_workloads
        .iter()
        .filter_map(|workload| {
            workload
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, workload))
        })
        .collect();
    for stored in comparisons {
        let id = stored
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "comparison workload id is missing".to_string())?;
        let paired_workload = paired_by_id
            .get(id)
            .ok_or_else(|| format!("comparison workload '{id}' is absent from paired.json"))?;
        let python_samples = measured_samples(
            paired_workload
                .get("engines")
                .and_then(|engines| engines.get("python"))
                .ok_or_else(|| format!("workload '{id}' has no python engine"))?,
            id,
            "python",
        )?;
        let rust_samples = measured_samples(
            paired_workload
                .get("engines")
                .and_then(|engines| engines.get("rust"))
                .ok_or_else(|| format!("workload '{id}' has no rust engine"))?,
            id,
            "rust",
        )?;
        for (metric, field) in [
            ("wallTime", "wallMs"),
            ("cpu", "cpuMs"),
            ("rss", "peakChildRssBytes"),
        ] {
            let Some(stored_metric) = stored.pointer(&format!("/metrics/{metric}")) else {
                continue;
            };
            let python_values = sample_values(
                &python_samples
                    .iter()
                    .map(|value| (*value).clone())
                    .collect::<Vec<_>>(),
                field,
            );
            let rust_values = sample_values(
                &rust_samples
                    .iter()
                    .map(|value| (*value).clone())
                    .collect::<Vec<_>>(),
                field,
            );
            match (python_values, rust_values) {
                (Some(python_values), Some(rust_values)) => {
                    let python_stats = stats(&python_values)
                        .ok_or_else(|| format!("workload '{id}' {metric} python stats missing"))?;
                    let rust_stats = stats(&rust_values)
                        .ok_or_else(|| format!("workload '{id}' {metric} rust stats missing"))?;
                    for (engine, actual, expected_prefix) in [
                        ("python", python_stats, "/python"),
                        ("rust", rust_stats, "/rust"),
                    ] {
                        let stored_count =
                            numeric(stored_metric, &format!("{expected_prefix}/count"));
                        let stored_median =
                            numeric(stored_metric, &format!("{expected_prefix}/median"));
                        let stored_p95 = numeric(stored_metric, &format!("{expected_prefix}/p95"));
                        if stored_count != Some(actual.count as f64) {
                            return Err(format!(
                                "workload '{id}' {metric} {engine} count mismatch between comparison and paired samples"
                            ));
                        }
                        if stored_median.map(|value| matches(value, actual.median)) != Some(true)
                            || stored_p95.map(|value| matches(value, actual.p95)) != Some(true)
                        {
                            return Err(format!(
                                "workload '{id}' {metric} {engine} median/p95 do not match recomputed paired samples"
                            ));
                        }
                    }
                    let ratio = rust_stats.median / python_stats.median;
                    if numeric(stored_metric, "/ratio").map(|value| matches(value, ratio))
                        != Some(true)
                    {
                        return Err(format!(
                            "workload '{id}' {metric} ratio does not match recomputed samples"
                        ));
                    }
                    let resamples = stored_metric
                        .pointer("/bootstrap/resamples")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize;
                    let seed = stored_metric
                        .pointer("/bootstrap/seed")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let stored_low = numeric(stored_metric, "/bootstrap/interval/low");
                    let stored_high = numeric(stored_metric, "/bootstrap/interval/high");
                    if let (Some(stored_low), Some(stored_high)) = (stored_low, stored_high) {
                        let interval = bootstrap_median_ratio_interval(
                            &rust_values,
                            &python_values,
                            resamples,
                            seed,
                        )
                        .ok_or_else(|| {
                            format!(
                                "workload '{id}' {metric} bootstrap interval cannot be recomputed"
                            )
                        })?;
                        if !matches(stored_low, interval.0) || !matches(stored_high, interval.1) {
                            return Err(format!(
                                "workload '{id}' {metric} bootstrap interval does not match recomputed samples"
                            ));
                        }
                    }
                }
                _ => {
                    let verdict = stored_metric
                        .get("verdict")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if verdict != "unavailable" {
                        return Err(format!(
                            "workload '{id}' {metric} is null in paired samples but the comparison reports '{verdict}'"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Runs `report-check`.
pub fn report_check(args: &[String]) -> Result<(), String> {
    let opts = parse_report_check_args(args)?;
    let report_text = fs::read_to_string(&opts.report)
        .map_err(|e| format!("cannot read report {}: {e}", opts.report.display()))?;
    let visible = strip_code_fences(&report_text);
    let lowered = visible.to_lowercase();
    for placeholder in PLACEHOLDERS {
        if lowered.contains(placeholder) {
            return Err(format!(
                "report contains forbidden placeholder text '{placeholder}'"
            ));
        }
    }
    let section_headings = headings(&visible);
    for required in REQUIRED_SECTIONS {
        if !section_headings
            .iter()
            .any(|heading| heading.contains(required))
        {
            return Err(format!("report is missing required section '{required}'"));
        }
    }

    // Local links must resolve; remote URLs are not data links.
    for target in markdown_links(&visible) {
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with('#')
            || target.starts_with("mailto:")
        {
            continue;
        }
        let without_anchor = target.split('#').next().unwrap_or("");
        let without_query = without_anchor.split('?').next().unwrap_or("");
        if without_query.is_empty() {
            continue;
        }
        let path = PathBuf::from(without_query);
        let resolved = if path.is_absolute() {
            path
        } else {
            opts.report
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        };
        if !resolved.exists() {
            return Err(format!(
                "report link '{target}' does not resolve to an existing file"
            ));
        }
    }

    let paired_path = opts.data_dir.join("paired.json");
    let comparison_path = opts.data_dir.join("comparison.json");
    let paired_bytes = fs::read(&paired_path)
        .map_err(|e| format!("missing paired data {}: {e}", paired_path.display()))?;
    let paired: Value = serde_json::from_slice(&paired_bytes)
        .map_err(|e| format!("invalid paired data {}: {e}", paired_path.display()))?;
    if paired.get("kind").and_then(Value::as_str) != Some(PAIRED_KIND) {
        return Err(format!(
            "paired data {} does not have kind '{PAIRED_KIND}'",
            paired_path.display()
        ));
    }
    let comparison_bytes = fs::read(&comparison_path)
        .map_err(|e| format!("missing comparison data {}: {e}", comparison_path.display()))?;
    let comparison: Value = serde_json::from_slice(&comparison_bytes)
        .map_err(|e| format!("invalid comparison data {}: {e}", comparison_path.display()))?;
    let paired_sha256 = sha256_bytes(&paired_bytes);
    verify_comparison_against_paired(&paired, &comparison, &paired_sha256)?;

    // Required report references, checked independently of link syntax.
    for required_file in ["paired.json", "comparison.json", "charts/manifest.json"] {
        if !report_text.contains(required_file) {
            return Err(format!(
                "report does not reference required data file '{required_file}'"
            ));
        }
    }

    let workload_ids: BTreeSet<String> = paired
        .get("workloads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|workload| {
            workload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    if workload_ids.is_empty() {
        return Err("paired data contains no workloads".to_string());
    }
    for id in &workload_ids {
        if !report_text.contains(id) {
            return Err(format!("report does not mention stored workload id '{id}'"));
        }
    }
    if let Some(revision) = paired
        .pointer("/source/revision")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        if !report_text.contains(revision) {
            return Err(
                "report does not include the stored source revision provenance".to_string(),
            );
        }
    }
    let charts = verify_charts(&report_text, &opts.data_dir, &comparison, &workload_ids)?;

    println!(
        "report-check: ok ({} workloads, {} charts, paired sha256 {})",
        workload_ids.len(),
        charts.len(),
        paired_sha256
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let directory = std::env::temp_dir().join(format!(
            "apk-lab-compare-test-{}-{}-{}-{}",
            name,
            std::process::id(),
            nanos,
            counter
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn stats_use_midpoint_median_and_nearest_rank_p95() {
        let summary = stats(&[1.0, 4.0, 2.0, 3.0]).unwrap();
        assert_eq!(summary.count, 4);
        assert_eq!(summary.median, 2.5);
        assert_eq!(summary.p95, 4.0);
        let summary = stats(&[10.0]).unwrap();
        assert_eq!(summary.median, 10.0);
        assert_eq!(summary.p95, 10.0);
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn bootstrap_interval_is_deterministic_and_contains_median_ratio() {
        let rust = [12.0, 13.0, 14.0, 15.0, 16.0];
        let python = [10.0, 10.0, 10.0, 10.0, 10.0];
        let first = bootstrap_median_ratio_interval(&rust, &python, 500, 7).unwrap();
        let second = bootstrap_median_ratio_interval(&rust, &python, 500, 7).unwrap();
        assert_eq!(first, second);
        assert!(first.0 <= 1.4 && first.1 >= 1.4, "{first:?}");
        assert!(bootstrap_median_ratio_interval(&rust[..2], &python, 10, 7).is_none());
    }

    #[test]
    fn verdict_applies_ratio_budget_and_absolute_floor() {
        // A ratio above the budget but an absolute difference inside the floor
        // is measurement noise, not a material regression.
        assert_eq!(
            classify(1.50, (1.20, 1.80), 3.0, Some(5.0), 1.05, true),
            Verdict::Pass
        );
        assert_eq!(
            classify(1.50, (1.20, 1.80), 8.0, Some(5.0), 1.05, true),
            Verdict::MaterialSlowdown
        );
        assert_eq!(
            classify(1.02, (0.95, 1.10), 8.0, Some(5.0), 1.05, true),
            Verdict::Inconclusive
        );
        assert_eq!(
            classify(1.10, (1.06, 1.20), 8.0, None, 1.05, false),
            Verdict::ExplanationRequired
        );
        assert_eq!(
            classify(1.01, (0.98, 1.04), 8.0, None, 1.05, true),
            Verdict::Pass
        );
    }

    #[test]
    fn verdict_unavailable_for_non_finite_interval() {
        assert_eq!(
            classify(1.0, (f64::NAN, 1.0), 1.0, None, 1.05, true),
            Verdict::Unavailable
        );
    }

    fn paired_fixture() -> Value {
        let python_samples = vec![
            json!({"kind":"warmup","index":0,"outputValid":true}),
            json!({"kind":"measured","index":0,"wallMs":10.0,"cpuMs":8.0,"peakChildRssBytes":1000.0,"outputValid":true}),
            json!({"kind":"measured","index":1,"wallMs":12.0,"cpuMs":9.0,"peakChildRssBytes":1100.0,"outputValid":true}),
        ];
        let rust_samples = vec![
            json!({"kind":"warmup","index":0,"outputValid":true}),
            json!({"kind":"measured","index":0,"wallMs":11.0,"cpuMs":8.0,"peakChildRssBytes":1200.0,"outputValid":true}),
            json!({"kind":"measured","index":1,"wallMs":12.5,"cpuMs":9.0,"peakChildRssBytes":1300.0,"outputValid":true}),
        ];
        let pairs = (0..2)
            .map(|index| {
                json!({
                    "index": index,
                    "order": ["python","rust"],
                    "pythonIndex": index,
                    "rustIndex": index,
                    "pythonOutputValid": true,
                    "rustOutputValid": true,
                    "exitCodesMatch": true,
                    "stdoutSha256Match": true,
                    "stderrSha256Match": true,
                    "correct": true,
                    "mismatches": [],
                })
            })
            .collect::<Vec<_>>();
        json!({
            "schemaVersion": 1,
            "kind": PAIRED_KIND,
            "status": "complete",
            "policy": {"id": "test-policy", "sha256": "a".repeat(64), "requiredWorkloads": ["w.small"]},
            "engines": {
                "python": {"directExecution": true},
                "rust": {"directExecution": true, "compilationIncluded": false}
            },
            "source": {"revision": "0123456789abcdef0123456789abcdef01234567"},
            "corpus": {"identitySha256": "c".repeat(64)},
            "workloads": [{
                "id": "w.small",
                "category": "inspection",
                "size": "small",
                "costly": false,
                "sampleClass": "local",
                "status": "measured-paired",
                "measuredPairs": 2,
                "pairs": pairs,
                "correctness": {"matched": true},
                "engines": {
                    "python": {
                        "samplesExpected": 2, "samplesCompleted": 2,
                        "warmupsExpected": 1, "warmupsCompleted": 1,
                        "samples": python_samples
                    },
                    "rust": {
                        "samplesExpected": 2, "samplesCompleted": 2,
                        "warmupsExpected": 1, "warmupsCompleted": 1,
                        "samples": rust_samples
                    }
                }
            }],
            "acceptance": {"valid": true, "reasons": []}
        })
    }

    fn policy_fixture() -> PerformancePolicy {
        serde_json::from_str(include_str!("../../migration/performance-policy.json")).unwrap()
    }

    fn relaxed_policy() -> PerformancePolicy {
        let mut policy = policy_fixture();
        policy.sample_classes.insert(
            "local".to_string(),
            super::super::benchmark::SampleClass {
                warmups: 1,
                samples: 2,
            },
        );
        policy
    }

    #[test]
    fn comparison_recomputes_summaries_and_passes_inside_budget() {
        let policy = relaxed_policy();
        let workload = &paired_fixture()["workloads"][0];
        let (comparison, reasons) = compare_workload("w.small", workload, &policy);
        assert!(reasons.is_empty(), "{reasons:?}");
        assert_eq!(comparison["verdict"], json!("pass"));
        assert_eq!(
            comparison["metrics"]["wallTime"]["python"]["median"],
            json!(11.0)
        );
        assert_eq!(
            comparison["metrics"]["wallTime"]["rust"]["median"],
            json!(11.75)
        );
        assert_eq!(
            comparison["metrics"]["wallTime"]["python"]["p95"],
            json!(12.0)
        );
        assert_eq!(
            comparison["metrics"]["wallTime"]["rust"]["p95"],
            json!(12.5)
        );
        assert_eq!(comparison["metrics"]["rss"]["verdict"], json!("pass"));
    }

    fn report_fixture() -> (PathBuf, PathBuf, PathBuf) {
        let directory = test_directory("report");
        let data_dir = directory.join("docs/benchmarks/apk-lab");
        fs::create_dir_all(data_dir.join("charts")).unwrap();
        let paired = paired_fixture();
        let paired_bytes = serde_json::to_vec_pretty(&paired).unwrap();
        fs::write(data_dir.join("paired.json"), &paired_bytes).unwrap();
        let paired_sha = sha256_bytes(&paired_bytes);
        let wall_interval =
            bootstrap_median_ratio_interval(&[11.0, 12.5], &[10.0, 12.0], 50, 5).unwrap();
        let comparison = json!({
            "schemaVersion": 1,
            "kind": COMPARISON_KIND,
            "status": "pass",
            "input": {"kind": PAIRED_KIND, "sha256": paired_sha},
            "policy": {"id": "test-policy"},
            "workloads": [{
                "id": "w.small",
                "metrics": {
                    "wallTime": {
                        "unit": "milliseconds",
                        "python": {"count": 2, "median": 11.0, "p95": 12.0},
                        "rust": {"count": 2, "median": 11.75, "p95": 12.5},
                        "ratio": 11.75 / 11.0,
                        "bootstrap": {
                            "resamples": 50,
                            "confidence": 0.95,
                            "seed": 5,
                            "interval": {"low": wall_interval.0, "high": wall_interval.1}
                        },
                        "verdict": "pass"
                    },
                    "cpu": {
                        "unit": "milliseconds",
                        "python": {"count": 2, "median": 8.5, "p95": 9.0},
                        "rust": {"count": 2, "median": 8.5, "p95": 9.0},
                        "ratio": 1.0,
                        "verdict": "pass"
                    },
                    "rss": {
                        "unit": "bytes",
                        "python": {"count": 2, "median": 1050.0, "p95": 1100.0},
                        "rust": {"count": 2, "median": 1250.0, "p95": 1300.0},
                        "ratio": 1250.0 / 1050.0,
                        "verdict": "pass"
                    }
                }
            }],
            "acceptance": {"valid": true, "reasons": []}
        });
        fs::write(
            data_dir.join("comparison.json"),
            serde_json::to_vec_pretty(&comparison).unwrap(),
        )
        .unwrap();
        fs::write(
            data_dir.join("charts/wall.png"),
            b"\x89PNG\r\n\x1a\nfixture",
        )
        .unwrap();
        fs::write(
            data_dir.join("charts/manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "charts": [{
                    "file": "charts/wall.png",
                    "workloads": ["w.small"],
                    "metric": "wallTime",
                    "unit": "milliseconds"
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let report = directory.join("TO-RUST.md");
        let text = "# Apache Lab migration\n\n             ## Goals and non-goals\nGoals.\n\n             ## Architecture\nArchitecture.\n\n             ## Migration map\nMap.\n\n             ## Clean-code decisions\nDecisions.\n\n             ## Provenance\nRevision 0123456789abcdef0123456789abcdef01234567.\n\n             ## Methodology\nMethod.\n\n             ## Data and charts\n[paired](docs/benchmarks/apk-lab/paired.json) [comparison](docs/benchmarks/apk-lab/comparison.json)\n             [manifest](docs/benchmarks/apk-lab/charts/manifest.json) [wall](docs/benchmarks/apk-lab/charts/wall.png)\n\n             ## Correctness and integration evidence\nw.small passed.\n\n             ## Rejected and inconclusive measurements\nNone.\n\n             ## Performance acceptance verdict\nPass.\n\n             ## Python retirement\nRetired.\n\n             ## Operational rollback and rebuild\nRebuild.\n\n             ## Limitations\nNone.\n"
        .to_string();
        fs::write(&report, text).unwrap();
        (directory, report, data_dir)
    }

    #[test]
    fn report_check_accepts_consistent_data_and_rejects_placeholder_or_missing_chart() {
        let (directory, report, data_dir) = report_fixture();
        let args: Vec<String> = [
            "--report".to_string(),
            report.display().to_string(),
            "--data-dir".to_string(),
            data_dir.display().to_string(),
        ]
        .to_vec();
        report_check(&args).expect("consistent report fixture must pass");

        let original = fs::read_to_string(&report).unwrap();
        fs::write(&report, format!("{original}\nTODO: fill in\n")).unwrap();
        let error = report_check(&args).unwrap_err();
        assert!(error.contains("placeholder"), "{error}");
        fs::write(&report, &original).unwrap();

        let chart = data_dir.join("charts/wall.png");
        fs::remove_file(&chart).unwrap();
        let error = report_check(&args).unwrap_err();
        assert!(
            error.contains("does not exist") || error.contains("chart"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn report_check_rejects_a_missing_required_section() {
        let (directory, report, data_dir) = report_fixture();
        let text = fs::read_to_string(&report).unwrap();
        fs::write(&report, text.replace("## Limitations\n", "")).unwrap();
        let args: Vec<String> = [
            "--report".to_string(),
            report.display().to_string(),
            "--data-dir".to_string(),
            data_dir.display().to_string(),
        ]
        .to_vec();
        let error = report_check(&args).unwrap_err();
        assert!(error.contains("required section"), "{error}");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn report_check_rejects_stale_or_inconsistent_comparison_summaries() {
        let (directory, report, data_dir) = report_fixture();
        let comparison_path = data_dir.join("comparison.json");
        let mut comparison: Value =
            serde_json::from_slice(&fs::read(&comparison_path).unwrap()).unwrap();
        comparison["workloads"][0]["metrics"]["wallTime"]["rust"]["median"] = json!(12.0);
        fs::write(
            &comparison_path,
            serde_json::to_vec_pretty(&comparison).unwrap(),
        )
        .unwrap();
        let args: Vec<String> = [
            "--report".to_string(),
            report.display().to_string(),
            "--data-dir".to_string(),
            data_dir.display().to_string(),
        ]
        .to_vec();
        let error = report_check(&args).unwrap_err();
        assert!(
            error.contains("median/p95") || error.contains("recomputed"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn report_check_rejects_comparison_from_a_different_paired_file() {
        let (directory, report, data_dir) = report_fixture();
        let comparison_path = data_dir.join("comparison.json");
        let mut comparison: Value =
            serde_json::from_slice(&fs::read(&comparison_path).unwrap()).unwrap();
        comparison["input"]["sha256"] = json!("b".repeat(64));
        fs::write(
            &comparison_path,
            serde_json::to_vec_pretty(&comparison).unwrap(),
        )
        .unwrap();
        let args: Vec<String> = [
            "--report".to_string(),
            report.display().to_string(),
            "--data-dir".to_string(),
            data_dir.display().to_string(),
        ]
        .to_vec();
        let error = report_check(&args).unwrap_err();
        assert!(error.contains("sha256 mismatch"), "{error}");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn comparison_flags_missing_samples() {
        let policy = relaxed_policy();
        let mut fixture = paired_fixture();
        fixture["workloads"][0]["engines"]["rust"]["samples"]
            .as_array_mut()
            .unwrap()
            .retain(|sample| sample["kind"] != "measured");
        let (comparison, _) = compare_workload("w.small", &fixture["workloads"][0], &policy);
        assert_eq!(comparison["verdict"], json!("fail"));
        assert!(comparison["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("measured samples")));
    }

    #[test]
    fn comparison_flags_material_slowdown() {
        let policy = relaxed_policy();
        let mut fixture = paired_fixture();
        for (index, sample) in fixture["workloads"][0]["engines"]["rust"]["samples"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .filter(|sample| sample["kind"] == "measured")
            .enumerate()
        {
            sample["wallMs"] = json!(100.0 + index as f64);
        }
        let (comparison, _) = compare_workload("w.small", &fixture["workloads"][0], &policy);
        assert_eq!(
            comparison["metrics"]["wallTime"]["verdict"],
            json!("material-slowdown")
        );
        assert_eq!(comparison["verdict"], json!("fail"));
    }

    #[test]
    fn comparison_marks_crossing_interval_inconclusive() {
        // Two ratios tightly clustered just over the budget would be material;
        // mixed ratios make the interval straddle 1.05 and must block as
        // inconclusive instead of being averaged into a pass.
        let policy = relaxed_policy();
        let mut fixture = paired_fixture();
        let rust_samples = fixture["workloads"][0]["engines"]["rust"]["samples"]
            .as_array_mut()
            .unwrap();
        rust_samples[1]["wallMs"] = json!(100.0);
        rust_samples[2]["wallMs"] = json!(11.0);
        let (comparison, _) = compare_workload("w.small", &fixture["workloads"][0], &policy);
        assert_eq!(
            comparison["metrics"]["wallTime"]["verdict"],
            json!("inconclusive")
        );
    }
}
