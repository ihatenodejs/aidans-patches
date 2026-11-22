//! Interactive Google Play account management.
//!
//! Native port of `apk_lab.play_accounts_tui`.
//!
//! All interaction goes through the [`Ui`] trait so the flows can be driven by
//! a synthetic terminal in tests. [`TerminalUi`] is the dialoguer-backed
//! implementation used by the CLI; dialoguer is MIT licensed, maintained by
//! console-rs, and declares Rust 1.66 as its MSRV.
//!
//! Secrets are never placed in command arguments for GitHub sync: the accounts
//! document is written to `gh`'s stdin. Diagnostics redact tokens before they
//! are truncated or displayed.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

use crate::accounts::{
    load_accounts, save_accounts, Account, AccountsError, AccountsFile, DEFAULT_DEVICE,
    DEFAULT_LOCALE, DEFAULT_TIMEZONE,
};

pub const CHECK_PACKAGE: &str = "com.adobe.scan.android";
pub const DEFAULT_SECRET_REPOSITORY: &str = "ihatenodejs/aidans-patches";
pub const SECRET_NAME: &str = "APKEEP_ACCOUNTS";
const DIAGNOSTIC_LIMIT: usize = 300;

const MENU: [&str; 6] = [
    "View accounts",
    "Add account",
    "Delete account",
    "Check accounts",
    "Sync GitHub Actions secret",
    "Exit",
];

/// Terminal or domain failure surfaced to the CLI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiError {
    message: String,
}

impl TuiError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TuiError {}

impl From<AccountsError> for TuiError {
    fn from(error: AccountsError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<io::Error> for TuiError {
    fn from(error: io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<dialoguer::Error> for TuiError {
    fn from(error: dialoguer::Error) -> Self {
        Self::new(error.to_string())
    }
}

/// Prompt surface. `None` means the user cancelled or went back.
pub trait Ui {
    fn text(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>, TuiError>;
    fn password(&mut self, prompt: &str) -> Result<Option<String>, TuiError>;
    fn select(&mut self, prompt: &str, choices: &[String]) -> Result<Option<usize>, TuiError>;
    fn confirm(&mut self, prompt: &str, default: bool) -> Result<Option<bool>, TuiError>;
    fn message(&mut self, text: &str);
}

/// Metadata probe used by "Check accounts". Implementations perform a real
/// authenticated Play metadata request; local file validation is not a check.
pub trait AccountChecker {
    fn check(&self, account: &Account, package: &str) -> Result<MetadataVersion, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataVersion {
    pub package_name: String,
    pub version_name: String,
    pub version_code: u64,
}

/// Production checker backed by gpapi, the same provider `play-metadata` uses.
#[derive(Debug, Default, Clone, Copy)]
pub struct GpapiChecker;

impl AccountChecker for GpapiChecker {
    fn check(&self, account: &Account, package: &str) -> Result<MetadataVersion, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("Cannot start metadata runtime: {error}"))?;
        runtime.block_on(async {
            let mut api = gpapi::Gpapi::new(&account.device, &account.email);
            api.set_locale(&account.locale);
            api.set_timezone(&account.timezone);
            api.set_aas_token(&account.aas_token);
            api.login()
                .await
                .map_err(|error| format!("Login failed: {error}"))?;
            match api.details(package).await {
                Ok(Some(details)) => {
                    let app = details
                        .item
                        .and_then(|item| item.details)
                        .and_then(|details| details.app_details);
                    match app {
                        Some(app) => {
                            let version_name = app.version_string.unwrap_or_default();
                            let version_code = app.version_code.unwrap_or(0);
                            if version_name.trim().is_empty() || version_code <= 0 {
                                return Err(
                                    "Missing version_string or positive version_code in response"
                                        .to_string(),
                                );
                            }
                            Ok(MetadataVersion {
                                package_name: package.to_string(),
                                version_name,
                                version_code: version_code as u64,
                            })
                        }
                        None => Err("Empty details response".to_string()),
                    }
                }
                Ok(None) => Err("Empty details response".to_string()),
                Err(error) => Err(format!("Details call failed: {error}")),
            }
        })
    }
}

/// dialoguer-backed terminal prompts.
#[derive(Debug, Default, Clone, Copy)]
pub struct TerminalUi;

fn cancelled<T>(error: dialoguer::Error) -> Result<Option<T>, TuiError> {
    match error {
        dialoguer::Error::IO(io) if is_cancellation(&io) => Ok(None),
        dialoguer::Error::IO(io) => Err(TuiError::from(io)),
    }
}

fn is_cancellation(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof
    )
}

impl Ui for TerminalUi {
    fn text(&mut self, prompt: &str, default: Option<&str>) -> Result<Option<String>, TuiError> {
        let mut input = dialoguer::Input::<String>::new()
            .with_prompt(prompt)
            .allow_empty(true);
        if let Some(default) = default {
            input = input.default(default.to_string());
        }
        input.interact_text().map(Some).or_else(cancelled)
    }

    fn password(&mut self, prompt: &str) -> Result<Option<String>, TuiError> {
        dialoguer::Password::new()
            .with_prompt(prompt)
            .allow_empty_password(true)
            .interact()
            .map(Some)
            .or_else(cancelled)
    }

    fn select(&mut self, prompt: &str, choices: &[String]) -> Result<Option<usize>, TuiError> {
        dialoguer::Select::new()
            .with_prompt(prompt)
            .items(choices)
            .default(0)
            .interact_opt()
            .or_else(cancelled)
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<Option<bool>, TuiError> {
        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(default)
            .interact_opt()
            .or_else(cancelled)
    }

    fn message(&mut self, text: &str) {
        println!("{text}");
    }
}

/// Runs the menu until exit or cancellation.
pub fn run<U: Ui, C: AccountChecker>(ui: &mut U, checker: &C) -> Result<(), TuiError> {
    let mut accounts = match load_accounts(false) {
        Ok(accounts) => accounts,
        Err(error) => {
            ui.message(&format!(
                "Warning: Could not load existing accounts: {error}"
            ));
            None
        }
    };

    loop {
        let choices: Vec<String> = MENU.iter().map(|choice| (*choice).to_string()).collect();
        let Some(choice) = ui.select("Google Play Accounts Management:", &choices)? else {
            ui.message("Exiting.");
            break;
        };
        match choice {
            0 => view_accounts(ui, accounts.as_ref()),
            1 => match add_account(ui, accounts.as_ref()) {
                Ok(Some(updated)) => accounts = Some(updated),
                Ok(None) => {}
                Err(error) => ui.message(&format!("Error: {error}")),
            },
            2 => match delete_account(ui, accounts.as_ref()) {
                Ok(Some(updated)) => accounts = Some(updated),
                Ok(None) => {}
                Err(error) => ui.message(&format!("Error: {error}")),
            },
            3 => check_accounts(ui, accounts.as_ref(), checker),
            4 => {
                if let Err(error) =
                    sync_github_secret(ui, accounts.as_ref(), &SyncOptions::default())
                {
                    ui.message(&format!("Error: {error}"));
                }
            }
            5 => {
                ui.message("Exiting.");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Runs the terminal TUI with the production metadata checker.
pub fn run_interactive() -> Result<(), TuiError> {
    let mut ui = TerminalUi;
    run(&mut ui, &GpapiChecker)
}

/// Compatibility entry point name used by the migration inventory.
pub fn run_play_accounts_tui<U: Ui, C: AccountChecker>(
    ui: &mut U,
    checker: &C,
) -> Result<(), TuiError> {
    run(ui, checker)
}

pub fn view_accounts<U: Ui>(ui: &mut U, accounts: Option<&AccountsFile>) {
    let Some(accounts) = accounts.filter(|accounts| !accounts.accounts.is_empty()) else {
        ui.message("No Google Play accounts configured.");
        return;
    };

    ui.message("Configured Google Play Accounts:");
    ui.message("============================================================");
    for (index, account) in accounts.accounts.iter().enumerate() {
        ui.message(&format!("{}. {}", index + 1, account.redacted_summary()));
        ui.message(&format!(
            "   Device: {} | Locale: {} | Timezone: {}",
            account.device, account.locale, account.timezone
        ));
        ui.message("   Token: [REDACTED]");
    }
    ui.message("============================================================");
}

/// Adds an account. `Ok(None)` means the flow was cancelled and nothing changed.
pub fn add_account<U: Ui>(
    ui: &mut U,
    accounts: Option<&AccountsFile>,
) -> Result<Option<AccountsFile>, TuiError> {
    ui.message("--- Add Google Play Account ---");
    let Some(label) = ui.text("Account Label (e.g. personal, work):", None)? else {
        ui.message("Canceled: label required.");
        return Ok(None);
    };
    if label.trim().is_empty() {
        ui.message("Canceled: label required.");
        return Ok(None);
    }

    let Some(email) = ui.text("Account Email:", None)? else {
        ui.message("Canceled: email required.");
        return Ok(None);
    };
    if email.trim().is_empty() {
        ui.message("Canceled: email required.");
        return Ok(None);
    }

    let Some(device) = ui.text("Device profile:", Some(DEFAULT_DEVICE))? else {
        ui.message("Canceled.");
        return Ok(None);
    };
    let device = if device.trim().is_empty() {
        DEFAULT_DEVICE.to_string()
    } else {
        device.trim().to_string()
    };

    let Some(locale) = ui.text("Locale:", Some(DEFAULT_LOCALE))? else {
        ui.message("Canceled.");
        return Ok(None);
    };
    let locale = if locale.trim().is_empty() {
        DEFAULT_LOCALE.to_string()
    } else {
        locale.trim().to_string()
    };

    let Some(timezone) = ui.text("Timezone:", Some(DEFAULT_TIMEZONE))? else {
        ui.message("Canceled.");
        return Ok(None);
    };
    let timezone = if timezone.trim().is_empty() {
        DEFAULT_TIMEZONE.to_string()
    } else {
        timezone.trim().to_string()
    };

    ui.message("Google OAuth Exchange Setup:");
    ui.message("1. Go to https://accounts.google.com/EmbeddedSetup to obtain an OAuth token.");
    ui.message("2. Retrieve the master oauth_token from the flow.");

    let methods = vec![
        "Exchange one-use OAuth token via apkeep".to_string(),
        "Enter pre-generated AAS token directly".to_string(),
    ];
    let Some(method) = ui.select("Token input method:", &methods)? else {
        ui.message("Canceled.");
        return Ok(None);
    };

    let aas_token = if method == 0 {
        let Some(oauth_token) = ui.password("Enter one-use OAuth token (masked):")? else {
            ui.message("Canceled: OAuth token required.");
            return Ok(None);
        };
        if oauth_token.trim().is_empty() {
            ui.message("Canceled: OAuth token required.");
            return Ok(None);
        }
        match exchange_oauth_for_aas(ui, email.trim(), oauth_token.trim())? {
            Some(token) => token,
            None => return Ok(None),
        }
    } else {
        let Some(token) = ui.password("Enter AAS token (masked):")? else {
            ui.message("Canceled: AAS token required.");
            return Ok(None);
        };
        if token.trim().is_empty() {
            ui.message("Canceled: AAS token required.");
            return Ok(None);
        }
        token.trim().to_string()
    };

    let candidate = Account::from_value(&json!({
        "label": label.trim(),
        "email": email.trim().to_lowercase(),
        "aasToken": aas_token,
        "device": device,
        "locale": locale,
        "timezone": timezone,
        "enabled": true,
    }))?;

    let mut updated_accounts = accounts.map_or_else(Vec::new, |file| file.accounts.clone());
    if let Some(existing) = updated_accounts
        .iter()
        .find(|account| account.email == candidate.email)
    {
        return Err(TuiError::new(format!(
            "Account with email {} already exists (label [{}])",
            candidate.email, existing.label
        )));
    }
    updated_accounts.push(candidate.clone());
    let updated = AccountsFile::new(updated_accounts);
    let destination = save_accounts(&updated, None)?;
    ui.message(&format!(
        "Account [{}] saved to {} (mode 0600).",
        candidate.label,
        destination.display()
    ));
    Ok(Some(updated))
}

/// Deletes one account after an explicit confirmation. `Ok(None)` means the
/// user cancelled or there was nothing to delete.
pub fn delete_account<U: Ui>(
    ui: &mut U,
    accounts: Option<&AccountsFile>,
) -> Result<Option<AccountsFile>, TuiError> {
    let Some(accounts) = accounts.filter(|accounts| !accounts.accounts.is_empty()) else {
        ui.message("No accounts to delete.");
        return Ok(None);
    };

    let mut choices: Vec<String> = accounts
        .accounts
        .iter()
        .map(|account| format!("{} ({})", account.label, account.redacted_email()))
        .collect();
    choices.push("Cancel".to_string());

    let Some(selected) = ui.select("Select account to delete:", &choices)? else {
        return Ok(None);
    };
    if selected >= accounts.accounts.len() {
        return Ok(None);
    }

    let target = &accounts.accounts[selected];
    let Some(confirmed) = ui.confirm(
        &format!(
            "Are you sure you want to permanently delete account [{}]?",
            target.label
        ),
        false,
    )?
    else {
        ui.message("Deletion canceled.");
        return Ok(None);
    };
    if !confirmed {
        ui.message("Deletion canceled.");
        return Ok(None);
    }

    let mut remaining = accounts.accounts.clone();
    remaining.remove(selected);
    let updated = AccountsFile::new(remaining);
    save_accounts(&updated, None)?;
    ui.message(&format!("Account [{}] deleted.", target.label));
    Ok(Some(updated))
}

/// Checks each enabled account with real metadata authentication.
pub fn check_accounts<U: Ui, C: AccountChecker>(
    ui: &mut U,
    accounts: Option<&AccountsFile>,
    checker: &C,
) {
    let Some(accounts) = accounts.filter(|accounts| !accounts.accounts.is_empty()) else {
        ui.message("No accounts configured to check.");
        return;
    };

    ui.message("Checking accounts against Google Play metadata...");
    let mut successes = 0usize;
    let mut failures = 0usize;
    for account in accounts.enabled() {
        match checker.check(account, CHECK_PACKAGE) {
            Ok(version) => {
                successes += 1;
                ui.message(&format!(
                    "  Account [{}]: Authenticated successfully (tested {} v{})",
                    account.label, version.package_name, version.version_name
                ));
            }
            Err(reason) => {
                failures += 1;
                let reason = redact_for_all(&reason, accounts, DIAGNOSTIC_LIMIT);
                ui.message(&format!(
                    "  Failed: [{}] {CHECK_PACKAGE} ({reason})",
                    account.label
                ));
            }
        }
    }
    ui.message(&format!(
        "Verified accounts: {successes} success, {failures} failures."
    ));
}

/// GitHub secret sync configuration. Tests inject a fake `gh`.
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub gh_bin: Option<PathBuf>,
    pub repository: String,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            gh_bin: None,
            repository: DEFAULT_SECRET_REPOSITORY.to_string(),
        }
    }
}

/// Synchronizes the accounts document into the repository secret via stdin.
pub fn sync_github_secret<U: Ui>(
    ui: &mut U,
    accounts: Option<&AccountsFile>,
    options: &SyncOptions,
) -> Result<(), TuiError> {
    let Some(accounts) = accounts.filter(|accounts| !accounts.accounts.is_empty()) else {
        ui.message("No accounts to sync.");
        return Ok(());
    };

    let Some(gh) = options.gh_bin.clone().or_else(resolve_gh_binary) else {
        ui.message("Error: 'gh' CLI is not installed or not in PATH. Local file unchanged.");
        return Ok(());
    };

    let auth = Command::new(&gh)
        .args(["auth", "status"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !auth.status.success() {
        let detail = redact_all(
            &String::from_utf8_lossy(&auth.stderr),
            accounts,
            DIAGNOSTIC_LIMIT,
        );
        ui.message(&format!(
            "Error: 'gh' is not authenticated ({detail}). Local file unchanged."
        ));
        return Ok(());
    }

    let payload = accounts.to_minified_json()?;
    let mut child = Command::new(&gh)
        .args([
            "secret",
            "set",
            SECRET_NAME,
            "--repo",
            options.repository.as_str(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| TuiError::new("Failed to open gh stdin"))?;
    stdin.write_all(payload.as_bytes())?;
    drop(stdin);
    let output = child.wait_with_output()?;

    if !output.status.success() {
        let detail = redact_all(
            &String::from_utf8_lossy(&output.stderr),
            accounts,
            DIAGNOSTIC_LIMIT,
        );
        ui.message(&format!("Failed to set secret: {detail}"));
    } else {
        ui.message(&format!(
            "Successfully synchronized {SECRET_NAME} secret to {}.",
            options.repository
        ));
    }
    Ok(())
}

fn redact_for_all(text: &str, accounts: &AccountsFile, limit: usize) -> String {
    let mut redacted = text.to_string();
    for account in &accounts.accounts {
        redacted = account.redact_diagnostic(&redacted, usize::MAX);
    }
    truncate(&redacted, limit)
}

fn redact_all(stderr: &str, accounts: &AccountsFile, limit: usize) -> String {
    let mut text = stderr.to_string();
    for account in &accounts.accounts {
        text = account.redact_diagnostic(&text, usize::MAX);
    }
    text.trim().chars().take(limit).collect()
}

/// `GH_BIN` first, then `gh` on `PATH`.
#[must_use]
pub fn resolve_gh_binary() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("GH_BIN") {
        let path = PathBuf::from(value);
        if is_executable(&path) {
            return Some(path);
        }
    }
    find_on_path("gh")
}

/// `APKEEP_BIN`, the APK Lab tool cache, then `apkeep` on `PATH`.
#[must_use]
pub fn resolve_apkeep_binary() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("APKEEP_BIN") {
        let path = PathBuf::from(value);
        if is_executable(&path) {
            return Some(path);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let cache = PathBuf::from(home)
            .join(".cache")
            .join("aidans-patches")
            .join("apk-lab")
            .join("tools")
            .join("apkeep");
        if let Some(found) = find_named_recursive(&cache, "apkeep") {
            return Some(found);
        }
    }

    find_on_path("apkeep")
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable(candidate))
}

fn find_named_recursive(root: &Path, name: &str) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_named_recursive(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|value| value.to_str()) == Some(name)
            && is_executable(&path)
        {
            return Some(path);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && path
                .metadata()
                .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Exchanges a one-use OAuth token through apkeep and returns an AAS token.
pub fn exchange_oauth_for_aas<U: Ui>(
    ui: &mut U,
    email: &str,
    oauth_token: &str,
) -> Result<Option<String>, TuiError> {
    let Some(apkeep) = resolve_apkeep_binary() else {
        ui.message("Notice: apkeep executable not found on host/cache.");
        ui.message("You can exchange manually with:");
        ui.message(&format!("  apkeep -e {email} --oauth-token [OAUTH_TOKEN]"));
        let Some(token) = ui.password("Or enter pre-generated AAS token:")? else {
            return Ok(None);
        };
        if token.trim().is_empty() {
            return Err(TuiError::new("AAS token cannot be empty"));
        }
        return Ok(Some(token.trim().to_string()));
    };

    let output = Command::new(&apkeep)
        .args(["-e", email, "--oauth-token", oauth_token])
        .output()?;
    if !output.status.success() {
        let sanitized = output
            .stderr
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>()
            .replace(oauth_token, "[REDACTED_OAUTH_TOKEN]");
        return Err(TuiError::new(format!(
            "Failed to exchange oauth token with apkeep: {}",
            truncate(sanitized.trim(), DIAGNOSTIC_LIMIT)
        )));
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(token) = extract_aas_token(&combined) {
        return Ok(Some(token));
    }

    ui.message("Could not automatically extract AAS token from apkeep output.");
    let Some(token) = ui.password("Please enter the AAS token returned:")? else {
        return Ok(None);
    };
    if token.trim().is_empty() {
        return Err(TuiError::new("AAS token cannot be empty"));
    }
    Ok(Some(token.trim().to_string()))
}

fn extract_aas_token(output: &str) -> Option<String> {
    for line in output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("aas_token") || lower.contains("aas token") {
            if let Some((_, value)) = line.split_once(':') {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        } else if line.trim().starts_with("aas_et/") {
            return Some(line.trim().to_string());
        }
    }
    output
        .split_whitespace()
        .find(|word| word.starts_with("aas_et/"))
        .map(str::to_string)
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_aas_token_from_labelled_output() {
        assert_eq!(
            extract_aas_token("aas_token: aas_et/example\n"),
            Some("aas_et/example".to_string())
        );
        assert_eq!(
            extract_aas_token("noise aas_et/other noise\n"),
            Some("aas_et/other".to_string())
        );
    }

    #[test]
    fn diagnostics_redact_before_truncating() {
        let account = Account {
            label: "test".to_string(),
            email: "user@example.com".to_string(),
            aas_token: "super-secret".to_string(),
            device: DEFAULT_DEVICE.to_string(),
            locale: DEFAULT_LOCALE.to_string(),
            timezone: DEFAULT_TIMEZONE.to_string(),
            enabled: true,
        };
        let text = format!(
            "prefix super-secret and user@example.com {}",
            "x".repeat(400)
        );
        let redacted = account.redact_diagnostic(&text, 50);
        assert!(!redacted.contains("super-secret"));
        assert!(!redacted.contains("user@example.com"));
        assert!(redacted.chars().count() <= 50);
    }
}
