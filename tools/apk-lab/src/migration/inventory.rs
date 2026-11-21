//! Contract inventory and drift check for the Python apk-lab oracle.
//!
//! `migration/contracts.json` freezes the observable interface of the Python
//! implementation: parser commands/options/defaults/mutual exclusions,
//! compatibility_ci commands, tests, external consumers, JSON wire fields,
//! exit classes, acquisition source labels, module entrypoints and the
//! Python-only regular expressions that a Rust port must reproduce.
//!
//! `inventory --check` independently re-discovers those interfaces from the
//! Python sources and rejects any unmapped addition, removal or metadata
//! change. It never imports `apk_lab`: discovery is a source parser, not an
//! echo of the frozen file. The check is intentionally stricter about stale
//! mappings than about formatting; source line numbers are never recorded.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Contract and discovery data
// ---------------------------------------------------------------------------

fn default_action() -> String {
    "store".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutexGroup {
    pub id: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionSpec {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub positional: Option<String>,
    #[serde(default)]
    pub positionals: Vec<String>,
    #[serde(default)]
    pub dest: Option<String>,
    #[serde(default = "default_action")]
    pub action: String,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    /// Source-level default. `Null` means no `default=` keyword or `default=None`.
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub nargs: Option<String>,
    #[serde(rename = "const", default)]
    pub const_value: Value,
    #[serde(rename = "type", default)]
    pub type_expr: Option<String>,
    #[serde(default)]
    pub mutex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub id: String,
    pub entry: String,
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default)]
    pub handler: Option<String>,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub subcommand_dest: Option<String>,
    #[serde(default)]
    pub subcommands_required: Option<bool>,
    #[serde(default)]
    pub options: Vec<OptionSpec>,
    #[serde(default)]
    pub mutex_groups: Vec<MutexGroup>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandContract {
    #[serde(flatten)]
    spec: CommandSpec,
    #[serde(default)]
    behavior: Value,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CiContract {
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    parser: Option<CommandSpec>,
    #[serde(default)]
    commands: Vec<CommandContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestSummary {
    modules: usize,
    functions: usize,
    cases: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    name: String,
    cases: usize,
    #[serde(default)]
    destination: String,
    #[serde(default)]
    destination_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestModule {
    path: String,
    /// apk_lab imports used by this test module. These are test-only
    /// internals, not supported public interfaces.
    #[serde(default)]
    interfaces: Vec<String>,
    tests: Vec<TestCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestContracts {
    summary: TestSummary,
    modules: Vec<TestModule>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Consumer {
    path: String,
    #[serde(default)]
    interfaces: Vec<String>,
    #[serde(default)]
    classification: String,
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    notes: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSchema {
    id: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    serialization: Value,
    #[serde(default)]
    notes: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegexEntry {
    #[serde(default)]
    source: String,
    #[serde(default)]
    function: String,
    #[serde(default)]
    op: String,
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    python_only_features: Vec<String>,
    #[serde(default)]
    behavior: String,
    #[serde(default)]
    destination: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExitClass {
    name: String,
    code: i64,
    #[serde(default)]
    meaning: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcquisitionSource {
    name: String,
    label: String,
    #[serde(default)]
    interface: String,
    #[serde(default)]
    meaning: String,
    #[serde(default)]
    destination: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entrypoint {
    kind: String,
    name: String,
    target: String,
    #[serde(default)]
    notes: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contracts {
    schema_version: u32,
    #[serde(default)]
    provenance: Value,
    #[serde(default)]
    discovery: Value,
    #[serde(default)]
    exit_classes: Vec<ExitClass>,
    #[serde(default)]
    global_parser: Option<CommandSpec>,
    #[serde(default)]
    commands: Vec<CommandContract>,
    #[serde(default)]
    compatibility_ci: Option<CiContract>,
    #[serde(default)]
    module_entrypoints: Vec<Entrypoint>,
    #[serde(default)]
    consumers: Vec<Consumer>,
    #[serde(default)]
    wire_schemas: Vec<WireSchema>,
    #[serde(default)]
    python_only_regexes: Vec<RegexEntry>,
    #[serde(default)]
    acquisition_sources: Vec<AcquisitionSource>,
    #[serde(default)]
    tests: Option<TestContracts>,
}

#[derive(Debug, Clone)]
struct Discovery {
    global_parser: CommandSpec,
    commands: Vec<CommandSpec>,
    ci_parser: CommandSpec,
    ci_commands: Vec<CommandSpec>,
    tests: TestContracts,
    consumers: Vec<Consumer>,
    wire_schemas: Vec<WireSchema>,
    regexes: Vec<RegexEntry>,
    exit_classes: Vec<ExitClass>,
    acquisition_sources: Vec<AcquisitionSource>,
    entrypoints: Vec<Entrypoint>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Runs `inventory` with the arguments that follow the subcommand.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut check = false;
    let mut root: Option<PathBuf> = None;
    let mut contracts: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check = true,
            "--root" => {
                i += 1;
                let value = args
                    .get(i)
                    .map(String::as_str)
                    .ok_or("--root requires a path")?;
                root = Some(PathBuf::from(value));
            }
            "--contracts" => {
                i += 1;
                let value = args
                    .get(i)
                    .map(String::as_str)
                    .ok_or("--contracts requires a path")?;
                contracts = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "apk-lab-migrate inventory [--check] [--root <repo>] [--contracts <path>]\n\
                     \n\
                     Discovers the Python apk-lab command, test, consumer and wire\n\
                     interfaces and validates migration/contracts.json. Any unmapped\n\
                     addition, removal or metadata change is an error."
                );
                return Ok(());
            }
            other => {
                return Err(format!(
                    "unknown inventory option '{other}' (expected --check, --root or --contracts)"
                ));
            }
        }
        i += 1;
    }

    let root = match root {
        Some(path) => path,
        None => discover_root()?,
    };
    let contracts_path = contracts.unwrap_or_else(|| {
        root.join("tools")
            .join("apk-lab")
            .join("migration")
            .join("contracts.json")
    });

    let discovery = discover_all(&root)?;
    let contract = load_contracts(&contracts_path)?;
    let issues = validate(&discovery, &contract);

    if !issues.is_empty() {
        let shown: Vec<&String> = issues.iter().take(80).collect();
        let mut message = format!(
            "inventory check failed with {} issue(s) against {}:\n",
            issues.len(),
            contracts_path.display()
        );
        for issue in shown {
            message.push_str("- ");
            message.push_str(issue);
            message.push('\n');
        }
        if issues.len() > 80 {
            message.push_str(&format!("- ... and {} more\n", issues.len() - 80));
        }
        return Err(message.trim_end().to_string());
    }

    if check {
        println!(
            "inventory check: OK ({} cli commands, {} ci commands, {} tests/{} cases, {} consumers, {} wire schemas, {} exit classes, {} acquisition sources, {} python-only regexes)",
            discovery.commands.len(),
            discovery.ci_commands.len(),
            discovery.tests.summary.functions,
            discovery.tests.summary.cases,
            discovery.consumers.len(),
            discovery.wire_schemas.len(),
            discovery.exit_classes.len(),
            discovery.acquisition_sources.len(),
            discovery.regexes.len()
        );
    } else {
        println!(
            "inventory: contracts at {} are current",
            contracts_path.display()
        );
    }
    Ok(())
}

fn discover_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    for dir in cwd.ancestors() {
        if dir.join("tools/apk-lab/apk_lab/cli.py").is_file() {
            return Ok(dir.to_path_buf());
        }
        if dir.join("apk_lab/cli.py").is_file() {
            if let Some(root) = dir.parent().and_then(|p| p.parent()) {
                return Ok(root.to_path_buf());
            }
        }
    }
    Err(format!(
        "could not locate the repository root containing tools/apk-lab/apk_lab/cli.py from {}",
        cwd.display()
    ))
}

fn load_contracts(path: &Path) -> Result<Contracts, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("cannot read contracts {}: {e}", path.display()))?;
    let contracts: Contracts = serde_json::from_str(&text)
        .map_err(|e| format!("invalid contracts JSON {}: {e}", path.display()))?;
    if contracts.schema_version != 1 {
        return Err(format!(
            "unsupported contracts schemaVersion {} in {} (expected 1)",
            contracts.schema_version,
            path.display()
        ));
    }
    Ok(contracts)
}

fn discover_all(root: &Path) -> Result<Discovery, String> {
    let lab = root.join("tools").join("apk-lab");
    let cli_source = read_required(&lab.join("apk_lab/cli.py"))?;
    let ci_source = read_required(&lab.join("apk_lab/compatibility_ci.py"))?;
    let (mut global_parser, commands) = scan_parser(&cli_source, "apk-lab")?;
    global_parser.id = "apk-lab".to_string();
    global_parser.path = Vec::new();
    let (mut ci_parser, ci_commands) = scan_parser(&ci_source, "compatibility_ci")?;
    ci_parser.id = "compatibility_ci".to_string();
    ci_parser.path = Vec::new();

    let tests = discover_tests(&lab)?;
    let consumers = discover_consumers(root)?;
    let wire_schemas = discover_wire(&lab)?;
    let regexes = discover_python_only_regexes(&lab)?;
    let exit_classes = discover_exit_classes(&lab)?;
    let acquisition_sources = discover_acquisition_sources(&lab)?;
    let entrypoints = discover_entrypoints(&lab)?;

    Ok(Discovery {
        global_parser,
        commands,
        ci_parser,
        ci_commands,
        tests,
        consumers,
        wire_schemas,
        regexes,
        exit_classes,
        acquisition_sources,
        entrypoints,
    })
}

fn read_required(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate(d: &Discovery, c: &Contracts) -> Vec<String> {
    let mut issues = Vec::new();

    match &c.global_parser {
        Some(spec) => compare_command("global parser", spec, &d.global_parser, &mut issues),
        None => issues.push("missing mapping: globalParser".to_string()),
    }

    let command_map: BTreeMap<&str, &CommandSpec> = c
        .commands
        .iter()
        .map(|c| (c.spec.id.as_str(), &c.spec))
        .collect();
    let discovered_ids: BTreeSet<&str> = d.commands.iter().map(|c| c.id.as_str()).collect();
    for command in &d.commands {
        match command_map.get(command.id.as_str()) {
            Some(mapped) => compare_command(
                &format!("apk-lab {}", command.id),
                mapped,
                command,
                &mut issues,
            ),
            None => issues.push(format!(
                "missing command mapping: apk-lab {} (handler {})",
                command.id,
                command.handler.as_deref().unwrap_or("<none>")
            )),
        }
    }
    for mapped in &c.commands {
        if !discovered_ids.contains(mapped.spec.id.as_str()) {
            issues.push(format!(
                "stale command mapping: apk-lab {} is no longer a parser command",
                mapped.spec.id
            ));
        }
    }

    match &c.compatibility_ci {
        Some(ci) => {
            if let Some(parser) = &ci.parser {
                compare_command("compatibility_ci parser", parser, &d.ci_parser, &mut issues);
            } else {
                issues.push("missing mapping: compatibilityCi.parser".to_string());
            }
            let ci_map: BTreeMap<&str, &CommandSpec> = ci
                .commands
                .iter()
                .map(|c| (c.spec.id.as_str(), &c.spec))
                .collect();
            let ci_ids: BTreeSet<&str> = d.ci_commands.iter().map(|c| c.id.as_str()).collect();
            for command in &d.ci_commands {
                match ci_map.get(command.id.as_str()) {
                    Some(mapped) => compare_command(
                        &format!("compatibility_ci {}", command.id),
                        mapped,
                        command,
                        &mut issues,
                    ),
                    None => issues.push(format!(
                        "missing compatibility_ci command mapping: {}",
                        command.id
                    )),
                }
            }
            for mapped in &ci.commands {
                if !ci_ids.contains(mapped.spec.id.as_str()) {
                    issues.push(format!(
                        "stale compatibility_ci command mapping: {}",
                        mapped.spec.id
                    ));
                }
            }
        }
        None => issues.push("missing mapping: compatibilityCi".to_string()),
    }

    compare_tests(&d.tests, c.tests.as_ref(), &mut issues);
    compare_consumers(&d.consumers, &c.consumers, &mut issues);
    compare_wire(&d.wire_schemas, &c.wire_schemas, &mut issues);
    compare_regexes(&d.regexes, &c.python_only_regexes, &mut issues);
    compare_exit_classes(&d.exit_classes, &c.exit_classes, &mut issues);
    compare_acquisition(&d.acquisition_sources, &c.acquisition_sources, &mut issues);
    compare_entrypoints(&d.entrypoints, &c.module_entrypoints, &mut issues);

    issues
}

fn compare_command(
    label: &str,
    mapped: &CommandSpec,
    found: &CommandSpec,
    issues: &mut Vec<String>,
) {
    if mapped.id != found.id {
        issues.push(format!(
            "mismatched mapping: {label} id: contract={} discovered={}",
            mapped.id, found.id
        ));
    }
    if mapped.path != found.path {
        issues.push(format!(
            "mismatched mapping: {label} path: contract={:?} discovered={:?}",
            mapped.path, found.path
        ));
    }
    if mapped.entry != found.entry {
        issues.push(format!(
            "mismatched mapping: {label} entry: contract={} discovered={}",
            mapped.entry, found.entry
        ));
    }
    if mapped.handler != found.handler {
        issues.push(format!(
            "mismatched mapping: {label} handler: contract={:?} discovered={:?}",
            mapped.handler, found.handler
        ));
    }
    if mapped.aliases != found.aliases {
        issues.push(format!(
            "mismatched mapping: {label} aliases: contract={:?} discovered={:?}",
            mapped.aliases, found.aliases
        ));
    }
    if mapped.required != found.required {
        issues.push(format!(
            "mismatched mapping: {label} required: contract={:?} discovered={:?}",
            mapped.required, found.required
        ));
    }
    if mapped.subcommand_dest != found.subcommand_dest {
        issues.push(format!(
            "mismatched mapping: {label} subcommandDest: contract={:?} discovered={:?}",
            mapped.subcommand_dest, found.subcommand_dest
        ));
    }
    if mapped.subcommands_required != found.subcommands_required {
        issues.push(format!(
            "mismatched mapping: {label} subcommandsRequired: contract={:?} discovered={:?}",
            mapped.subcommands_required, found.subcommands_required
        ));
    }
    compare_options(label, &mapped.options, &found.options, issues);
    if mapped.mutex_groups != found.mutex_groups {
        issues.push(format!(
            "mismatched mapping: {label} mutexGroups: contract={:?} discovered={:?}",
            mapped.mutex_groups, found.mutex_groups
        ));
    }
}

fn compare_options(
    label: &str,
    mapped: &[OptionSpec],
    found: &[OptionSpec],
    issues: &mut Vec<String>,
) {
    let mapped_by_key: BTreeMap<String, &OptionSpec> =
        mapped.iter().map(|o| (option_key(o), o)).collect();
    let found_keys: BTreeSet<String> = found.iter().map(option_key).collect();
    for option in found {
        let key = option_key(option);
        match mapped_by_key.get(&key) {
            Some(expected) => compare_option(label, &key, expected, option, issues),
            None => issues.push(format!("missing option mapping: {label} option {key}")),
        }
    }
    for option in mapped {
        let key = option_key(option);
        if !found_keys.contains(&key) {
            issues.push(format!(
                "stale option mapping: {label} option {key} is no longer declared"
            ));
        }
    }
    if mapped.len() != found.len() {
        issues.push(format!(
            "mismatched mapping: {label} option count: contract={} discovered={}",
            mapped.len(),
            found.len()
        ));
    }
}

fn option_key(o: &OptionSpec) -> String {
    if !o.names.is_empty() {
        o.names.join("|")
    } else if !o.positionals.is_empty() {
        format!("positional:{}", o.positionals.join("|"))
    } else if let Some(p) = &o.positional {
        format!("positional:{p}")
    } else {
        "<unnamed>".to_string()
    }
}

fn compare_option(
    label: &str,
    key: &str,
    mapped: &OptionSpec,
    found: &OptionSpec,
    issues: &mut Vec<String>,
) {
    if mapped.names != found.names {
        issues.push(format!(
            "mismatched mapping: {label} option {key} names: contract={:?} discovered={:?}",
            mapped.names, found.names
        ));
    }
    if mapped.positional != found.positional {
        issues.push(format!(
            "mismatched mapping: {label} option {key} positional: contract={:?} discovered={:?}",
            mapped.positional, found.positional
        ));
    }
    if mapped.positionals != found.positionals {
        issues.push(format!(
            "mismatched mapping: {label} option {key} positionals: contract={:?} discovered={:?}",
            mapped.positionals, found.positionals
        ));
    }
    if mapped.dest != found.dest {
        issues.push(format!(
            "mismatched mapping: {label} option {key} dest: contract={:?} discovered={:?}",
            mapped.dest, found.dest
        ));
    }
    if mapped.action != found.action {
        issues.push(format!(
            "mismatched mapping: {label} option {key} action: contract={} discovered={}",
            mapped.action, found.action
        ));
    }
    if mapped.choices != found.choices {
        issues.push(format!(
            "mismatched mapping: {label} option {key} choices: contract={:?} discovered={:?}",
            mapped.choices, found.choices
        ));
    }
    if mapped.default != found.default {
        issues.push(format!(
            "mismatched mapping: {label} option {key} default: contract={} discovered={}",
            mapped.default, found.default
        ));
    }
    if mapped.required != found.required {
        issues.push(format!(
            "mismatched mapping: {label} option {key} required: contract={:?} discovered={:?}",
            mapped.required, found.required
        ));
    }
    if mapped.nargs != found.nargs {
        issues.push(format!(
            "mismatched mapping: {label} option {key} nargs: contract={:?} discovered={:?}",
            mapped.nargs, found.nargs
        ));
    }
    if mapped.const_value != found.const_value {
        issues.push(format!(
            "mismatched mapping: {label} option {key} const: contract={} discovered={}",
            mapped.const_value, found.const_value
        ));
    }
    if mapped.type_expr != found.type_expr {
        issues.push(format!(
            "mismatched mapping: {label} option {key} type: contract={:?} discovered={:?}",
            mapped.type_expr, found.type_expr
        ));
    }
    if mapped.mutex != found.mutex {
        issues.push(format!(
            "mismatched mapping: {label} option {key} mutex: contract={:?} discovered={:?}",
            mapped.mutex, found.mutex
        ));
    }
}

fn compare_tests(found: &TestContracts, mapped: Option<&TestContracts>, issues: &mut Vec<String>) {
    let Some(mapped) = mapped else {
        issues.push("missing mapping: tests".to_string());
        return;
    };
    if mapped.summary.modules != found.summary.modules
        || mapped.summary.functions != found.summary.functions
        || mapped.summary.cases != found.summary.cases
    {
        issues.push(format!(
            "mismatched mapping: test summary: contract={}/{}/{} discovered={}/{}/{} (modules/functions/cases)",
            mapped.summary.modules,
            mapped.summary.functions,
            mapped.summary.cases,
            found.summary.modules,
            found.summary.functions,
            found.summary.cases
        ));
    }
    let mapped_tests: BTreeMap<(String, String), &TestCase> = mapped
        .modules
        .iter()
        .flat_map(|m| {
            m.tests
                .iter()
                .map(move |t| ((m.path.clone(), t.name.clone()), t))
        })
        .collect();
    let found_keys: BTreeSet<(String, String)> = found
        .modules
        .iter()
        .flat_map(|m| {
            m.tests
                .iter()
                .map(move |t| (m.path.clone(), t.name.clone()))
        })
        .collect();
    let mapped_modules: BTreeMap<&str, &TestModule> = mapped
        .modules
        .iter()
        .map(|m| (m.path.as_str(), m))
        .collect();
    for module in &found.modules {
        if let Some(m) = mapped_modules.get(module.path.as_str()) {
            if m.interfaces != module.interfaces {
                issues.push(format!(
                    "mismatched test-only interfaces: {}: contract={:?} discovered={:?}",
                    module.path, m.interfaces, module.interfaces
                ));
            }
        }
        for test in &module.tests {
            match mapped_tests.get(&(module.path.clone(), test.name.clone())) {
                Some(m) => {
                    if m.cases != test.cases {
                        issues.push(format!(
                            "mismatched mapping: {}::{} cases: contract={} discovered={}",
                            module.path, test.name, m.cases, test.cases
                        ));
                    }
                    if m.destination.trim().is_empty() {
                        issues.push(format!(
                            "missing destination: {}::{} has no Rust/differential destination",
                            module.path, test.name
                        ));
                    }
                    if m.destination_status.trim().is_empty() {
                        issues.push(format!(
                            "missing destination status: {}::{} must state planned/implemented",
                            module.path, test.name
                        ));
                    }
                }
                None => issues.push(format!(
                    "missing test mapping: {}::{} ({} case(s))",
                    module.path, test.name, test.cases
                )),
            }
        }
    }
    let found_modules: BTreeSet<&str> = found.modules.iter().map(|m| m.path.as_str()).collect();
    for module in &mapped.modules {
        if !found_modules.contains(module.path.as_str()) {
            issues.push(format!("stale test module mapping: {}", module.path));
        }
        for test in &module.tests {
            if !found_keys.contains(&(module.path.clone(), test.name.clone())) {
                issues.push(format!(
                    "stale test mapping: {}::{} no longer exists",
                    module.path, test.name
                ));
            }
        }
    }
}

fn compare_consumers(found: &[Consumer], mapped: &[Consumer], issues: &mut Vec<String>) {
    let mapped_by_path: BTreeMap<&str, &Consumer> =
        mapped.iter().map(|c| (c.path.as_str(), c)).collect();
    let found_paths: BTreeSet<&str> = found.iter().map(|c| c.path.as_str()).collect();
    for consumer in found {
        match mapped_by_path.get(consumer.path.as_str()) {
            Some(m) => {
                if m.interfaces != consumer.interfaces {
                    issues.push(format!(
                        "mismatched consumer interfaces: {}: contract={:?} discovered={:?}",
                        consumer.path, m.interfaces, consumer.interfaces
                    ));
                }
            }
            None => issues.push(format!(
                "missing consumer mapping: {} ({})",
                consumer.path,
                consumer.interfaces.join(", ")
            )),
        }
    }
    for consumer in mapped {
        if !found_paths.contains(consumer.path.as_str()) {
            issues.push(format!(
                "stale consumer mapping: {} no longer references apk_lab",
                consumer.path
            ));
        }
    }
}

fn compare_wire(found: &[WireSchema], mapped: &[WireSchema], issues: &mut Vec<String>) {
    let mapped_by_id: BTreeMap<&str, &WireSchema> =
        mapped.iter().map(|w| (w.id.as_str(), w)).collect();
    let found_ids: BTreeSet<&str> = found.iter().map(|w| w.id.as_str()).collect();
    for schema in found {
        match mapped_by_id.get(schema.id.as_str()) {
            Some(m) => {
                if m.fields != schema.fields {
                    issues.push(format!(
                        "mismatched wire fields: {}: contract={:?} discovered={:?}",
                        schema.id, m.fields, schema.fields
                    ));
                }
                if m.source != schema.source {
                    issues.push(format!(
                        "mismatched wire source: {}: contract={} discovered={}",
                        schema.id, m.source, schema.source
                    ));
                }
                if m.kind != schema.kind {
                    issues.push(format!(
                        "mismatched wire kind: {}: contract={} discovered={}",
                        schema.id, m.kind, schema.kind
                    ));
                }
            }
            None => issues.push(format!(
                "missing wire schema mapping: {} ({})",
                schema.id,
                schema.fields.join(", ")
            )),
        }
    }
    for schema in mapped {
        if !found_ids.contains(schema.id.as_str()) {
            issues.push(format!(
                "stale wire schema mapping: {} is no longer discovered",
                schema.id
            ));
        }
    }
}

fn regex_key(r: &RegexEntry) -> (String, String, String, String) {
    (
        r.source.clone(),
        r.function.clone(),
        r.op.clone(),
        r.pattern.clone(),
    )
}

fn compare_regexes(found: &[RegexEntry], mapped: &[RegexEntry], issues: &mut Vec<String>) {
    let mut mapped_keys: Vec<_> = mapped.iter().map(regex_key).collect();
    let mut found_keys: Vec<_> = found.iter().map(regex_key).collect();
    mapped_keys.sort();
    found_keys.sort();
    for key in &found_keys {
        if !mapped_keys.contains(key) {
            issues.push(format!(
                "missing python-only regex mapping: {}::{} re.{} pattern={:?}",
                key.0, key.1, key.2, key.3
            ));
        }
    }
    for key in &mapped_keys {
        if !found_keys.contains(key) {
            issues.push(format!(
                "stale python-only regex mapping: {}::{} re.{} pattern={:?}",
                key.0, key.1, key.2, key.3
            ));
        }
    }
    for regex in found {
        if let Some(mapped_regex) = mapped.iter().find(|m| regex_key(m) == regex_key(regex)) {
            if mapped_regex.python_only_features != regex.python_only_features {
                issues.push(format!(
                    "mismatched regex features: {}::{} re.{}: contract={:?} discovered={:?}",
                    regex.source,
                    regex.function,
                    regex.op,
                    mapped_regex.python_only_features,
                    regex.python_only_features
                ));
            }
        }
    }
}

fn compare_exit_classes(found: &[ExitClass], mapped: &[ExitClass], issues: &mut Vec<String>) {
    let found_pairs: Vec<_> = found.iter().map(|e| (&e.name, e.code)).collect();
    let mapped_pairs: Vec<_> = mapped.iter().map(|e| (&e.name, e.code)).collect();
    if found_pairs != mapped_pairs {
        issues.push(format!(
            "mismatched exit classes: contract={:?} discovered={:?}",
            mapped_pairs, found_pairs
        ));
    }
}

fn compare_acquisition(
    found: &[AcquisitionSource],
    mapped: &[AcquisitionSource],
    issues: &mut Vec<String>,
) {
    let found_pairs: Vec<_> = found.iter().map(|a| (&a.name, &a.label)).collect();
    let mapped_pairs: Vec<_> = mapped.iter().map(|a| (&a.name, &a.label)).collect();
    if found_pairs != mapped_pairs {
        issues.push(format!(
            "mismatched acquisition source labels: contract={:?} discovered={:?}",
            mapped_pairs, found_pairs
        ));
    }
}

fn compare_entrypoints(found: &[Entrypoint], mapped: &[Entrypoint], issues: &mut Vec<String>) {
    let found_triples: Vec<_> = found
        .iter()
        .map(|e| (&e.kind, &e.name, &e.target))
        .collect();
    let mapped_triples: Vec<_> = mapped
        .iter()
        .map(|e| (&e.kind, &e.name, &e.target))
        .collect();
    if found_triples != mapped_triples {
        issues.push(format!(
            "mismatched module entrypoints: contract={:?} discovered={:?}",
            mapped_triples, found_triples
        ));
    }
}

// ---------------------------------------------------------------------------
// argparse discovery
// ---------------------------------------------------------------------------

fn scan_parser(source: &str, entry: &str) -> Result<(CommandSpec, Vec<CommandSpec>), String> {
    let mut commands: Vec<CommandSpec> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut var_cmd: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut var_sub: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut var_group: BTreeMap<String, (Vec<String>, String)> = BTreeMap::new();
    var_sub.insert("parser".to_string(), Vec::new());

    let root = CommandSpec {
        id: String::new(),
        entry: entry.to_string(),
        path: Vec::new(),
        handler: None,
        aliases: None,
        required: None,
        subcommand_dest: None,
        subcommands_required: None,
        options: Vec::new(),
        mutex_groups: Vec::new(),
    };
    let mut root = root;

    // First pass: parser variables, subparser ownership and mutex groups.
    for (line_start, line) in line_offsets(source) {
        let Some((var, rhs, rhs_start)) = parse_assignment(line_start, line) else {
            continue;
        };
        let Some((recv, method, method_pos)) = find_parser_method(rhs) else {
            continue;
        };
        let open = rhs_start
            + method_pos
            + rhs[method_pos..]
                .find('(')
                .ok_or_else(|| format!("missing '(' in parser declaration: {rhs}"))?;
        let (args, _close) = call_args(source, open)?;
        let kwargs = keyword_args(&args);
        match method.as_str() {
            "add_parser" => {
                let Some(name) = args.first().and_then(|a| parse_string_literal(a)) else {
                    continue;
                };
                let parent = match var_sub.get(&recv) {
                    Some(p) => p.clone(),
                    None => match var_cmd.get(&recv) {
                        Some(p) => p.clone(),
                        None => continue,
                    },
                };
                let mut path = parent;
                path.push(name);
                var_cmd.insert(var.clone(), path.clone());
                let (cmd_index, cmd) = ensure_command(&mut commands, &mut index, entry, &path);
                if let Some(Value::Array(items)) =
                    kwargs.get("aliases").and_then(|v| parse_literal(v))
                {
                    cmd.aliases = Some(
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                    );
                }
                if let Some(Value::Bool(b)) = kwargs.get("required").and_then(|v| parse_literal(v))
                {
                    cmd.required = Some(b);
                }
                let _ = cmd_index;
            }
            "add_subparsers" => {
                let parent = if let Some(p) = var_cmd.get(&recv) {
                    p.clone()
                } else if recv == "parser" {
                    Vec::new()
                } else {
                    continue;
                };
                var_sub.insert(var.clone(), parent.clone());
                if parent.is_empty() {
                    if let Some(dest) = kwargs.get("dest").and_then(|v| parse_string_literal(v)) {
                        root.subcommand_dest = Some(dest);
                    }
                    if let Some(Value::Bool(b)) =
                        kwargs.get("required").and_then(|v| parse_literal(v))
                    {
                        root.subcommands_required = Some(b);
                    }
                } else {
                    let (cmd_index, cmd) =
                        ensure_command(&mut commands, &mut index, entry, &parent);
                    if let Some(dest) = kwargs.get("dest").and_then(|v| parse_string_literal(v)) {
                        cmd.subcommand_dest = Some(dest);
                    }
                    if let Some(Value::Bool(b)) =
                        kwargs.get("required").and_then(|v| parse_literal(v))
                    {
                        cmd.subcommands_required = Some(b);
                    }
                    let _ = cmd_index;
                }
            }
            "add_mutually_exclusive_group" => {
                let Some(parent) = var_cmd.get(&recv).cloned() else {
                    continue;
                };
                let (cmd_index, cmd) = ensure_command(&mut commands, &mut index, entry, &parent);
                let required = kwargs
                    .get("required")
                    .and_then(|v| parse_literal(v))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let group_id = format!("g{}", cmd.mutex_groups.len() + 1);
                cmd.mutex_groups.push(MutexGroup {
                    id: group_id.clone(),
                    required,
                });
                var_group.insert(var.clone(), (parent, group_id));
                let _ = cmd_index;
            }
            _ => {}
        }
    }

    // Second pass: options and handlers in source order.
    let mut cursor = 0;
    while let Some(rel) = source[cursor..].find(".add_argument(") {
        let dot = cursor + rel;
        let recv = receiver_before(source, dot).unwrap_or_default();
        let open = dot + ".add_argument".len();
        let (args, close) = call_args(source, open)?;
        let (parent, group) = if let Some((p, g)) = var_group.get(&recv) {
            (p.clone(), Some(g.clone()))
        } else if let Some(p) = var_cmd.get(&recv) {
            (p.clone(), None)
        } else if recv == "parser" {
            (Vec::new(), None)
        } else {
            cursor = close + 1;
            continue;
        };
        let kwargs = keyword_args(&args);
        let mut names = Vec::new();
        let mut positional = Vec::new();
        for arg in &args {
            if argument_is_keyword(arg) {
                continue;
            }
            if let Some(text) = parse_string_literal(arg) {
                if text.starts_with('-') {
                    names.push(text);
                } else {
                    positional.push(text);
                }
            }
        }
        let option = OptionSpec {
            names,
            positional: positional.first().cloned(),
            positionals: positional.clone(),
            dest: kwargs.get("dest").and_then(|v| parse_string_literal(v)),
            action: kwargs
                .get("action")
                .and_then(|v| parse_string_literal(v))
                .unwrap_or_else(default_action),
            choices: kwargs.get("choices").and_then(|v| literal_string_list(v)),
            default: kwargs
                .get("default")
                .and_then(|v| parse_literal(v))
                .unwrap_or(Value::Null),
            required: kwargs
                .get("required")
                .and_then(|v| parse_literal(v))
                .and_then(|v| v.as_bool()),
            nargs: kwargs
                .get("nargs")
                .and_then(|v| parse_literal(v))
                .and_then(|v| v.as_str().map(str::to_string)),
            const_value: kwargs
                .get("const")
                .and_then(|v| parse_literal(v))
                .unwrap_or(Value::Null),
            type_expr: kwargs.get("type").map(|v| v.trim().to_string()),
            mutex: group,
        };
        if parent.is_empty() {
            root.options.push(option);
        } else {
            let (_, command) = ensure_command(&mut commands, &mut index, entry, &parent);
            command.options.push(option);
        }
        cursor = close + 1;
    }

    let mut cursor = 0;
    while let Some(rel) = source[cursor..].find(".set_defaults(") {
        let dot = cursor + rel;
        let recv = receiver_before(source, dot).unwrap_or_default();
        let open = dot + ".set_defaults".len();
        let (args, close) = call_args(source, open)?;
        let kwargs = keyword_args(&args);
        if let Some(handler_expr) = kwargs.get("handler") {
            if let Some(parent) = var_cmd.get(&recv).cloned() {
                let handler = normalize_handler(handler_expr);
                let (_, command) = ensure_command(&mut commands, &mut index, entry, &parent);
                command.handler = Some(handler);
            }
        }
        cursor = close + 1;
    }

    Ok((root, commands))
}

fn ensure_command<'a>(
    commands: &'a mut Vec<CommandSpec>,
    index: &mut BTreeMap<String, usize>,
    entry: &str,
    path: &[String],
) -> (usize, &'a mut CommandSpec) {
    let id = path.join(" ");
    if let Some(position) = index.get(&id) {
        return (*position, &mut commands[*position]);
    }
    let position = commands.len();
    commands.push(CommandSpec {
        id: id.clone(),
        entry: entry.to_string(),
        path: path.to_vec(),
        handler: None,
        aliases: None,
        required: None,
        subcommand_dest: None,
        subcommands_required: None,
        options: Vec::new(),
        mutex_groups: Vec::new(),
    });
    index.insert(id, position);
    (position, &mut commands[position])
}

fn normalize_handler(expr: &str) -> String {
    if expr.contains("run_play_accounts_tui") {
        "run_play_accounts_tui".to_string()
    } else {
        let trimmed = expr.trim();
        if is_identifier(trimmed) {
            trimmed.to_string()
        } else {
            parse_string_literal(trimmed).unwrap_or_else(|| format!("expr:{trimmed}"))
        }
    }
}

fn find_parser_method(rhs: &str) -> Option<(String, String, usize)> {
    let needles = [
        ".add_parser(",
        ".add_subparsers(",
        ".add_mutually_exclusive_group(",
    ];
    for needle in needles {
        if let Some(pos) = rhs.find(needle) {
            let recv = receiver_before(rhs, pos)?;
            let method = needle
                .trim_start_matches('.')
                .trim_end_matches('(')
                .to_string();
            return Some((recv, method, pos));
        }
    }
    None
}

fn receiver_before(text: &str, dot: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut end = dot;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    if start == end {
        None
    } else {
        Some(text[start..end].to_string())
    }
}

/// Returns (variable, right-hand side, absolute source offset of the right-hand side).
fn parse_assignment(line_start: usize, line: &str) -> Option<(String, &str, usize)> {
    let trimmed_start = line.len() - line.trim_start().len();
    let leading = &line[trimmed_start..];
    if leading.is_empty() || leading.starts_with('#') {
        return None;
    }
    let mut char_indices = leading.char_indices().peekable();
    let mut end = 0;
    while let Some((i, c)) = char_indices.peek().copied() {
        if i == 0 {
            if !(c.is_ascii_alphabetic() || c == '_') {
                return None;
            }
        } else if !(c.is_ascii_alphanumeric() || c == '_') {
            end = i;
            break;
        }
        char_indices.next();
        end = i + c.len_utf8();
    }
    if end == 0 {
        return None;
    }
    let var = &leading[..end];
    let rest = &leading[end..];
    let rest_trim = rest.trim_start();
    if !rest_trim.starts_with('=') || rest_trim.starts_with("==") {
        return None;
    }
    let eq = end + (rest.len() - rest_trim.len());
    let rhs_start_in_leading = eq + 1;
    let rhs = leading[rhs_start_in_leading..].trim_start();
    let rhs_offset_in_leading =
        rhs_start_in_leading + (leading[rhs_start_in_leading..].len() - rhs.len());
    Some((
        var.to_string(),
        rhs,
        line_start + trimmed_start + rhs_offset_in_leading,
    ))
}

fn line_offsets(source: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        out.push((start, line.trim_end_matches(['\n', '\r'])));
        start += line.len();
    }
    if start < source.len() {
        out.push((start, &source[start..]));
    }
    out
}

/// Finds the matching ')' for the '(' at `open`; string-aware.
fn find_matching(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn call_args(source: &str, open: usize) -> Result<(Vec<String>, usize), String> {
    let close = find_matching(source, open)
        .ok_or_else(|| format!("unbalanced call starting at byte {open}"))?;
    let inner = &source[open + 1..close];
    Ok((split_top_level(inner, ','), close))
}

fn split_top_level(text: &str, delimiter: char) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'(' || c == b'[' || c == b'{' {
            depth += 1;
        } else if c == b')' || c == b']' || c == b'}' {
            depth -= 1;
        } else if c == delimiter as u8 && depth == 0 {
            let part = text[start..i].trim();
            if !part.is_empty() {
                parts.push(part.to_string());
            }
            start = i + 1;
        }
        i += 1;
    }
    let part = text[start..].trim();
    if !part.is_empty() {
        parts.push(part.to_string());
    }
    parts
}

fn keyword_args(args: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for arg in args {
        if let Some((key, value)) = split_keyword(arg) {
            out.insert(key, value);
        }
    }
    out
}

fn split_keyword(segment: &str) -> Option<(String, String)> {
    let bytes = segment.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'(' || c == b'[' || c == b'{' {
            depth += 1;
        } else if c == b')' || c == b']' || c == b'}' {
            depth -= 1;
        } else if c == b'=' && depth == 0 {
            let key = segment[..i].trim();
            if is_identifier(key) {
                return Some((key.to_string(), segment[i + 1..].trim().to_string()));
            }
            return None;
        }
        i += 1;
    }
    None
}

fn argument_is_keyword(segment: &str) -> bool {
    split_keyword(segment).is_some()
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_string_literal(expr: &str) -> Option<String> {
    let text = expr.trim();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut prefix = String::new();
    while i < bytes.len() && !matches!(bytes[i], b'"' | b'\'') {
        let c = bytes[i];
        if !c.is_ascii_alphabetic() {
            return None;
        }
        prefix.push(c as char);
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let prefix_lower = prefix.to_ascii_lowercase();
    if prefix_lower.contains('f') {
        return None;
    }
    let raw = prefix_lower.contains('r');
    let quote = bytes[i];
    let triple = i + 2 < bytes.len() && bytes[i + 1] == quote && bytes[i + 2] == quote;
    i += if triple { 3 } else { 1 };
    let mut out = String::new();
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if triple {
            if i + 2 < bytes.len()
                && bytes[i] == quote
                && bytes[i + 1] == quote
                && bytes[i + 2] == quote
            {
                return Some(out);
            }
            out.push(c as char);
            i += 1;
            continue;
        }
        if escaped {
            if raw {
                out.push('\\');
                out.push(c as char);
            } else {
                match c {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    other => out.push(other as char),
                }
            }
            escaped = false;
        } else if c == b'\\' {
            if raw {
                out.push('\\');
            } else {
                escaped = true;
            }
        } else if c == quote {
            return Some(out);
        } else {
            out.push(c as char);
        }
        i += 1;
    }
    None
}

fn parse_literal(expr: &str) -> Option<Value> {
    let text = expr.trim();
    if let Some(value) = parse_string_literal(text) {
        return Some(Value::String(value));
    }
    match text {
        "True" => return Some(Value::Bool(true)),
        "False" => return Some(Value::Bool(false)),
        "None" | "" => return Some(Value::Null),
        "[]" => return Some(Value::Array(Vec::new())),
        _ => {}
    }
    if let Ok(value) = text.parse::<i64>() {
        return Some(json!(value));
    }
    if let Ok(value) = text.parse::<f64>() {
        return Some(json!(value));
    }
    if text.starts_with('[') && text.ends_with(']') {
        let inner = &text[1..text.len() - 1];
        let items = split_top_level(inner, ',');
        let values: Vec<Value> = items
            .iter()
            .filter_map(|item| parse_literal(item))
            .collect();
        return Some(Value::Array(values));
    }
    Some(Value::String(format!("expr:{text}")))
}

fn literal_string_list(expr: &str) -> Option<Vec<String>> {
    match parse_literal(expr)? {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                out.push(item.as_str()?.to_string());
            }
            Some(out)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn discover_tests(lab: &Path) -> Result<TestContracts, String> {
    let tests_dir = lab.join("tests");
    let mut files: Vec<PathBuf> = fs::read_dir(&tests_dir)
        .map_err(|e| format!("cannot read {}: {e}", tests_dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("test_") && n.ends_with(".py"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();

    let mut modules = Vec::new();
    let mut summary = TestSummary {
        modules: 0,
        functions: 0,
        cases: 0,
    };
    for file in files {
        let text = read_required(&file)?;
        let mut names: Vec<String> = Vec::new();
        for line in text.lines() {
            if let Some(name) = test_def_name(line) {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        let mut counts: BTreeMap<String, usize> = names.iter().cloned().map(|n| (n, 1)).collect();
        let mut cursor = 0usize;
        while let Some(rel) = text[cursor..].find("@pytest.mark.parametrize") {
            let marker = cursor + rel;
            let Some(open_rel) = text[marker..].find('(') else {
                break;
            };
            let open = marker + open_rel;
            let (args, close) = call_args(&text, open)?;
            if args.len() >= 2 {
                if let Some(target) = first_test_after(&text, close + 1) {
                    if let Some(Value::Array(items)) = parse_literal(&args[1]) {
                        let entry = counts.entry(target).or_insert(1);
                        *entry *= items.len();
                    }
                }
            }
            cursor = close + 1;
        }
        let mut cases = Vec::new();
        for name in &names {
            let count = *counts.get(name).unwrap_or(&1);
            cases.push(TestCase {
                name: name.clone(),
                cases: count,
                destination: String::new(),
                destination_status: String::new(),
            });
        }
        summary.modules += 1;
        summary.functions += cases.len();
        summary.cases += cases.iter().map(|t| t.cases).sum::<usize>();
        modules.push(TestModule {
            path: format!("tests/{}", file.file_name().unwrap().to_string_lossy()),
            interfaces: apk_lab_tokens(&text),
            tests: cases,
        });
    }

    Ok(TestContracts { summary, modules })
}

fn test_def_name(line: &str) -> Option<String> {
    let rest = if let Some(rest) = line.strip_prefix("def ") {
        rest
    } else {
        line.strip_prefix("async def ")?
    };
    let rest = rest.trim_start();
    if !rest.starts_with("test_") {
        return None;
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn first_test_after(text: &str, offset: usize) -> Option<String> {
    for line in text[offset..].lines() {
        if let Some(name) = test_def_name(line) {
            return Some(name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Consumers
// ---------------------------------------------------------------------------

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".venv",
    "node_modules",
    "target",
    "__pycache__",
    ".pytest_cache",
    ".apk-lab-migration",
    "plans",
    ".apk-lab",
];

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "zip", "apk", "apkm", "xapk", "apks", "jar", "mpp", "so", "dex", "lock",
    "woff", "woff2", "ico", "svg",
];

fn discover_consumers(root: &Path) -> Result<Vec<Consumer>, String> {
    let mut consumers = Vec::new();
    walk_consumers(root, root, &mut consumers)?;
    consumers.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(consumers)
}

fn walk_consumers(root: &Path, dir: &Path, out: &mut Vec<Consumer>) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk_consumers(root, &path, out)?;
            continue;
        }
        if name.contains(".local.") || name.ends_with(".local.json") {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if BINARY_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .map_err(|e| format!("cannot relativize {}: {e}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded_consumer(&relative) || is_benchmark_evidence(&relative, &text) {
            continue;
        }
        let interfaces = apk_lab_tokens(&text);
        if interfaces.is_empty() {
            continue;
        }
        out.push(Consumer {
            path: relative,
            interfaces,
            classification: String::new(),
            supported: false,
            notes: String::new(),
        });
    }
    Ok(())
}

/// Generated and historical benchmark evidence under `docs/benchmarks/apk-lab`
/// records module names, but it is oracle data rather than a supported
/// executable caller. Recognition is deliberately narrow: a schemaVersion 1
/// JSON document in that directory whose kind is a known evidence kind. The
/// Python contracts capture additionally requires its declared engine/profile
/// identity. Unknown kinds, runtime configuration, malformed data, and identity
/// mismatches stay discoverable.
fn is_benchmark_evidence(relative: &str, text: &str) -> bool {
    const DIR: &str = "docs/benchmarks/apk-lab/";
    let Some(name) = relative.strip_prefix(DIR) else {
        return false;
    };
    if name.is_empty() || name.contains('/') || !name.ends_with(".json") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return false;
    }
    match value.get("kind").and_then(Value::as_str) {
        Some("apk-lab-python-contracts") => {
            value.get("engine").and_then(Value::as_str) == Some("python")
                && value.get("profile").and_then(Value::as_str) == Some("offline")
        }
        Some(
            "apk-lab-migration-execution-checkpoint"
            | "apk-lab-benchmark-result"
            | "apk-lab-benchmark-output-pins"
            | "apk-lab-mislabeled-container-observation",
        ) => true,
        _ => false,
    }
}

fn is_excluded_consumer(path: &str) -> bool {
    path.starts_with("tools/apk-lab/apk_lab/")
        || path.starts_with("tools/apk-lab/tests/")
        || path.starts_with("tools/apk-lab/.venv/")
        || path.starts_with("tools/apk-lab/src/migration/")
        || path.starts_with("tools/apk-lab/migration/")
}

/// Extracts `apk_lab` and dotted module references with word boundaries.
fn apk_lab_tokens(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let needle: &[u8] = b"apk_lab";
    let mut found = BTreeSet::new();
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let previous_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if !previous_ok {
            i += needle.len();
            continue;
        }
        let mut end = i + needle.len();
        if end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            i = end;
            continue;
        }
        let mut token = String::from("apk_lab");
        while end < bytes.len() && bytes[end] == b'.' {
            let mut cursor = end + 1;
            if cursor >= bytes.len()
                || !(bytes[cursor].is_ascii_alphabetic() || bytes[cursor] == b'_')
            {
                break;
            }
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
            token.push_str(&text[end..cursor]);
            end = cursor;
        }
        if end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            i = end;
            continue;
        }
        found.insert(token);
        i = end;
    }
    found.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Wire schemas
// ---------------------------------------------------------------------------

fn discover_wire(lab: &Path) -> Result<Vec<WireSchema>, String> {
    let mut schemas = Vec::new();
    let models_path = lab.join("apk_lab/models.py");
    if let Ok(models) = fs::read_to_string(&models_path) {
        for (name, block) in top_blocks(&models) {
            let Some(name) = name else { continue };
            if let Some(body) = method_block(&block, "to_dict") {
                schemas.push(WireSchema {
                    id: format!("models.{name}"),
                    source: "tools/apk-lab/apk_lab/models.py".to_string(),
                    kind: "to_dict".to_string(),
                    fields: string_keys(&body),
                    serialization: Value::Null,
                    notes: String::new(),
                });
            }
        }
    }

    let cli_path = lab.join("apk_lab/cli.py");
    if let Ok(cli) = fs::read_to_string(&cli_path) {
        for function in ["handle_strings", "handle_manifest", "handle_ui_dump"] {
            if let Some(body) = top_function_block(&cli, function) {
                schemas.push(WireSchema {
                    id: format!("cli.{function}"),
                    source: "tools/apk-lab/apk_lab/cli.py".to_string(),
                    kind: "string_keys".to_string(),
                    fields: string_keys(&body),
                    serialization: Value::Null,
                    notes: String::new(),
                });
            }
        }
    }

    let ci_path = lab.join("apk_lab/compatibility_ci.py");
    if let Ok(ci) = fs::read_to_string(&ci_path) {
        for function in ["build_error_result", "build_result_item", "main"] {
            if let Some(body) = top_function_block(&ci, function) {
                schemas.push(WireSchema {
                    id: format!("compatibility_ci.{function}"),
                    source: "tools/apk-lab/apk_lab/compatibility_ci.py".to_string(),
                    kind: "string_keys".to_string(),
                    fields: string_keys(&body),
                    serialization: Value::Null,
                    notes: String::new(),
                });
            }
        }
    }

    for (file, class) in [
        ("device.py", "CrashReport"),
        ("device.py", "ResolvedFrame"),
        ("manifest_inspection.py", "ManifestComponent"),
        ("dex_strings.py", "DexStringMatch"),
    ] {
        let path = lab.join("apk_lab").join(file);
        if let Ok(source) = fs::read_to_string(&path) {
            for (name, block) in top_blocks(&source) {
                if name.as_deref() == Some(class) {
                    schemas.push(WireSchema {
                        id: format!("{}.{}", file.trim_end_matches(".py"), class),
                        source: format!("tools/apk-lab/apk_lab/{file}"),
                        kind: "dataclass_fields".to_string(),
                        fields: dataclass_fields(&block),
                        serialization: Value::Null,
                        notes: String::new(),
                    });
                }
            }
        }
    }

    Ok(schemas)
}

fn top_blocks(text: &str) -> Vec<(Option<String>, Vec<String>)> {
    let mut blocks: Vec<(Option<String>, Vec<String>)> = Vec::new();
    let mut current: Option<(Option<String>, Vec<String>)> = None;
    for line in text.lines() {
        let first_is_space = line
            .chars()
            .next()
            .map(|c| c.is_whitespace())
            .unwrap_or(false);
        let boundary = !line.is_empty() && !first_is_space;
        if boundary {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            if line.starts_with("class ") {
                let name = line.strip_prefix("class ").and_then(|rest| {
                    rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .map(|end| rest[..end].to_string())
                });
                current = Some((name, vec![line.to_string()]));
            } else {
                current = Some((None, vec![line.to_string()]));
            }
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }
    blocks
}

fn method_block(block: &[String], name: &str) -> Option<Vec<String>> {
    let needle = format!("def {name}");
    for (index, line) in block.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with(needle.as_str())
            || !trimmed[needle.len()..].trim_start().starts_with('(')
        {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let mut body = Vec::new();
        for next in &block[index + 1..] {
            if !next.trim().is_empty() && indentation(next) <= indent {
                break;
            }
            body.push(next.clone());
        }
        return Some(body);
    }
    None
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn top_function_block(source: &str, name: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = source.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix("def ") {
            let candidate = rest
                .split(|c: char| c == '(' || c.is_whitespace())
                .next()
                .unwrap_or("");
            if candidate == name {
                let mut body = vec![(*line).to_string()];
                for next in &lines[index + 1..] {
                    if next.starts_with("def ") || next.starts_with("class ") {
                        break;
                    }
                    body.push((*next).to_string());
                }
                return Some(body);
            }
        }
    }
    None
}

fn string_keys(block: &[String]) -> Vec<String> {
    let text = block.join("\n");
    let bytes = text.as_bytes();
    let mut keys = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'"' && bytes[i] != b'\'' {
            i += 1;
            continue;
        }
        let quote = bytes[i];
        let mut j = i + 1;
        let mut escaped = false;
        let mut value = Vec::new();
        while j < bytes.len() {
            let c = bytes[j];
            if escaped {
                value.push(c);
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == quote {
                break;
            } else {
                value.push(c);
            }
            j += 1;
        }
        if j + 1 < bytes.len() && bytes[j + 1] == b':' {
            let key = String::from_utf8_lossy(&value).to_string();
            if is_json_key(&key) && !keys.contains(&key) {
                keys.push(key);
            }
        }
        i = j + 1;
    }
    keys
}

fn is_json_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn dataclass_fields(block: &[String]) -> Vec<String> {
    let mut fields = Vec::new();
    for line in block {
        let trimmed = line.strip_prefix("    ").unwrap_or("");
        let starts_with_space = trimmed
            .chars()
            .next()
            .map(|c| c.is_whitespace())
            .unwrap_or(false);
        if trimmed.len() == line.len() || starts_with_space {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let name = trimmed[..colon].trim();
        if is_identifier(name) && !fields.iter().any(|f| f == name) {
            fields.push(name.to_string());
        }
    }
    fields
}

// ---------------------------------------------------------------------------
// Exit classes, acquisition sources, entrypoints
// ---------------------------------------------------------------------------

fn discover_exit_classes(lab: &Path) -> Result<Vec<ExitClass>, String> {
    let source = fs::read_to_string(lab.join("apk_lab/models.py")).unwrap_or_default();
    for (name, block) in top_blocks(&source) {
        if name.as_deref() != Some("ExitCode") {
            continue;
        }
        let mut out = Vec::new();
        for line in block {
            let trimmed = line.trim();
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if !is_identifier(key)
                || !key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                continue;
            }
            if let Ok(code) = value.parse::<i64>() {
                out.push(ExitClass {
                    name: key.to_string(),
                    code,
                    meaning: String::new(),
                });
            }
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

fn discover_acquisition_sources(lab: &Path) -> Result<Vec<AcquisitionSource>, String> {
    let source = fs::read_to_string(lab.join("apk_lab/acquisition.py")).unwrap_or_default();
    for (name, block) in top_blocks(&source) {
        if name.as_deref() != Some("AcquisitionSource") {
            continue;
        }
        let mut out = Vec::new();
        for line in block {
            let trimmed = line.trim();
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if !is_identifier(key) || !key.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                continue;
            }
            if let Some(label) = parse_string_literal(value) {
                out.push(AcquisitionSource {
                    name: key.to_string(),
                    label,
                    interface: String::new(),
                    meaning: String::new(),
                    destination: String::new(),
                });
            }
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

fn discover_entrypoints(lab: &Path) -> Result<Vec<Entrypoint>, String> {
    let mut out = Vec::new();
    if let Ok(pyproject) = fs::read_to_string(lab.join("pyproject.toml")) {
        let mut section = String::new();
        for line in pyproject.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                section = trimmed.to_string();
                continue;
            }
            if section != "[project.scripts]" {
                continue;
            }
            let Some((name, target)) = trimmed.split_once('=') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                continue;
            }
            if let Some(target) = parse_string_literal(target) {
                out.push(Entrypoint {
                    kind: "console-script".to_string(),
                    name: name.to_string(),
                    target,
                    notes: String::new(),
                });
            }
        }
    }

    let package = lab.join("apk_lab");
    if let Ok(entries) = fs::read_dir(&package) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            if has_main_guard(&text) {
                let stem = file.file_stem().unwrap_or_default().to_string_lossy();
                out.push(Entrypoint {
                    kind: "module-main".to_string(),
                    name: format!("apk_lab.{stem}"),
                    target: format!("python -m apk_lab.{stem}"),
                    notes: String::new(),
                });
            }
        }
    }
    Ok(out)
}

fn has_main_guard(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("if __name__")
            && trimmed.contains("\"__main__\"")
            && trimmed.ends_with(':')
    })
}

// ---------------------------------------------------------------------------
// Python-only regular expressions
// ---------------------------------------------------------------------------

fn discover_python_only_regexes(lab: &Path) -> Result<Vec<RegexEntry>, String> {
    let package = lab.join("apk_lab");
    let Ok(entries) = fs::read_dir(&package) else {
        return Ok(Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
        .collect();
    files.sort();

    let mut out = Vec::new();
    for file in files {
        let text = read_required(&file)?;
        let source = format!(
            "tools/apk-lab/apk_lab/{}",
            file.file_name().unwrap_or_default().to_string_lossy()
        );
        for op in [
            "compile",
            "match",
            "fullmatch",
            "search",
            "findall",
            "finditer",
            "split",
            "sub",
            "subn",
        ] {
            let needle = format!("re.{op}(");
            let mut cursor = 0usize;
            while let Some(rel) = text[cursor..].find(needle.as_str()) {
                let position = cursor + rel;
                let open = position + "re.".len() + op.len();
                let (args, close) = call_args(&text, open)?;
                cursor = close + 1;
                if args.is_empty() {
                    continue;
                }
                let first = args[0].trim();
                if first.len() >= 2 {
                    let prefix: String = first
                        .chars()
                        .take_while(|c| c.is_ascii_alphabetic())
                        .collect::<String>()
                        .to_ascii_lowercase();
                    if prefix.contains('f') {
                        continue;
                    }
                }
                let Some(pattern) = parse_string_literal(first) else {
                    continue;
                };
                let features = python_only_features(&pattern);
                if features.is_empty() {
                    continue;
                }
                let function = function_before(&text, position);
                out.push(RegexEntry {
                    source: source.clone(),
                    function,
                    op: op.to_string(),
                    pattern,
                    python_only_features: features,
                    behavior: String::new(),
                    destination: String::new(),
                });
            }
        }
    }
    Ok(out)
}

fn function_before(text: &str, position: usize) -> String {
    let mut current = "<module>".to_string();
    for line in text[..position].lines() {
        if let Some(rest) = line.strip_prefix("def ") {
            let name = rest
                .split(|c: char| c == '(' || c.is_whitespace())
                .next()
                .unwrap_or("");
            if !name.is_empty() {
                current = name.to_string();
            }
        }
    }
    current
}

fn python_only_features(pattern: &str) -> Vec<String> {
    let mut features = Vec::new();
    if pattern.contains("(?=") {
        features.push("lookahead".to_string());
    }
    if pattern.contains("(?!") {
        features.push("negative-lookahead".to_string());
    }
    if pattern.contains("(?<=") {
        features.push("lookbehind".to_string());
    }
    if pattern.contains("(?<!") {
        features.push("negative-lookbehind".to_string());
    }
    if pattern.contains("\\Z") {
        features.push("end-of-string-Z".to_string());
    }
    if has_numeric_backreference(pattern) {
        features.push("backreference".to_string());
    }
    if pattern.contains("(?P=") {
        features.push("named-backreference".to_string());
    }
    if pattern.contains("(?>") {
        features.push("atomic-group".to_string());
    }
    if has_possessive_quantifier(pattern) {
        features.push("possessive-quantifier".to_string());
    }
    if pattern.contains("\\G") {
        features.push("G-anchor".to_string());
    }
    features
}

fn has_numeric_backreference(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1].is_ascii_digit() && bytes[i + 1] != b'0' {
            return true;
        }
        i += 1;
    }
    false
}

fn has_possessive_quantifier(pattern: &str) -> bool {
    pattern.contains("*+")
        || pattern.contains("++")
        || pattern.contains("?+")
        || pattern.contains("}+")
}

// ---------------------------------------------------------------------------
// Tests (cargo)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migration/fixtures/inventory")
    }

    fn fixture_root() -> PathBuf {
        fixture_dir().join(".source-repo")
    }

    fn fixture_contracts() -> Contracts {
        load_contracts(&fixture_dir().join("contracts.json"))
            .expect("load portable fixture contract")
    }

    fn issues_for(contracts: &Contracts) -> Vec<String> {
        validate(
            &discover_all(&fixture_root()).expect("discover portable fixture"),
            contracts,
        )
    }

    #[test]
    fn portable_fixture_inventory_is_current() {
        let issues = issues_for(&fixture_contracts());
        assert!(issues.is_empty(), "unexpected issues: {issues:#?}");
    }

    #[test]
    fn portable_fixture_manifest_hashes_match() {
        let dir = fixture_dir();
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("manifest.json")).expect("read manifest"),
        )
        .expect("parse manifest");
        assert_eq!(manifest["synthetic"].as_bool(), Some(true));
        assert_eq!(manifest["containsPrivateData"].as_bool(), Some(false));

        let mut listed = BTreeSet::new();
        for entry in manifest["files"].as_array().expect("files array") {
            let relative = entry["path"].as_str().expect("file path");
            listed.insert(relative.to_string());
            let bytes = fs::read(dir.join(relative))
                .unwrap_or_else(|e| panic!("read fixture {relative}: {e}"));
            assert_eq!(
                entry["bytes"].as_u64(),
                Some(bytes.len() as u64),
                "size mismatch for {relative}"
            );
            let digest = format!("{:x}", Sha256::digest(&bytes));
            assert_eq!(
                entry["sha256"].as_str(),
                Some(digest.as_str()),
                "hash mismatch for {relative}"
            );
        }

        let mut actual = BTreeSet::new();
        collect_fixture_files(&dir, &dir, &mut actual);
        actual.remove("manifest.json");
        assert_eq!(
            listed, actual,
            "manifest must list every durable fixture file exactly once"
        );
    }

    fn collect_fixture_files(base: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).expect("read fixture directory") {
            let entry = entry.expect("fixture entry");
            let path = entry.path();
            if path.is_dir() {
                collect_fixture_files(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base)
                        .expect("fixture containment")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    #[test]
    fn missing_command_mapping_is_rejected() {
        let mut contracts = fixture_contracts();
        contracts.commands.pop();
        let issues = issues_for(&contracts);
        assert!(
            issues.iter().any(|i| i.contains("missing command mapping")),
            "expected missing command issue: {issues:#?}"
        );
    }

    #[test]
    fn missing_option_mapping_is_rejected() {
        let mut contracts = fixture_contracts();
        contracts.commands[0].spec.options.pop();
        let issues = issues_for(&contracts);
        assert!(
            issues.iter().any(|i| i.contains("missing option mapping")),
            "expected missing option issue: {issues:#?}"
        );
    }

    #[test]
    fn changed_option_default_is_rejected() {
        let mut contracts = fixture_contracts();
        contracts.commands[0].spec.options[1].default = json!("changed");
        let issues = issues_for(&contracts);
        assert!(
            issues.iter().any(|i| i.contains("default")),
            "expected default mismatch: {issues:#?}"
        );
    }

    #[test]
    fn missing_test_mapping_is_rejected() {
        let mut contracts = fixture_contracts();
        contracts.tests.as_mut().unwrap().modules[0].tests.pop();
        let issues = issues_for(&contracts);
        assert!(
            issues.iter().any(|i| i.contains("missing test mapping")),
            "expected missing test issue: {issues:#?}"
        );
    }

    #[test]
    fn benchmark_evidence_is_not_a_consumer_unless_kind_or_identity_changes() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        let evidence_dir = root.join("docs/benchmarks/apk-lab");
        fs::create_dir_all(&evidence_dir).expect("create evidence dir");

        let python_contracts = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-python-contracts",
            "engine": "python",
            "profile": "offline",
            "python": { "module": "apk_lab.compatibility_ci" },
        });
        fs::write(
            evidence_dir.join("python-contracts.json"),
            python_contracts.to_string(),
        )
        .expect("write python contracts evidence");

        let checkpoint = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-migration-execution-checkpoint",
            "status": "in_progress",
            "commands": ["apk_lab.cli", "apk_lab.compatibility_ci"],
            "modules": ["apk_lab.compatibility_ci"],
        });
        fs::write(
            evidence_dir.join("execution-status.json"),
            checkpoint.to_string(),
        )
        .expect("write checkpoint evidence");

        let benchmark_result = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-result",
            "engine": "python",
            "source": { "entrypoint": "apk_lab.cli bench" },
            "measurements": [{ "module": "apk_lab.compatibility_ci" }],
        });
        fs::write(
            evidence_dir.join("baseline.json"),
            benchmark_result.to_string(),
        )
        .expect("write benchmark result evidence");

        let pins = json!({
            "schemaVersion": 1,
            "kind": "apk-lab-benchmark-output-pins",
            "rows": [{ "note": "prepared via apk_lab.cli" }],
        });
        fs::write(evidence_dir.join("pins.json"), pins.to_string())
            .expect("write output pin evidence");

        let consumer_path = root.join("tools/consumer/run.py");
        fs::create_dir_all(consumer_path.parent().expect("consumer parent"))
            .expect("create consumer dir");
        fs::write(&consumer_path, "import apk_lab.compatibility_ci\n").expect("write consumer");

        let consumers = discover_consumers(root).expect("discover consumers");
        let paths: Vec<&str> = consumers.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, vec!["tools/consumer/run.py"], "{consumers:#?}");
        assert_eq!(
            consumers[0].interfaces,
            vec!["apk_lab.compatibility_ci".to_string()]
        );

        let mut runtime_config = checkpoint.clone();
        runtime_config["kind"] = json!("runtime-config");
        fs::write(
            evidence_dir.join("execution-status.json"),
            runtime_config.to_string(),
        )
        .expect("rewrite checkpoint as runtime config");
        let consumers = discover_consumers(root).expect("rediscover consumers");
        let paths: Vec<&str> = consumers.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "docs/benchmarks/apk-lab/execution-status.json",
                "tools/consumer/run.py"
            ],
            "{consumers:#?}"
        );

        let mut wrong_identity = python_contracts.clone();
        wrong_identity["engine"] = json!("node");
        fs::write(
            evidence_dir.join("python-contracts.json"),
            wrong_identity.to_string(),
        )
        .expect("rewrite python contracts identity");
        let consumers = discover_consumers(root).expect("rediscover consumers");
        let paths: Vec<&str> = consumers.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "docs/benchmarks/apk-lab/execution-status.json",
                "docs/benchmarks/apk-lab/python-contracts.json",
                "tools/consumer/run.py"
            ],
            "{consumers:#?}"
        );
    }

    #[test]
    fn missing_consumer_mapping_is_rejected() {
        let mut contracts = fixture_contracts();
        contracts.consumers.pop();
        let issues = issues_for(&contracts);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("missing consumer mapping")),
            "expected missing consumer issue: {issues:#?}"
        );
    }

    #[test]
    fn stale_command_mapping_is_rejected() {
        let mut contracts = fixture_contracts();
        let mut stale = contracts.commands[0].clone();
        stale.spec.id = "removed".to_string();
        stale.spec.path = vec!["removed".to_string()];
        contracts.commands.push(stale);
        let issues = issues_for(&contracts);
        assert!(
            issues.iter().any(|i| i.contains("stale command mapping")),
            "expected stale command issue: {issues:#?}"
        );
    }

    #[test]
    fn repository_contract_is_current() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("repository root");
        if std::env::var("APK_LAB_INVENTORY_SKIP_REPO_CHECK").as_deref() == Ok("1") {
            return;
        }
        let contracts_path = manifest_dir.join("migration/contracts.json");
        let contracts = load_contracts(&contracts_path).expect("load repository contracts");
        let discovery = discover_all(root).expect("discover repository");
        let issues = validate(&discovery, &contracts);
        assert!(issues.is_empty(), "repository inventory drift: {issues:#?}");
    }
}
