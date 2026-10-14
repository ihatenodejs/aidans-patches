use serde::{Deserialize, Serialize};
use std::io::{self, Read};

#[derive(Debug, Clone, Deserialize)]
pub struct AccountConfig {
    pub label: String,
    pub email: String,
    #[serde(rename = "aasToken")]
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
    "px_9a".to_string()
}
fn default_locale() -> String {
    "en_US".to_string()
}
fn default_timezone() -> String {
    "UTC".to_string()
}
fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountsFile {
    pub version: u32,
    pub accounts: Vec<AccountConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataRequest {
    pub accounts: AccountsFile,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackageVersionResult {
    #[serde(rename = "packageName")]
    pub package_name: String,
    #[serde(rename = "versionName")]
    pub version_name: String,
    #[serde(rename = "versionCode")]
    pub version_code: u64,
    #[serde(rename = "accountLabel")]
    pub account_label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackageFailure {
    #[serde(rename = "packageName")]
    pub package_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MetadataResponse {
    pub results: Vec<PackageVersionResult>,
    pub failures: Vec<PackageFailure>,
}

#[async_trait::async_trait(?Send)]
pub trait PlayDetailsProvider {
    async fn resolve_package(
        &self,
        account: &AccountConfig,
        package_name: &str,
    ) -> Result<(String, u64), String>;
}

pub struct GpapiProvider;

#[async_trait::async_trait(?Send)]
impl PlayDetailsProvider for GpapiProvider {
    async fn resolve_package(
        &self,
        account: &AccountConfig,
        package_name: &str,
    ) -> Result<(String, u64), String> {
        let mut api = gpapi::Gpapi::new(&account.device, &account.email);
        api.set_locale(&account.locale);
        api.set_timezone(&account.timezone);
        api.set_aas_token(&account.aas_token);
        if let Err(e) = api.login().await {
            return Err(format!("Login failed: {e}"));
        }

        match api.details(package_name).await {
            Ok(Some(details)) => {
                let app_details = details
                    .item
                    .and_then(|item| item.details)
                    .and_then(|d| d.app_details);

                if let Some(app) = app_details {
                    let version_name = app.version_string.unwrap_or_default();
                    let version_code = app.version_code.unwrap_or(0);
                    if !version_name.trim().is_empty() && version_code > 0 {
                        return Ok((version_name, version_code as u64));
                    }
                }
                Err("Missing version_string or positive version_code in response".to_string())
            }
            Ok(None) => Err("Empty details response".to_string()),
            Err(e) => Err(format!("Details call failed: {e}")),
        }
    }
}

pub async fn execute_metadata_resolution<P: PlayDetailsProvider>(
    provider: &P,
    req: MetadataRequest,
) -> MetadataResponse {
    let mut results: Vec<PackageVersionResult> = Vec::new();
    let mut failures: Vec<PackageFailure> = Vec::new();

    let enabled_accounts: Vec<&AccountConfig> =
        req.accounts.accounts.iter().filter(|a| a.enabled).collect();

    for package in req.packages {
        let mut resolved = false;
        let mut _last_reason = "No enabled accounts".to_string();

        for account in &enabled_accounts {
            match provider.resolve_package(account, &package).await {
                Ok((version_name, version_code)) => {
                    results.push(PackageVersionResult {
                        package_name: package.clone(),
                        version_name,
                        version_code,
                        account_label: account.label.clone(),
                    });
                    resolved = true;
                    break;
                }
                Err(err) => {
                    _last_reason = err;
                }
            }
        }

        if !resolved {
            failures.push(PackageFailure {
                package_name: package,
                reason: "unavailable".to_string(),
            });
        }
    }

    MetadataResponse { results, failures }
}

#[tokio::main]
async fn main() {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("Failed to read stdin: {e}");
        std::process::exit(1);
    }

    let req: MetadataRequest = match serde_json::from_str(&input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Invalid request JSON: {e}");
            std::process::exit(1);
        }
    };

    let provider = GpapiProvider;
    let resp = execute_metadata_resolution(&provider, req).await;

    match serde_json::to_string(&resp) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("Failed to serialize response: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        responses: std::collections::HashMap<(String, String), Result<(String, u64), String>>,
    }

    #[async_trait::async_trait(?Send)]
    impl PlayDetailsProvider for MockProvider {
        async fn resolve_package(
            &self,
            account: &AccountConfig,
            package_name: &str,
        ) -> Result<(String, u64), String> {
            if let Some(res) = self
                .responses
                .get(&(account.label.clone(), package_name.to_string()))
            {
                res.clone()
            } else {
                Err("account unavailable".to_string())
            }
        }
    }

    #[tokio::test]
    async fn test_first_success_ordering() {
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            ("primary".to_string(), "com.test.pkg".to_string()),
            Ok(("1.0.0".to_string(), 100)),
        );
        responses.insert(
            ("secondary".to_string(), "com.test.pkg".to_string()),
            Ok(("2.0.0".to_string(), 200)),
        );

        let provider = MockProvider { responses };

        let req = MetadataRequest {
            accounts: AccountsFile {
                version: 1,
                accounts: vec![
                    AccountConfig {
                        label: "primary".to_string(),
                        email: "prim@test.com".to_string(),
                        aas_token: "tok1".to_string(),
                        device: "px_9a".to_string(),
                        locale: "en_US".to_string(),
                        timezone: "UTC".to_string(),
                        enabled: true,
                    },
                    AccountConfig {
                        label: "secondary".to_string(),
                        email: "sec@test.com".to_string(),
                        aas_token: "tok2".to_string(),
                        device: "px_9a".to_string(),
                        locale: "en_US".to_string(),
                        timezone: "UTC".to_string(),
                        enabled: true,
                    },
                ],
            },
            packages: vec!["com.test.pkg".to_string()],
        };

        let res = execute_metadata_resolution(&provider, req).await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].account_label, "primary");
        assert_eq!(res.results[0].version_name, "1.0.0");
        assert_eq!(res.results[0].version_code, 100);
        assert!(res.failures.is_empty());
    }

    #[tokio::test]
    async fn test_fallback_to_second_account() {
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            ("primary".to_string(), "com.test.pkg".to_string()),
            Err("package not in catalog".to_string()),
        );
        responses.insert(
            ("secondary".to_string(), "com.test.pkg".to_string()),
            Ok(("2.5.0".to_string(), 250)),
        );

        let provider = MockProvider { responses };

        let req = MetadataRequest {
            accounts: AccountsFile {
                version: 1,
                accounts: vec![
                    AccountConfig {
                        label: "primary".to_string(),
                        email: "prim@test.com".to_string(),
                        aas_token: "tok1".to_string(),
                        device: "px_9a".to_string(),
                        locale: "en_US".to_string(),
                        timezone: "UTC".to_string(),
                        enabled: true,
                    },
                    AccountConfig {
                        label: "secondary".to_string(),
                        email: "sec@test.com".to_string(),
                        aas_token: "tok2".to_string(),
                        device: "px_9a".to_string(),
                        locale: "en_US".to_string(),
                        timezone: "UTC".to_string(),
                        enabled: true,
                    },
                ],
            },
            packages: vec!["com.test.pkg".to_string()],
        };

        let res = execute_metadata_resolution(&provider, req).await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].account_label, "secondary");
        assert_eq!(res.results[0].version_name, "2.5.0");
        assert_eq!(res.results[0].version_code, 250);
        assert!(res.failures.is_empty());
    }

    #[tokio::test]
    async fn test_all_accounts_failure() {
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            ("primary".to_string(), "com.test.pkg".to_string()),
            Err("error 1".to_string()),
        );

        let provider = MockProvider { responses };

        let req = MetadataRequest {
            accounts: AccountsFile {
                version: 1,
                accounts: vec![AccountConfig {
                    label: "primary".to_string(),
                    email: "prim@test.com".to_string(),
                    aas_token: "tok1".to_string(),
                    device: "px_9a".to_string(),
                    locale: "en_US".to_string(),
                    timezone: "UTC".to_string(),
                    enabled: true,
                }],
            },
            packages: vec!["com.test.pkg".to_string()],
        };

        let res = execute_metadata_resolution(&provider, req).await;
        assert!(res.results.is_empty());
        assert_eq!(res.failures.len(), 1);
        assert_eq!(res.failures[0].package_name, "com.test.pkg");
        assert_eq!(res.failures[0].reason, "unavailable");
    }
}
