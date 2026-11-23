//! Native Play delivery provider boundary.
//!
//! The acquisition orchestrator uses these types for direct AAS, OAuth, and
//! dispenser authentication. Tests inject deterministic backends; production
//! uses the `gpapi` implementation. Tokens, email addresses, and dispenser auth
//! material are never logged, printed, or serialized.
//!
//! The provider boundary is deliberately small: [`PlayBackend`] opens one
//! authenticated session per enabled account, and [`PlaySession`] resolves
//! details before downloading. The version check therefore happens before any
//! bytes are requested, and tests can prove that ordering without touching the
//! network. Every failed attempt removes the probe-created directory, so a
//! partial delivery is never advertised as an output.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
use std::env;
use std::fs::{self, File};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_DISPENSER_URL: &str = "https://auroraoss.com/api/auth";
const DISPENSER_USER_AGENT: &str = "com.aurora.store-4.6.1-70";
const DEFAULT_DEVICE: &str = "px_9a";
const DEFAULT_LOCALE: &str = "en_US";
const DEFAULT_TIMEZONE: &str = "UTC";
const REDACTED: &str = "[REDACTED]";
const READ_CHUNK: usize = 64 * 1024;
const REPORT_KIND: &str = "apk-lab-play-delivery-probe";

/// Fixed Android profile sent to the Aurora dispenser, compiled into the
/// probe so runtime does not read another checkout or an account record.
///
/// The values are a pinned Pixel 4a property set; the dispenser rejects
/// abbreviated profiles with HTTP 500.
const DISPENSER_PROFILE: &str = include_str!("aurora-profile.json");

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Aas,
    Oauth,
    Dispenser,
}

impl Mode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "aas" => Ok(Mode::Aas),
            "oauth" => Ok(Mode::Oauth),
            "dispenser" => Ok(Mode::Dispenser),
            other => Err(format!(
                "unknown mode '{other}'; expected aas, oauth, or dispenser"
            )),
        }
    }
}

/// One account from the untouched `apkeep`-compatible accounts document.
///
/// No `Debug` implementation: the type holds an AAS token and an email.
#[derive(Clone, Deserialize)]
pub struct AccountConfig {
    pub label: String,
    pub email: String,
    #[serde(rename = "aasToken", default)]
    pub aas_token: String,
    #[serde(default = "default_device")]
    pub device: String,
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_device() -> String {
    DEFAULT_DEVICE.to_string()
}

fn default_locale() -> String {
    DEFAULT_LOCALE.to_string()
}

fn default_timezone() -> String {
    DEFAULT_TIMEZONE.to_string()
}

fn default_enabled() -> bool {
    true
}

#[derive(Deserialize)]
pub struct AccountsFile {
    pub version: u32,
    pub accounts: Vec<AccountConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version_name: String,
    pub version_code: u64,
}

/// Authentication material supplied to the provider boundary. `oauth_token`
/// is only present in OAuth mode and is never retained in a report.
#[derive(Clone, Copy)]
pub struct AuthContext<'a> {
    pub mode: Mode,
    pub oauth_token: Option<&'a str>,
    pub dispenser_url: &'a str,
}

/// Provider seam. `open` performs authentication and returns one session so
/// that OAuth tokens (which are single-use) are consumed exactly once and the
/// details/version check share the same authenticated connection as download.
pub trait PlayBackend {
    fn open<'a>(
        &'a self,
        account: &'a AccountConfig,
        auth: &'a AuthContext<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn PlaySession + 'a>, String>>;
}

pub trait PlaySession {
    fn details<'a>(&'a mut self, package: &'a str) -> BoxFuture<'a, Result<VersionInfo, String>>;

    fn download<'a>(
        &'a mut self,
        package: &'a str,
        version_code: Option<i64>,
        dest: &'a Path,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// Blocking HTTP boundary for the dispenser, injectable in tests.
pub trait DispenserHttp {
    fn post(&self, url: &str, user_agent: &str, body: &str) -> Result<String, String>;
}

struct UreqDispenserHttp;

impl DispenserHttp for UreqDispenserHttp {
    fn post(&self, url: &str, user_agent: &str, body: &str) -> Result<String, String> {
        let mut response = ureq::post(url)
            .header("user-agent", user_agent)
            .header("content-type", "application/json")
            .send(body)
            .map_err(|error| format!("dispenser HTTP request failed: {error}"))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("dispenser response body could not be read: {error}"))
    }
}

pub struct GpapiBackend {
    http: Box<dyn DispenserHttp>,
}

impl GpapiBackend {
    pub fn new() -> Self {
        Self {
            http: Box::new(UreqDispenserHttp),
        }
    }

    /// Injects a deterministic dispenser transport for tests and other
    /// development harnesses. Production code uses [`GpapiBackend::new`].
    pub fn with_http(http: Box<dyn DispenserHttp>) -> Self {
        Self { http }
    }

    fn dispense(&self, url: &str) -> Result<String, String> {
        dispense_auth_token(self.http.as_ref(), url)
    }
}

impl Default for GpapiBackend {
    fn default() -> Self {
        Self::new()
    }
}

struct GpapiSession {
    api: gpapi::Gpapi,
}

impl PlayBackend for GpapiBackend {
    fn open<'a>(
        &'a self,
        account: &'a AccountConfig,
        auth: &'a AuthContext<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn PlaySession + 'a>, String>> {
        Box::pin(async move {
            let mut api =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_gpapi(account)))
                    .map_err(|_| "provider rejected the configured device profile".to_string())?;

            match auth.mode {
                Mode::Aas => api.set_aas_token(account.aas_token.as_str()),
                Mode::Oauth => {
                    let oauth_token = auth
                        .oauth_token
                        .ok_or_else(|| "OAuth mode requires an OAuth token".to_string())?;
                    api.request_aas_token(oauth_token)
                        .await
                        .map_err(|error| format!("OAuth token exchange failed: {error}"))?;
                }
                Mode::Dispenser => {
                    let auth_token = self.dispense(auth.dispenser_url)?;
                    api.set_auth_token(auth_token);
                }
            }

            if let Err(error) = api.login().await {
                if auth.mode != Mode::Dispenser || !error.to_string().contains("Terms of Service") {
                    return Err(format!("Play login failed: {error}"));
                }
                api.accept_tos()
                    .await
                    .map_err(|error| format!("Play Terms of Service acceptance failed: {error}"))?;
                // gpapi documents accept_tos as making the API ready; a second
                // login would request the same ToS token again.
            }

            Ok(Box::new(GpapiSession { api }) as Box<dyn PlaySession + 'a>)
        })
    }
}

fn build_gpapi(account: &AccountConfig) -> gpapi::Gpapi {
    let mut api = gpapi::Gpapi::new(account.device.as_str(), account.email.as_str());
    api.set_locale(account.locale.as_str());
    api.set_timezone(account.timezone.as_str());
    api
}

impl PlaySession for GpapiSession {
    fn details<'a>(&'a mut self, package: &'a str) -> BoxFuture<'a, Result<VersionInfo, String>> {
        Box::pin(async move {
            let response = self
                .api
                .details(package)
                .await
                .map_err(|error| format!("Play details request failed: {error}"))?
                .ok_or_else(|| "Play details response was empty".to_string())?;

            let app = response
                .item
                .and_then(|item| item.details)
                .and_then(|details| details.app_details)
                .ok_or_else(|| "Play details response did not contain app details".to_string())?;

            let version_name = app.version_string.unwrap_or_default();
            let version_code = app.version_code.unwrap_or(0);
            if version_code <= 0 {
                return Err("Play details response did not contain a positive version code".into());
            }

            Ok(VersionInfo {
                version_name,
                version_code: version_code as u64,
            })
        })
    }

    fn download<'a>(
        &'a mut self,
        package: &'a str,
        version_code: Option<i64>,
        dest: &'a Path,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.api
                .download(package, version_code, true, false, false, dest, None)
                .await
                .map(|_| ())
                .map_err(|error| format!("Play download failed: {error}"))
        })
    }
}

fn dispense_auth_token(http: &dyn DispenserHttp, url: &str) -> Result<String, String> {
    let body = http.post(url, DISPENSER_USER_AGENT, DISPENSER_PROFILE)?;
    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("dispenser response was not valid JSON: {error}"))?;
    parsed
        .get("authToken")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "dispenser response did not contain a non-empty authToken".to_string())
}

#[derive(Default)]
struct Options {
    package: String,
    accounts_file: Option<PathBuf>,
    output: PathBuf,
    mode: Mode,
    version_code: Option<i64>,
    account_label: Option<String>,
    oauth_token_file: Option<PathBuf>,
    dispenser_url: String,
    report: Option<PathBuf>,
    check: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let Some(options) = parse_args(args)? else {
        print_usage();
        return Ok(());
    };

    let accounts = load_accounts(options.accounts_file.as_deref())?;
    validate_accounts(&accounts, options.mode)?;

    let oauth_token = match options.mode {
        Mode::Oauth => {
            let path = options
                .oauth_token_file
                .as_deref()
                .ok_or_else(|| "--oauth-token-file is required in oauth mode".to_string())?;
            Some(read_private_token_file(path)?)
        }
        Mode::Aas | Mode::Dispenser => {
            if options.oauth_token_file.is_some() {
                return Err("--oauth-token-file is only valid in oauth mode".to_string());
            }
            None
        }
    };

    let backend = GpapiBackend::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not create the Play runtime: {error}"))?;
    let report = runtime.block_on(execute(
        &options,
        &accounts,
        oauth_token.as_deref(),
        &backend,
    ))?;

    let sanitizer = Sanitizer::new(&accounts, oauth_token.as_deref());
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialize the probe report: {error}"))?;
    let json = sanitizer.sanitize(&json);

    if let Some(path) = options.report.as_deref() {
        if let Err(error) = fs::write(path, format!("{json}\n")) {
            // The delivery succeeded, but the command cannot report it.
            // Remove only the probe directory before returning failure.
            let _ = fs::remove_dir_all(&report.download_dir);
            return Err(format!(
                "could not write report '{}': {error}",
                path.display()
            ));
        }
    }
    println!("{json}");
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    if args
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        return Ok(None);
    }

    let mut options = Options {
        dispenser_url: DEFAULT_DISPENSER_URL.to_string(),
        ..Options::default()
    };
    let mut package = None;
    let mut output = None;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (argument.as_str(), None),
        };

        match flag {
            "--package" => package = Some(take_value(args, &mut index, inline, flag)?),
            "--accounts-file" => {
                options.accounts_file =
                    Some(PathBuf::from(take_value(args, &mut index, inline, flag)?))
            }
            "--output" => output = Some(PathBuf::from(take_value(args, &mut index, inline, flag)?)),
            "--mode" => options.mode = Mode::parse(&take_value(args, &mut index, inline, flag)?)?,
            "--version-code" => {
                let raw = take_value(args, &mut index, inline, flag)?;
                let code: i64 = raw
                    .parse()
                    .map_err(|_| format!("invalid --version-code '{raw}'; expected an integer"))?;
                if code <= 0 {
                    return Err("--version-code must be a positive integer".to_string());
                }
                options.version_code = Some(code);
            }
            "--account-label" => {
                options.account_label = Some(take_value(args, &mut index, inline, flag)?)
            }
            "--oauth-token-file" => {
                options.oauth_token_file =
                    Some(PathBuf::from(take_value(args, &mut index, inline, flag)?))
            }
            "--dispenser-url" => {
                options.dispenser_url = take_value(args, &mut index, inline, flag)?
            }
            "--report" => {
                options.report = Some(PathBuf::from(take_value(args, &mut index, inline, flag)?))
            }
            "--check" => {
                if inline.is_some() {
                    return Err("--check does not take a value".to_string());
                }
                options.check = true;
            }
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => return Err(format!("unexpected argument '{other}'")),
        }

        index += 1;
    }

    let package = package.ok_or_else(|| "--package is required".to_string())?;
    if package.trim().is_empty() {
        return Err("--package must not be empty".to_string());
    }
    let output = output.ok_or_else(|| "--output is required".to_string())?;
    options.package = package;
    options.output = output;
    Ok(Some(options))
}

fn take_value(
    args: &[String],
    index: &mut usize,
    inline: Option<&str>,
    flag: &str,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value.to_string());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn load_accounts(explicit: Option<&Path>) -> Result<Vec<AccountConfig>, String> {
    if let Some(path) = explicit {
        return parse_accounts_file(path);
    }
    if let Some(path) = env::var_os("APKEEP_ACCOUNTS_FILE") {
        return parse_accounts_file(Path::new(&path));
    }
    if let Ok(raw) = env::var("APKEEP_ACCOUNTS") {
        if !raw.trim().is_empty() {
            return parse_accounts_json(&raw, "APKEEP_ACCOUNTS");
        }
    }
    let home = env::var_os("HOME").ok_or_else(|| {
        "HOME is not set; pass --accounts-file or APKEEP_ACCOUNTS_FILE".to_string()
    })?;
    parse_accounts_file(
        &PathBuf::from(home).join(".config/aidans-patches/google-play-accounts.json"),
    )
}

fn parse_accounts_file(path: &Path) -> Result<Vec<AccountConfig>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("could not read accounts file '{}': {error}", path.display()))?;
    parse_accounts_json(&raw, &path.display().to_string())
}

fn parse_accounts_json(raw: &str, source: &str) -> Result<Vec<AccountConfig>, String> {
    let document: AccountsFile = serde_json::from_str(raw)
        .map_err(|error| format!("accounts document from {source} is invalid: {error}"))?;
    if document.version != 1 {
        return Err(format!(
            "accounts document from {source} has unsupported version {}; expected 1",
            document.version
        ));
    }
    Ok(document.accounts)
}

fn validate_accounts(accounts: &[AccountConfig], mode: Mode) -> Result<(), String> {
    if accounts.is_empty() {
        return Err("accounts document contains no accounts".to_string());
    }

    let mut labels = BTreeSet::new();
    let mut emails = BTreeSet::new();
    for account in accounts {
        let label = account.label.trim();
        if label.is_empty() {
            return Err("account label must not be empty".to_string());
        }
        let email = account.email.trim();
        if email.is_empty() {
            return Err(format!("account '{label}' must have a non-empty email"));
        }
        if !labels.insert(label.to_string()) {
            return Err(format!("duplicate account label '{label}'"));
        }
        if !emails.insert(email.to_ascii_lowercase()) {
            return Err(format!(
                "account '{label}' duplicates another email address"
            ));
        }
        if account.device.trim().is_empty() {
            return Err(format!("account '{label}' must have a non-empty device"));
        }
        if account.locale.trim().is_empty() {
            return Err(format!("account '{label}' must have a non-empty locale"));
        }
        if account.timezone.trim().is_empty() {
            return Err(format!("account '{label}' must have a non-empty timezone"));
        }
        if mode == Mode::Aas && account.aas_token.trim().is_empty() {
            return Err(format!("account '{label}' must have a non-empty aasToken"));
        }
    }

    Ok(())
}

fn read_private_token_file(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "could not inspect OAuth token file '{}': {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "OAuth token file '{}' must be a regular file; symlinks and special files are rejected",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "OAuth token file '{}' has permissions {mode:04o}; group/world access must be removed (0600)",
                path.display()
            ));
        }
    }

    let mut contents = String::new();
    File::open(path)
        .and_then(|mut file| file.read_to_string(&mut contents))
        .map_err(|error| {
            format!(
                "could not read OAuth token file '{}': {error}",
                path.display()
            )
        })?;

    let token = contents.trim().to_string();
    if token.is_empty() {
        return Err("OAuth token file must contain a non-empty token".to_string());
    }
    Ok(token)
}

async fn execute(
    options: &Options,
    accounts: &[AccountConfig],
    oauth_token: Option<&str>,
    backend: &dyn PlayBackend,
) -> Result<ProbeReport, String> {
    let sanitizer = Sanitizer::new(accounts, oauth_token);

    let candidates: Vec<&AccountConfig> = accounts
        .iter()
        .filter(|account| account.enabled)
        .filter(|account| {
            options
                .account_label
                .as_deref()
                .is_none_or(|label| account.label == label)
        })
        .collect();

    if candidates.is_empty() {
        let detail = match options.account_label.as_deref() {
            Some(label) => format!("no enabled account is labeled '{label}'"),
            None => "accounts document contains no enabled accounts".to_string(),
        };
        return Err(sanitizer.sanitize(&detail));
    }

    let mut last_error = "no account was attempted".to_string();
    for account in candidates {
        let directory = match create_probe_dir(&options.output, &options.package) {
            Ok(directory) => directory,
            Err(error) => return Err(sanitizer.sanitize(&error)),
        };

        match attempt(backend, account, options, oauth_token, &directory).await {
            Ok(report) => return Ok(report),
            Err(error) => {
                // Only the directory this probe created is removed. The
                // caller-supplied output root is never touched.
                let _ = fs::remove_dir_all(&directory);
                last_error = sanitizer.sanitize(&error);
            }
        }
    }

    Err(last_error)
}

async fn attempt(
    backend: &dyn PlayBackend,
    account: &AccountConfig,
    options: &Options,
    oauth_token: Option<&str>,
    directory: &Path,
) -> Result<ProbeReport, String> {
    let auth = AuthContext {
        mode: options.mode,
        oauth_token,
        dispenser_url: &options.dispenser_url,
    };
    let mut session = backend.open(account, &auth).await?;

    let observed = session.details(&options.package).await?;
    if let Some(requested) = options.version_code {
        if observed.version_code != requested as u64 {
            return Err(format!(
                "requested version code {requested} but provider details report {}",
                observed.version_code
            ));
        }
    }

    session
        .download(&options.package, options.version_code, directory)
        .await?;

    let files = collect_files(directory)?;
    let base_name = format!("{}.apk", options.package);
    let has_base = files
        .iter()
        .any(|file| file.path.rsplit('/').next() == Some(base_name.as_str()));
    let check = CheckOutcome {
        ok: has_base && !files.is_empty(),
        base_apk: has_base,
        file_count: files.len(),
    };

    if !check.ok {
        let reason = if files.is_empty() {
            "provider delivered no files"
        } else {
            "provider delivered no base APK"
        };
        return Err(if options.check {
            format!("check failed: incomplete delivery: {reason}")
        } else {
            format!("incomplete delivery: {reason}")
        });
    }

    Ok(ProbeReport {
        kind: REPORT_KIND,
        schema_version: 1,
        package: options.package.clone(),
        account_label: account.label.clone(),
        mode: options.mode,
        requested_version_code: options.version_code,
        observed,
        host: HostInfo {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
        },
        download_dir: directory.to_string_lossy().into_owned(),
        files,
        check,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    kind: &'static str,
    schema_version: u32,
    package: String,
    account_label: String,
    mode: Mode,
    requested_version_code: Option<i64>,
    observed: VersionInfo,
    host: HostInfo,
    download_dir: String,
    files: Vec<FileEntry>,
    check: CheckOutcome,
}

#[derive(Debug, Serialize)]
struct HostInfo {
    os: String,
    arch: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckOutcome {
    ok: bool,
    base_apk: bool,
    file_count: usize,
}

fn create_probe_dir(output: &Path, package: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(output).map_err(|error| {
        format!(
            "could not create output directory '{}': {error}",
            output.display()
        )
    })?;
    if !output.is_dir() {
        return Err(format!(
            "output path '{}' is not a directory",
            output.display()
        ));
    }

    let safe_package: String = package
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    for attempt in 0..100u32 {
        let directory = output.join(format!(
            "play-probe-{safe_package}-{}-{stamp}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "could not create probe directory '{}': {error}",
                    directory.display()
                ))
            }
        }
    }

    Err("could not create a unique probe directory".to_string())
}

fn collect_files(root: &Path) -> Result<Vec<FileEntry>, String> {
    let mut found = Vec::new();
    walk_regular_files(root, root, &mut found)?;
    found.sort_by(|left, right| left.0.cmp(&right.0));

    let mut files = Vec::with_capacity(found.len());
    for (relative, absolute) in found {
        let metadata = fs::symlink_metadata(&absolute).map_err(|error| {
            format!("could not inspect downloaded file '{}': {error}", relative)
        })?;
        if !metadata.file_type().is_file() {
            continue;
        }
        files.push(FileEntry {
            sha256: sha256_file(&absolute)?,
            size: metadata.len(),
            path: relative,
        });
    }
    Ok(files)
}

fn walk_regular_files(
    root: &Path,
    directory: &Path,
    found: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "could not read download directory '{}': {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect directory entry: {error}"))?;
        let path = entry.path();
        if file_type.is_dir() {
            walk_regular_files(root, &path, found)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "downloaded file escaped the probe directory".to_string())?;
            let mut components = VecDeque::new();
            for component in relative.components() {
                components.push_back(component.as_os_str().to_string_lossy().into_owned());
            }
            found.push((components.into_iter().collect::<Vec<_>>().join("/"), path));
        }
        // Symlinks and other special files are intentionally ignored; they
        // are never followed or hashed.
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("could not open '{}': {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not read '{}': {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Replaces every known credential or email with a fixed marker. Secrets are
/// deduplicated and empty values are ignored.
struct Sanitizer {
    secrets: Vec<String>,
}

impl Sanitizer {
    fn new(accounts: &[AccountConfig], oauth_token: Option<&str>) -> Self {
        let mut secrets = BTreeSet::new();
        for account in accounts {
            if !account.aas_token.trim().is_empty() {
                secrets.insert(account.aas_token.clone());
            }
            if !account.email.trim().is_empty() {
                secrets.insert(account.email.clone());
            }
        }
        if let Some(token) = oauth_token {
            if !token.trim().is_empty() {
                secrets.insert(token.to_string());
            }
        }
        let mut secrets: Vec<String> = secrets.into_iter().collect();
        // Longest first so a token that contains another known value is
        // redacted as a whole.
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self { secrets }
    }

    fn sanitize(&self, value: &str) -> String {
        let mut sanitized = value.to_string();
        for secret in &self.secrets {
            sanitized = sanitized.replace(secret, REDACTED);
        }
        sanitized
    }
}

fn print_usage() {
    println!(
        "native Play delivery probe --package PKG --output DIR [options]\n\
         \n\
         Development-only native Play delivery probe. It downloads into a fresh\n\
         directory under --output and reports base/split files plus observed\n\
         details. No Python or other interpreter is invoked.\n\
         \n\
         Options:\n\
         \x20 --package PKG             Android package name (required)\n\
         \x20 --output DIR              Output root; a fresh probe directory is created (required)\n\
         \x20 --accounts-file PATH      apkeep-compatible accounts JSON; defaults to\n\
         \x20                           APKEEP_ACCOUNTS_FILE or\n\
         \x20                           ~/.config/aidans-patches/google-play-accounts.json\n\
         \x20 --mode aas|oauth|dispenser Authentication mode (default: aas)\n\
         \x20 --version-code N          Exact version code; latest when omitted\n\
         \x20 --account-label L         Try only this enabled account\n\
         \x20 --oauth-token-file PATH   One-use OAuth token (0600 regular file; oauth mode only)\n\
         \x20 --dispenser-url URL       Dispenser endpoint (default: {DEFAULT_DISPENSER_URL})\n\
         \x20 --report PATH             Write the sanitized JSON report to PATH\n\
         \x20 --check                   Fail unless a base APK and at least one file are delivered"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    fn account(label: &str, token: &str) -> AccountConfig {
        AccountConfig {
            label: label.to_string(),
            email: format!("{label}@example.invalid"),
            aas_token: token.to_string(),
            device: DEFAULT_DEVICE.to_string(),
            locale: DEFAULT_LOCALE.to_string(),
            timezone: DEFAULT_TIMEZONE.to_string(),
            enabled: true,
        }
    }

    fn options(output: &Path) -> Options {
        Options {
            package: "com.example.app".to_string(),
            accounts_file: None,
            output: output.to_path_buf(),
            mode: Mode::Aas,
            version_code: None,
            account_label: None,
            oauth_token_file: None,
            dispenser_url: DEFAULT_DISPENSER_URL.to_string(),
            report: None,
            check: false,
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    type FakeFiles = Result<Vec<(String, Vec<u8>)>, String>;

    #[derive(Default)]
    struct MockState {
        details: BTreeMap<String, Result<VersionInfo, String>>,
        downloads: BTreeMap<String, FakeFiles>,
        open_calls: Vec<String>,
        download_calls: Vec<String>,
    }

    struct MockBackend {
        state: Rc<RefCell<MockState>>,
    }

    impl MockBackend {
        fn new(state: MockState) -> Self {
            Self {
                state: Rc::new(RefCell::new(state)),
            }
        }
    }

    struct MockSession {
        label: String,
        state: Rc<RefCell<MockState>>,
    }

    impl PlayBackend for MockBackend {
        fn open<'a>(
            &'a self,
            account: &'a AccountConfig,
            _auth: &'a AuthContext<'a>,
        ) -> BoxFuture<'a, Result<Box<dyn PlaySession + 'a>, String>> {
            let state = Rc::clone(&self.state);
            let label = account.label.clone();
            Box::pin(async move {
                state.borrow_mut().open_calls.push(label.clone());
                Ok(Box::new(MockSession { label, state }) as Box<dyn PlaySession + 'a>)
            })
        }
    }

    impl PlaySession for MockSession {
        fn details<'a>(
            &'a mut self,
            _package: &'a str,
        ) -> BoxFuture<'a, Result<VersionInfo, String>> {
            let state = Rc::clone(&self.state);
            let label = self.label.clone();
            Box::pin(async move {
                state
                    .borrow()
                    .details
                    .get(&label)
                    .cloned()
                    .unwrap_or_else(|| Err("no fake details configured".to_string()))
            })
        }

        fn download<'a>(
            &'a mut self,
            _package: &'a str,
            _version_code: Option<i64>,
            dest: &'a Path,
        ) -> BoxFuture<'a, Result<(), String>> {
            let state = Rc::clone(&self.state);
            let label = self.label.clone();
            Box::pin(async move {
                state.borrow_mut().download_calls.push(label.clone());
                match state
                    .borrow()
                    .downloads
                    .get(&label)
                    .cloned()
                    .unwrap_or_else(|| Err("no fake download configured".to_string()))
                {
                    Ok(files) => {
                        for (name, bytes) in files {
                            fs::write(dest.join(name), bytes)
                                .map_err(|error| format!("fake download write failed: {error}"))?;
                        }
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            })
        }
    }

    fn version(code: u64) -> VersionInfo {
        VersionInfo {
            version_name: format!("1.0.{code}"),
            version_code: code,
        }
    }

    fn complete_download(package: &str) -> FakeFiles {
        Ok(vec![
            (format!("{package}.apk"), b"base-bytes".to_vec()),
            (
                format!("{package}.config.arm64_v8a.apk"),
                b"split-bytes".to_vec(),
            ),
        ])
    }

    #[test]
    fn delivers_base_and_split_with_hashes() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let options = options(&output);
        let accounts = vec![account("primary", "aas_et/primary-secret")];

        let mut state = MockState::default();
        state.details.insert("primary".into(), Ok(version(42)));
        state
            .downloads
            .insert("primary".into(), complete_download(&options.package));
        let backend = MockBackend::new(state);

        let report = block_on(execute(&options, &accounts, None, &backend)).unwrap();

        assert_eq!(report.package, options.package);
        assert_eq!(report.account_label, "primary");
        assert_eq!(report.mode, Mode::Aas);
        assert_eq!(report.observed.version_code, 42);
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.files[0].path, "com.example.app.apk");
        assert_eq!(report.files[1].path, "com.example.app.config.arm64_v8a.apk");
        assert_eq!(report.files[0].size, 10);
        assert_eq!(
            report.files[0].sha256,
            format!("{:x}", Sha256::digest(b"base-bytes"))
        );
        assert!(report.check.ok);
        assert!(report.check.base_apk);
        assert!(Path::new(&report.download_dir).is_dir());
        assert_eq!(report.host.os, env::consts::OS);
    }

    #[test]
    fn empty_delivery_fails_and_removes_probe_directory() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let options = options(&output);
        let accounts = vec![account("primary", "aas_et/primary-secret")];

        let mut state = MockState::default();
        state.details.insert("primary".into(), Ok(version(42)));
        state.downloads.insert("primary".into(), Ok(Vec::new()));
        let backend = MockBackend::new(state);

        let error = block_on(execute(&options, &accounts, None, &backend)).unwrap_err();
        assert!(error.contains("no files"), "unexpected error: {error}");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }

    #[test]
    fn partial_delivery_without_base_apk_fails() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let options = options(&output);
        let accounts = vec![account("primary", "aas_et/primary-secret")];

        let mut state = MockState::default();
        state.details.insert("primary".into(), Ok(version(42)));
        state.downloads.insert(
            "primary".into(),
            Ok(vec![(
                format!("{}.config.arm64_v8a.apk", options.package),
                b"split".to_vec(),
            )]),
        );
        let backend = MockBackend::new(state);

        let error = block_on(execute(&options, &accounts, None, &backend)).unwrap_err();
        assert!(error.contains("no base APK"), "unexpected error: {error}");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }

    #[test]
    fn wrong_version_fails_before_download() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let mut options = options(&output);
        options.version_code = Some(41);
        let accounts = vec![account("primary", "aas_et/primary-secret")];

        let mut state = MockState::default();
        state.details.insert("primary".into(), Ok(version(42)));
        state
            .downloads
            .insert("primary".into(), complete_download(&options.package));
        let backend = MockBackend::new(state);

        let error = block_on(execute(&options, &accounts, None, &backend)).unwrap_err();
        assert!(
            error.contains("requested version code 41"),
            "unexpected error: {error}"
        );
        assert!(backend.state.borrow().download_calls.is_empty());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }

    #[test]
    fn rotates_to_next_account_after_provider_failure() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let options = options(&output);
        let accounts = vec![
            account("primary", "aas_et/primary-secret"),
            account("secondary", "aas_et/secondary-secret"),
        ];

        let mut state = MockState::default();
        state
            .details
            .insert("primary".into(), Err("provider rejected account".into()));
        state.details.insert("secondary".into(), Ok(version(42)));
        state
            .downloads
            .insert("secondary".into(), complete_download(&options.package));
        let backend = MockBackend::new(state);

        let report = block_on(execute(&options, &accounts, None, &backend)).unwrap();
        assert_eq!(report.account_label, "secondary");
        assert_eq!(
            backend.state.borrow().open_calls,
            vec!["primary".to_string(), "secondary".to_string()]
        );
    }

    #[test]
    fn account_label_restricts_attempts() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let mut options = options(&output);
        options.account_label = Some("secondary".to_string());
        let accounts = vec![
            account("primary", "aas_et/primary-secret"),
            account("secondary", "aas_et/secondary-secret"),
        ];

        let mut state = MockState::default();
        state.details.insert("primary".into(), Ok(version(41)));
        state.details.insert("secondary".into(), Ok(version(42)));
        state
            .downloads
            .insert("secondary".into(), complete_download(&options.package));
        let backend = MockBackend::new(state);

        let report = block_on(execute(&options, &accounts, None, &backend)).unwrap();
        assert_eq!(report.account_label, "secondary");
        assert_eq!(
            backend.state.borrow().open_calls,
            vec!["secondary".to_string()]
        );
    }

    #[test]
    fn account_tokens_and_oauth_tokens_are_redacted_from_errors() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("out");
        let mut options = options(&output);
        options.mode = Mode::Oauth;
        let aas_secret = "aas_et/account-token-value";
        let oauth_secret = "oauth-one-use-token-value";
        let accounts = vec![account("primary", aas_secret)];

        let mut state = MockState::default();
        state.details.insert(
            "primary".into(),
            Err(format!("failure for {aas_secret} and {oauth_secret}")),
        );
        let backend = MockBackend::new(state);

        let error =
            block_on(execute(&options, &accounts, Some(oauth_secret), &backend)).unwrap_err();
        assert!(!error.contains(aas_secret), "leaked AAS token: {error}");
        assert!(!error.contains(oauth_secret), "leaked OAuth token: {error}");
        assert!(error.contains(REDACTED));
    }

    #[test]
    fn sanitizer_removes_emails_and_longest_tokens_first() {
        let accounts = vec![account("primary", "abcdef")];
        let sanitizer = Sanitizer::new(&accounts, Some("abcdef-longer"));
        let sanitized =
            sanitizer.sanitize("abcdef-longer then abcdef then primary@example.invalid");
        assert_eq!(
            sanitized,
            format!("{REDACTED} then {REDACTED} then {REDACTED}")
        );
    }

    #[test]
    fn duplicate_accounts_are_rejected() {
        let accounts = vec![
            account("primary", "aas_et/one"),
            account("primary", "aas_et/two"),
        ];
        let error = validate_accounts(&accounts, Mode::Aas).unwrap_err();
        assert!(error.contains("duplicate account label"));
    }

    #[test]
    fn empty_aas_token_is_rejected_in_aas_mode() {
        let accounts = vec![account("primary", "")];
        let error = validate_accounts(&accounts, Mode::Aas).unwrap_err();
        assert!(error.contains("aasToken"));
        assert!(validate_accounts(&accounts, Mode::Oauth).is_ok());
    }

    #[test]
    fn dispenser_profile_is_valid_json() {
        let parsed: serde_json::Value = serde_json::from_str(DISPENSER_PROFILE).unwrap();
        assert!(parsed.is_object());
    }

    struct FakeHttp {
        result: Result<String, String>,
    }

    impl DispenserHttp for FakeHttp {
        fn post(&self, _url: &str, _user_agent: &str, _body: &str) -> Result<String, String> {
            self.result.clone()
        }
    }

    #[test]
    fn dispenser_returns_auth_token() {
        let http = FakeHttp {
            result: Ok(r#"{"authToken":"ya29.test-token"}"#.to_string()),
        };
        assert_eq!(
            dispense_auth_token(&http, DEFAULT_DISPENSER_URL).unwrap(),
            "ya29.test-token"
        );
    }

    #[test]
    fn malformed_dispenser_json_is_rejected() {
        let http = FakeHttp {
            result: Ok("not json".to_string()),
        };
        let error = dispense_auth_token(&http, DEFAULT_DISPENSER_URL).unwrap_err();
        assert!(error.contains("valid JSON"), "unexpected error: {error}");
    }

    #[test]
    fn dispenser_without_auth_token_is_rejected() {
        let http = FakeHttp {
            result: Ok(r#"{"token":"ignored"}"#.to_string()),
        };
        let error = dispense_auth_token(&http, DEFAULT_DISPENSER_URL).unwrap_err();
        assert!(error.contains("authToken"), "unexpected error: {error}");
    }

    #[test]
    fn dispenser_http_failure_is_rejected() {
        let http = FakeHttp {
            result: Err("connection refused".to_string()),
        };
        let error = dispense_auth_token(&http, DEFAULT_DISPENSER_URL).unwrap_err();
        assert!(error.contains("connection refused"));
    }

    #[cfg(unix)]
    #[test]
    fn oauth_token_file_requires_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("oauth.token");
        fs::write(&path, "one-use-token\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_token_file(&path).is_err());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_private_token_file(&path).unwrap(), "one-use-token");
    }

    #[cfg(unix)]
    #[test]
    fn oauth_token_file_rejects_symlinks() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.token");
        fs::write(&target, "one-use-token").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let link = temp.path().join("link.token");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(read_private_token_file(&link).is_err());
    }
}
