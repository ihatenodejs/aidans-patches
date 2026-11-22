//! Per-sample staging for asset-pipeline workloads.
//!
//! Asset workloads operate on large, private, hash-pinned originals (the raw
//! emoji font, the processed font, the OCR model, a PDFBox AAR and Play source
//! images). The benchmark must never point a timed process at the user's cache
//! and must never mutate it. A workload can therefore declare `stagedFiles`
//! entries that copy exactly one declared, hash-verified input role into its
//! own per-sample workspace *before* the process is spawned.
//!
//! Rules enforced here:
//!
//! * the role must name a declared `inputs` entry, so only verified inputs are
//!   ever staged;
//! * the destination is a normalized relative path under the sample workspace,
//!   with no root, prefix, `..`, `.`, empty or placeholder components;
//! * two entries cannot target the same destination;
//! * copies use `fs::copy` (never a hard link), so the source is independent;
//! * after the process exits the declared source is re-hashed against its pin
//!   and every staged copy is re-hashed against what was copied. Any mutation
//!   or deletion is a hard sample error rather than a silently accepted run.
//!
//! The same staging runs for every warmup, measured sample and `--prepare`
//! execution, because it happens inside `run_one`. Workloads without
//! `stagedFiles` take the untouched pre-existing path.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{resolve_against, verify_file_hash, InputRef};

/// One declared input role copied into the sample workspace before timing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StagedFile {
    /// Role of an entry in the workload's `inputs` array.
    pub input_role: String,
    /// Relative destination below the per-sample workspace.
    pub path: String,
}

/// A staged copy tracked for the post-run mutation check.
#[derive(Debug, Clone)]
pub(crate) struct StagedInput {
    source: PathBuf,
    destination: PathBuf,
    role: String,
    relative_path: String,
    declared_sha256: String,
    staged_sha256: String,
}

/// Validate the manifest-level shape of `stagedFiles`.
///
/// This runs during `validate_manifest`, before any process can start, so a
/// malformed destination can never be created even by `--prepare`.
pub(crate) fn validate_staged_files(
    workload_id: &str,
    inputs: &[InputRef],
    staged: &[StagedFile],
) -> Result<(), String> {
    let mut destinations = std::collections::BTreeSet::new();
    for entry in staged {
        if entry.input_role.trim().is_empty() {
            return Err(format!(
                "workload '{workload_id}' stagedFiles contains an empty inputRole"
            ));
        }
        if !entry
            .input_role
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err(format!(
                "workload '{workload_id}' stagedFiles inputRole '{}' may only contain ASCII letters, digits, '.', '-', '_'",
                entry.input_role
            ));
        }
        if !inputs.iter().any(|input| input.role == entry.input_role) {
            return Err(format!(
                "workload '{workload_id}' stagedFiles inputRole '{}' is not a declared input role",
                entry.input_role
            ));
        }
        let relative = validated_relative_destination(workload_id, &entry.path)?;
        let normalized = normalized_key(&relative);
        if !destinations.insert(normalized.clone()) {
            return Err(format!(
                "workload '{workload_id}' stagedFiles declares duplicate destination '{normalized}'"
            ));
        }
    }
    Ok(())
}

/// Copy every declared staged input into `workspace`.
///
/// The source is verified against its declared SHA-256 immediately before the
/// copy, and the copy is verified again afterwards. Any failure returns `Err`
/// so `run_one` aborts before the timed process exists.
pub(crate) fn stage_declared_inputs(
    repo_root: &Path,
    inputs: &[InputRef],
    staged: &[StagedFile],
    workspace: &Path,
) -> Result<Vec<StagedInput>, String> {
    let mut result = Vec::with_capacity(staged.len());
    for entry in staged {
        let input = inputs
            .iter()
            .find(|input| input.role == entry.input_role)
            .ok_or_else(|| {
                format!(
                    "staged input role '{}' is not a declared workload input",
                    entry.input_role
                )
            })?;
        let relative = validated_relative_destination("staged input", &entry.path)?;
        let source = resolve_against(repo_root, Path::new(&input.path));
        let verified = verify_file_hash(&source, &input.sha256)
            .map_err(|error| format!("staged input '{}': {error}", entry.input_role))?;

        let destination = workspace.join(&relative);
        if !destination.starts_with(workspace) || destination == workspace {
            return Err(format!(
                "staged input '{}' destination '{}' escapes the sample workspace",
                entry.input_role, entry.path
            ));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "cannot create staged input directory for '{}': {error}",
                    entry.path
                )
            })?;
        }
        fs::copy(&source, &destination).map_err(|error| {
            format!(
                "cannot stage input role '{}' to '{}': {error}",
                entry.input_role, entry.path
            )
        })?;
        let copied = verify_file_hash(&destination, &input.sha256).map_err(|error| {
            format!(
                "staged copy for input role '{}' at '{}' failed verification: {error}",
                entry.input_role, entry.path
            )
        })?;
        if copied.size_bytes != verified.size_bytes {
            return Err(format!(
                "staged copy for input role '{}' at '{}' has a different size",
                entry.input_role, entry.path
            ));
        }
        result.push(StagedInput {
            source,
            destination,
            role: entry.input_role.clone(),
            relative_path: normalized_key(&relative),
            declared_sha256: input.sha256.clone(),
            staged_sha256: copied.sha256,
        });
    }
    Ok(result)
}

/// Re-hash sources and staged copies after the measured process exits.
///
/// Errors are returned, never panics: a workload that modifies the user cache
/// or its own staged input invalidates the sample.
pub(crate) fn verify_staged_inputs(staged: &[StagedInput]) -> Vec<String> {
    let mut errors = Vec::new();
    for entry in staged {
        match verify_file_hash(&entry.source, &entry.declared_sha256) {
            Ok(_) => {}
            Err(error) => errors.push(format!(
                "staged input '{}' source was modified or removed during the run: {error}",
                entry.role
            )),
        }
        match verify_file_hash(&entry.destination, &entry.staged_sha256) {
            Ok(_) => {}
            Err(error) => errors.push(format!(
                "staged input '{}' at '{}' was modified or removed during the run: {error}",
                entry.role, entry.relative_path
            )),
        }
    }
    errors
}

fn validated_relative_destination(workload_id: &str, value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        return Err(format!(
            "workload '{workload_id}' stagedFiles contains an empty path"
        ));
    }
    if value.contains('\0') {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path contains a NUL byte"
        ));
    }
    if value.contains('{') || value.contains('}') {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' must be a fixed relative path, not a placeholder",
            value
        ));
    }
    if value.ends_with('/') || value.ends_with('\\') {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' must name a file, not a directory",
            value
        ));
    }
    if value.split(['/', '\\']).any(str::is_empty) {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' contains an empty component",
            value
        ));
    }
    if value
        .split(['/', '\\'])
        .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' contains a disallowed component",
            value
        ));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' must be relative to the sample workspace",
            value
        ));
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part.is_empty() {
                    return Err(format!(
                        "workload '{workload_id}' stagedFiles path '{}' contains an empty component",
                        value
                    ));
                }
                relative.push(part);
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(format!(
                "workload '{workload_id}' stagedFiles path '{}' contains a disallowed component",
                value
            ))
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(format!(
            "workload '{workload_id}' stagedFiles path '{}' has no file component",
            value
        ));
    }
    Ok(relative)
}

/// Stable `/`-separated spelling used in validation and error messages.
fn normalized_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().replace('\\', "/")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    fn input(role: &str, path: &str, sha256: &str) -> InputRef {
        InputRef {
            role: role.to_string(),
            path: path.to_string(),
            sha256: sha256.to_string(),
        }
    }

    fn staged(role: &str, path: &str) -> StagedFile {
        StagedFile {
            input_role: role.to_string(),
            path: path.to_string(),
        }
    }

    const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn rejects_traversal_absolute_and_placeholder_destinations() {
        let inputs = vec![input("rawFont", "font.ttf", ZERO)];
        for path in [
            "../escape.ttf",
            "a/../../escape.ttf",
            "/tmp/escape.ttf",
            "a/./b.ttf",
            "a//b.ttf",
            "{workspace}/font.ttf",
            "",
        ] {
            let error = validate_staged_files("w", &inputs, &[staged("rawFont", path)])
                .expect_err("malformed destination must be rejected");
            assert!(
                error.contains("stagedFiles"),
                "unexpected diagnostic for {path:?}: {error}"
            );
        }
    }

    #[test]
    fn rejects_undeclared_roles_duplicates_and_bad_roles() {
        let inputs = vec![input("rawFont", "font.ttf", ZERO)];
        let error = validate_staged_files("w", &inputs, &[staged("other", "font.ttf")])
            .expect_err("undeclared role must be rejected");
        assert!(error.contains("not a declared input role"));

        let error = validate_staged_files(
            "w",
            &inputs,
            &[
                staged("rawFont", "a/font.ttf"),
                staged("rawFont", "a/font.ttf"),
            ],
        )
        .expect_err("duplicate destination must be rejected");
        assert!(error.contains("duplicate destination"));

        let error = validate_staged_files("w", &inputs, &[staged("bad role", "font.ttf")])
            .expect_err("malformed role must be rejected");
        assert!(error.contains("inputRole"));
    }

    #[test]
    fn stages_verified_input_without_hardlink_and_detects_mutation() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("private-cache.ttf");
        let bytes = b"frozen fixture bytes".to_vec();
        fs::write(&source, &bytes).expect("write source");
        let digest = super::super::sha256_bytes(&bytes);

        let inputs = vec![input("rawFont", "private-cache.ttf", &digest)];
        let staged_files = vec![staged("rawFont", "cache/font.ttf")];
        validate_staged_files("w", &inputs, &staged_files).expect("valid staging spec");

        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        let staged_inputs = stage_declared_inputs(temp.path(), &inputs, &staged_files, &workspace)
            .expect("stage input");
        assert_eq!(
            fs::read(workspace.join("cache/font.ttf")).expect("staged bytes"),
            bytes
        );
        #[cfg(unix)]
        {
            let source_meta = fs::metadata(&source).expect("source meta");
            let staged_meta = fs::metadata(workspace.join("cache/font.ttf")).expect("staged meta");
            assert_ne!(
                source_meta.ino(),
                staged_meta.ino(),
                "must not be a hard link"
            );
            assert_eq!(staged_meta.nlink(), 1);
        }

        fs::write(workspace.join("cache/font.ttf"), b"mutated").expect("mutate copy");
        let errors = verify_staged_inputs(&staged_inputs);
        assert!(errors.iter().any(|error| error.contains("staged input")));

        let staged_inputs = stage_declared_inputs(temp.path(), &inputs, &staged_files, &workspace)
            .expect("restage input");
        fs::write(&source, b"mutated source").expect("mutate source");
        let errors = verify_staged_inputs(&staged_inputs);
        assert!(errors
            .iter()
            .any(|error| error.contains("source was modified")));
    }

    #[test]
    fn rejects_staging_a_source_that_does_not_match_its_pin() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::write(temp.path().join("font.ttf"), b"actual").expect("write source");
        let inputs = vec![input("rawFont", "font.ttf", ZERO)];
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        let error = stage_declared_inputs(
            temp.path(),
            &inputs,
            &[staged("rawFont", "font.ttf")],
            &workspace,
        )
        .expect_err("hash mismatch must abort staging");
        assert!(
            error.contains("sha256 mismatch"),
            "unexpected error: {error}"
        );
        assert!(!workspace.join("font.ttf").exists());
    }
}
