use crate::modules::devlauncher::models::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public persistence types
// ---------------------------------------------------------------------------

/// Per-file parse diagnostic produced by a tolerant directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDiagnostic {
    pub file: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

/// Result of a tolerant directory listing: valid profiles plus per-file
/// diagnostics. One malformed profile never prevents the others from
/// loading — parse failures are reported as diagnostics, not hard errors.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileLoadResult {
    pub profiles: Vec<LaunchProfileV2>,
    pub diagnostics: Vec<ProfileDiagnostic>,
}

impl ProfileLoadResult {
    fn diag(
        &mut self,
        file: impl Into<String>,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(ProfileDiagnostic {
            file: file.into(),
            severity,
            message: message.into(),
        });
    }
}

/// Report from [`ProfileManagerV2::migrate_legacy`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationReport {
    /// Files rewritten from the legacy schema (v1 `actions`) to V2.
    pub migrated: Vec<String>,
    /// Files already in V2 format — left untouched.
    pub already_v2: Vec<String>,
    /// Files that could not be read or parsed.
    pub failed: Vec<MigrationFailure>,
    /// Files skipped for other reasons (e.g. temporary files).
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationFailure {
    pub file: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Legacy profile manager API — retained for backward compatibility.
/// All methods operate on V2 storage internally and round-trip through
/// the legacy model.
pub trait ProfileManager: Send + Sync {
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String>;
    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String>;
    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String>;
    fn delete_profile(&self, name: &str) -> Result<(), String>;
    /// Find an existing profile whose project_path matches the given path.
    /// Retained for API compatibility; prefer `ProfileManagerV2`.
    #[allow(dead_code)]
    fn find_by_project_path(&self, path: &str) -> Option<LaunchProfile>;
}

/// Versioned profile manager API.
pub trait ProfileManagerV2: Send + Sync {
    /// List all profiles (V2 format). Tolerant: malformed files are skipped
    /// with diagnostics and never abort the listing.
    fn list_profiles_v2(&self) -> Result<Vec<LaunchProfileV2>, String>;
    /// List all profiles together with per-file diagnostics.
    fn list_profiles_with_diagnostics(&self) -> Result<ProfileLoadResult, String>;
    /// Look up a profile by its `name` field (first match, deterministic order).
    fn get_profile_v2(&self, name: &str) -> Result<LaunchProfileV2, String>;
    /// Look up a profile by its stable profile ID.
    /// Retained for API compatibility; the CRUD command surface uses names.
    #[allow(dead_code)]
    fn get_profile_by_id(&self, id: &str) -> Result<LaunchProfileV2, String>;
    /// Persist a profile atomically. The stable profile ID is preserved or
    /// adopted from an existing profile with the same name / project path.
    fn save_profile_v2(&self, profile: &LaunchProfileV2) -> Result<(), String>;
    /// Delete the profile with the given name (safe: only deletes files
    /// inside the profiles directory that were discovered by listing).
    fn delete_profile_v2(&self, name: &str) -> Result<(), String>;
    /// Delete the profile with the given stable ID.
    fn delete_profile_by_id(&self, id: &str) -> Result<(), String>;
    /// Find an existing profile whose project root matches the given path.
    fn find_by_project_path_v2(&self, path: &str) -> Option<LaunchProfileV2>;
    /// Rewrite all legacy (v1) profile files on disk to the V2 schema.
    fn migrate_legacy(&self) -> Result<MigrationReport, String>;
}

// ---------------------------------------------------------------------------
// JsonProfileManager
// ---------------------------------------------------------------------------

pub struct JsonProfileManager {
    profiles_dir: PathBuf,
}

impl JsonProfileManager {
    pub fn new(profiles_dir: PathBuf) -> Self {
        Self { profiles_dir }
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.profiles_dir).map_err(|e| {
            format!(
                "Failed to create profiles dir '{}': {}",
                self.profiles_dir.display(),
                e
            )
        })
    }

    /// Safe file name for a profile: sanitized name + short stable ID.
    /// The raw `name` field is NEVER used directly as a path component,
    /// so profile names cannot escape the profiles directory.
    fn filename_for(&self, profile: &LaunchProfileV2) -> PathBuf {
        let safe = sanitize_filename(&profile.name);
        let id8: String = profile.id.chars().take(8).collect();
        let id8 = if id8.is_empty() {
            "profile".to_string()
        } else {
            id8
        };
        self.profiles_dir.join(format!("{}-{}.json", safe, id8))
    }

    /// Scan the profiles directory for profile JSON files (direct children
    /// only, sorted by name for deterministic ordering). Temporary files
    /// (`.name.<uuid>.tmp`) are excluded.
    fn scan_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        let Ok(entries) = fs::read_dir(&self.profiles_dir) else {
            return files;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.ends_with(".json") || name.starts_with('.') || name.ends_with(".tmp") {
                continue;
            }
            files.push(path);
        }
        files.sort();
        files
    }

    /// Parse a profile file's content. Tries V2 first (unknown fields are
    /// preserved into `extra`), then falls back to the legacy v1 schema.
    ///
    /// A bare legacy file must NOT be accepted as V2: its `actions` would
    /// be dropped into `extra` and the profile would silently load with
    /// zero steps. The schema is decided from the document shape — `steps`
    /// marks V2, `actions` without `steps` marks legacy.
    fn parse_profile_file(content: &str) -> Result<LoadOutcome, String> {
        let value: serde_json::Value =
            serde_json::from_str(content).map_err(|e| format!("Malformed JSON: {}", e))?;

        let has_v2_steps = value.get("steps").map(|s| s.is_array()).unwrap_or(false);
        let has_legacy_actions = value.get("actions").map(|a| a.is_array()).unwrap_or(false);

        if has_v2_steps {
            let profile: LaunchProfileV2 =
                serde_json::from_value(value).map_err(|e| format!("V2 parse failed: {}", e))?;
            return Ok(LoadOutcome::V2(profile));
        }
        if has_legacy_actions {
            let legacy: LaunchProfile =
                serde_json::from_value(value).map_err(|e| format!("Legacy parse failed: {}", e))?;
            return Ok(LoadOutcome::Legacy(legacy));
        }
        // No actions and no steps: accept as V2 (an empty profile is
        // valid), otherwise report the V2 parse error.
        let profile: LaunchProfileV2 =
            serde_json::from_value(value).map_err(|e| format!("V2 parse failed: {}", e))?;
        Ok(LoadOutcome::V2(profile))
    }

    fn load_result(&self) -> ProfileLoadResult {
        let mut result = ProfileLoadResult::default();

        if !self.profiles_dir.exists() {
            result.diag(
                self.profiles_dir.display().to_string(),
                DiagnosticSeverity::Info,
                "Profiles directory does not exist yet; it will be created on first save",
            );
            return result;
        }

        for path in self.scan_files() {
            let file = path.display().to_string();
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    result.diag(
                        file.clone(),
                        DiagnosticSeverity::Error,
                        format!("Failed to read: {}", e),
                    );
                    continue;
                }
            };

            match Self::parse_profile_file(&content) {
                Ok(LoadOutcome::V2(profile)) => {
                    if profile.name.trim().is_empty() {
                        result.diag(
                            file.clone(),
                            DiagnosticSeverity::Warning,
                            "Profile has an empty name; loaded anyway",
                        );
                    }
                    result.profiles.push(profile);
                }
                Ok(LoadOutcome::Legacy(legacy)) => {
                    result.diag(
                        file.clone(),
                        DiagnosticSeverity::Warning,
                        format!(
                            "Legacy schema detected for profile '{}'; converted in memory. Run migrate_legacy to rewrite the file.",
                            legacy.name
                        ),
                    );
                    result.profiles.push(migrate_legacy_profile(legacy));
                }
                Err(e) => {
                    result.diag(
                        file.clone(),
                        DiagnosticSeverity::Error,
                        format!("Malformed JSON: {}", e),
                    );
                }
            }
        }

        // Deterministic ordering: by name (case-insensitive), then by ID.
        result.profiles.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });

        result
    }

    /// Find a previously persisted profile to reuse its stable ID:
    /// by ID first, then by name, then by project root.
    fn find_for_stable_id(
        &self,
        id: &str,
        name: &str,
        project_root: Option<&str>,
    ) -> Option<LaunchProfileV2> {
        let loaded = self.load_result().profiles;
        loaded
            .iter()
            .find(|p| !p.id.is_empty() && p.id == id)
            .or_else(|| loaded.iter().find(|p| p.name == name))
            .or_else(|| {
                project_root.and_then(|root| {
                    let wanted = normalize_path(root);
                    loaded.iter().find(|p| {
                        p.project_root.as_deref().map(normalize_path).as_deref()
                            == Some(wanted.as_str())
                    })
                })
            })
            .cloned()
    }

    fn find_path_for_id(&self, id: &str) -> Option<PathBuf> {
        let loaded = self.load_result();
        let profile = loaded.profiles.iter().find(|p| p.id == id)?;
        Some(self.filename_for(profile))
    }

    /// Find the on-disk path of the profile with the given name.
    fn find_path_for_name(&self, name: &str) -> Option<PathBuf> {
        let loaded = self.load_result();
        let profile = loaded
            .profiles
            .iter()
            .find(|p| p.name == name)
            .or_else(|| {
                loaded
                    .profiles
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
            })?;
        Some(self.filename_for(profile))
    }

    /// Atomic write: serialize to a temp file in the same directory, fsync,
    /// then rename over the destination. On Windows `std::fs::rename` cannot
    /// replace an existing file, so the destination is removed first (a tiny
    /// window during which the profile is absent — the previous content is
    /// never left half-written, and a crash during the write leaves only the
    /// temp file, never a corrupted profile).
    fn atomic_write(&self, path: &Path, content: &str) -> Result<(), String> {
        let dir = path
            .parent()
            .ok_or_else(|| format!("No parent directory for {}", path.display()))?;
        let base = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("profile.json");
        let tmp = dir.join(format!(".{}.{}.tmp", base, generate_stable_id()));

        let write_result = (|| -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
            Ok(())
        })();

        if let Err(e) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "Failed to write temp file '{}': {}",
                tmp.display(),
                e
            ));
        }

        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)
                .map_err(|e| format!("Failed to replace '{}': {}", path.display(), e))?;
        }

        fs::rename(&tmp, path).map_err(|e| {
            format!(
                "Failed to rename '{}' -> '{}': {}",
                tmp.display(),
                path.display(),
                e
            )
        })
    }
}

enum LoadOutcome {
    V2(LaunchProfileV2),
    Legacy(LaunchProfile),
}

// ---------------------------------------------------------------------------
// Legacy API
// ---------------------------------------------------------------------------

impl ProfileManager for JsonProfileManager {
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String> {
        Ok(self
            .load_result()
            .profiles
            .into_iter()
            .map(LaunchProfile::from)
            .collect())
    }

    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String> {
        self.get_profile_v2(name).map(LaunchProfile::from)
    }

    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String> {
        let existing = self.find_for_stable_id("", &profile.name, profile.project_path.as_deref());
        let v2 = merge_legacy_into_v2(existing.as_ref(), profile.clone());
        self.save_profile_v2(&v2)
    }

    fn delete_profile(&self, name: &str) -> Result<(), String> {
        self.delete_profile_v2(name)
    }

    fn find_by_project_path(&self, path: &str) -> Option<LaunchProfile> {
        self.find_by_project_path_v2(path).map(LaunchProfile::from)
    }
}

// ---------------------------------------------------------------------------
// V2 API
// ---------------------------------------------------------------------------

impl ProfileManagerV2 for JsonProfileManager {
    fn list_profiles_v2(&self) -> Result<Vec<LaunchProfileV2>, String> {
        Ok(self.load_result().profiles)
    }

    fn list_profiles_with_diagnostics(&self) -> Result<ProfileLoadResult, String> {
        Ok(self.load_result())
    }

    fn get_profile_v2(&self, name: &str) -> Result<LaunchProfileV2, String> {
        let loaded = self.load_result().profiles;
        loaded
            .iter()
            .find(|p| p.name == name)
            .or_else(|| {
                // Case-insensitive fallback for convenience.
                loaded.iter().find(|p| p.name.eq_ignore_ascii_case(name))
            })
            .cloned()
            .ok_or_else(|| format!("Profile '{}' not found", name))
    }

    fn get_profile_by_id(&self, id: &str) -> Result<LaunchProfileV2, String> {
        let loaded = self.load_result().profiles;
        loaded
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Profile with id '{}' not found", id))
    }

    fn save_profile_v2(&self, profile: &LaunchProfileV2) -> Result<(), String> {
        self.ensure_dir()?;

        let mut profile = profile.clone();
        if profile.id.trim().is_empty() {
            profile.id = generate_stable_id();
        }
        if profile.schema_version.trim().is_empty() {
            profile.schema_version = PROFILE_SCHEMA_VERSION.to_string();
        }
        if profile.name.trim().is_empty() {
            return Err("Profile name must not be empty".to_string());
        }

        // Adopt the stable ID of an existing profile (same ID, name or
        // project root) so repeated saves update the same file.
        if let Some(existing) =
            self.find_for_stable_id(&profile.id, &profile.name, profile.project_root.as_deref())
        {
            profile.id = existing.id.clone();
        }

        let target = self.filename_for(&profile);

        // Remove a stale file if the profile previously lived under a
        // different (sanitized name, id) combination.
        if let Some(prev) = self.find_path_for_id(&profile.id) {
            if prev != target && prev.starts_with(&self.profiles_dir) {
                let _ = fs::remove_file(&prev);
            }
        }

        let content = serde_json::to_string_pretty(&profile)
            .map_err(|e| format!("Failed to serialize profile '{}': {}", profile.name, e))?;
        self.atomic_write(&target, &content)
    }

    fn delete_profile_v2(&self, name: &str) -> Result<(), String> {
        let path = self
            .find_path_for_name(name)
            .ok_or_else(|| format!("Profile '{}' not found", name))?;
        self.remove_if_inside(&path, name)
    }

    fn delete_profile_by_id(&self, id: &str) -> Result<(), String> {
        let path = self
            .find_path_for_id(id)
            .ok_or_else(|| format!("Profile with id '{}' not found", id))?;
        self.remove_if_inside(&path, id)
    }

    fn find_by_project_path_v2(&self, path: &str) -> Option<LaunchProfileV2> {
        let wanted = normalize_path(path);
        self.load_result()
            .profiles
            .into_iter()
            .find(|p| p.project_root.as_deref().map(normalize_path) == Some(wanted.clone()))
    }

    fn migrate_legacy(&self) -> Result<MigrationReport, String> {
        self.ensure_dir()?;
        let mut report = MigrationReport::default();

        for path in self.scan_files() {
            let file = path.display().to_string();
            if !path.starts_with(&self.profiles_dir) {
                report.skipped.push(file);
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    report.failed.push(MigrationFailure {
                        file,
                        error: format!("Failed to read: {}", e),
                    });
                    continue;
                }
            };

            match Self::parse_profile_file(&content) {
                Ok(LoadOutcome::V2(_)) => report.already_v2.push(file),
                Ok(LoadOutcome::Legacy(legacy)) => {
                    let v2 = migrate_legacy_profile(legacy);
                    let content = match serde_json::to_string_pretty(&v2) {
                        Ok(c) => c,
                        Err(e) => {
                            report.failed.push(MigrationFailure {
                                file: file.clone(),
                                error: format!("Failed to serialize: {}", e),
                            });
                            continue;
                        }
                    };
                    let target = self.filename_for(&v2);
                    if let Err(e) = self.atomic_write(&target, &content) {
                        report.failed.push(MigrationFailure {
                            file: file.clone(),
                            error: e,
                        });
                        continue;
                    }
                    if target != path {
                        // Relocate a legacy file (which may have been named
                        // after an unsanitized profile name) to the safe
                        // naming scheme. The old path came from scan_files,
                        // so it is inside the profiles directory.
                        let _ = fs::remove_file(&path);
                    }
                    report.migrated.push(file);
                }
                Err(e) => report.failed.push(MigrationFailure {
                    file,
                    error: format!("Malformed JSON: {}", e),
                }),
            }
        }

        Ok(report)
    }
}

impl JsonProfileManager {
    /// Delete a profile file, refusing to touch anything outside the
    /// profiles directory (defense in depth against path traversal).
    fn remove_if_inside(&self, path: &Path, what: &str) -> Result<(), String> {
        if !path.starts_with(&self.profiles_dir) {
            return Err(format!(
                "Refusing to delete '{}': outside profiles directory",
                path.display()
            ));
        }
        fs::remove_file(path).map_err(|e| format!("Failed to delete profile '{}': {}", what, e))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sanitize a profile name into a safe file-name component.
///
/// Only `[A-Za-z0-9._-]` survive; everything else becomes `-`, runs of
/// separators collapse, hidden-file dots are neutralized, and Windows
/// reserved device names are prefixed. The result can never contain path
/// separators or a `..` component.
fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_sep = false;
    for ch in name.chars() {
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-');
        if keep {
            out.push(ch);
            last_sep = false;
        } else if !last_sep {
            out.push('-');
            last_sep = true;
        }
    }
    while out.ends_with('-') || out.ends_with('.') {
        out.pop();
    }
    if out.starts_with('.') {
        out.insert(0, 'p');
    }
    if out.is_empty() {
        out = "profile".to_string();
    }
    let upper = out.to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&upper.as_str()) {
        out = format!("profile-{}", out);
    }
    out
}

/// Normalize a path for comparison: backslashes → forward slashes,
/// trailing separators stripped, ASCII-lowercased on Windows.
fn normalize_path(p: &str) -> String {
    let normalized = p.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/').to_string();
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// Merge a legacy `LaunchProfile` into an existing V2 profile so that
/// V2-only step properties (dependencies, environment, policies, unknown
/// forward-compatible fields) survive a legacy-API save.
///
/// - Fresh profiles (no existing V2) are migrated with sequential
///   dependency chaining, preserving legacy run order.
/// - Updates keep the existing V2 graph shape: steps are matched by action
///   ID and only the legacy-editable fields (kind, label, enabled,
///   working_directory) are refreshed; steps whose IDs no longer exist are
///   appended as-is.
fn merge_legacy_into_v2(
    existing: Option<&LaunchProfileV2>,
    legacy: LaunchProfile,
) -> LaunchProfileV2 {
    let mut steps: Vec<LaunchStep> = legacy.actions.into_iter().map(LaunchStep::from).collect();

    match existing {
        Some(old) => {
            let old_steps: HashMap<&str, &LaunchStep> =
                old.steps.iter().map(|s| (s.id.as_str(), s)).collect();
            for step in &mut steps {
                if let Some(old_step) = old_steps.get(step.id.as_str()) {
                    step.depends_on = old_step.depends_on.clone();
                    step.environment = old_step.environment.clone();
                    step.visibility = old_step.visibility.clone();
                    step.execution_mode = old_step.execution_mode.clone();
                    step.completion = old_step.completion.clone();
                    step.timeout = old_step.timeout;
                    step.failure_policy = old_step.failure_policy.clone();
                    step.retry_policy = old_step.retry_policy.clone();
                    step.metadata = old_step.metadata.clone();
                    step.extra = old_step.extra.clone();
                    // The legacy action model cannot express `candidate_ports`
                    // (dev-server fallback ports). Carry them over from the
                    // existing V2 step so a legacy-API save never drops them.
                    if let (
                        StepKind::WaitForPort {
                            candidate_ports, ..
                        },
                        StepKind::WaitForPort {
                            candidate_ports: old_candidates,
                            ..
                        },
                    ) = (&mut step.kind, &old_step.kind)
                    {
                        if !old_candidates.is_empty() {
                            *candidate_ports = old_candidates.clone();
                        }
                    }
                }
            }
            // The legacy edit may have removed steps the old graph depended
            // on (e.g. a compose step deleted from the action list). Drop
            // dangling dependencies so the merged profile still validates.
            let live_ids: HashSet<String> = steps.iter().map(|s| s.id.clone()).collect();
            for step in &mut steps {
                step.depends_on.retain(|dep| live_ids.contains(dep));
            }
        }
        None => {
            for i in 1..steps.len() {
                let previous = steps[i - 1].id.clone();
                steps[i].depends_on.push(previous);
            }
        }
    }

    LaunchProfileV2 {
        schema_version: PROFILE_SCHEMA_VERSION.to_string(),
        id: existing
            .map(|p| p.id.clone())
            .unwrap_or_else(generate_stable_id),
        name: legacy.name,
        description: legacy.description,
        project_root: legacy.project_path,
        steps,
        environment_binding_id: legacy.environment_binding_id,
        preferred_ide: legacy.preferred_ide,
        default_execution_mode: existing.and_then(|p| p.default_execution_mode.clone()),
        extra: existing.map(|p| p.extra.clone()).unwrap_or_default(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::time::SystemTime;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_test_{}_{}_{}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_v2(name: &str, project_root: Option<&str>) -> LaunchProfileV2 {
        LaunchProfileV2 {
            schema_version: PROFILE_SCHEMA_VERSION.to_string(),
            id: generate_stable_id(),
            name: name.to_string(),
            description: "test".to_string(),
            project_root: project_root.map(String::from),
            steps: vec![LaunchStep {
                id: "s1".to_string(),
                label: "step".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "echo hi".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            }],
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: Map::new(),
        }
    }

    fn legacy_fixture() -> LaunchProfile {
        LaunchProfile {
            name: "Legacy App".to_string(),
            description: "legacy".to_string(),
            project_path: Some("/proj".to_string()),
            actions: vec![
                LaunchAction {
                    id: "act_1".to_string(),
                    label: "Start".to_string(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".to_string(),
                        working_dir: Some("./backend".to_string()),
                        persistent: Some(true),
                    },
                },
                LaunchAction {
                    id: "act_2".to_string(),
                    label: "Wait".to_string(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".to_string(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                },
            ],
            environment_binding_id: None,
            preferred_ide: Some(PreferredIde::Vscode),
            schema_version: None,
            id: None,
            steps: None,
        }
    }

    #[test]
    fn sanitize_filename_removes_dangerous_chars() {
        // Runs of unsafe characters collapse into a single dash.
        assert_eq!(
            sanitize_filename("My Project (Backend)"),
            "My-Project-Backend"
        );
        // Path separators become dashes; leading dots (hidden files on
        // Unix) are neutralized with a `p` prefix.
        assert_eq!(sanitize_filename("../../etc/passwd"), "p..-..-etc-passwd");
        assert_eq!(sanitize_filename("CON"), "profile-CON");
        assert_eq!(sanitize_filename("..."), "profile");
        assert_eq!(sanitize_filename(""), "profile");
        assert_eq!(sanitize_filename(".hidden"), "p.hidden");
        for n in ["../../x", "a/b\\c", "CON", "...", ""] {
            let s = sanitize_filename(n);
            // Never a path separator, never a hidden file, never "..".
            assert!(!s.contains('/') && !s.contains('\\'), "{} -> {}", n, s);
            assert!(!s.starts_with('.'), "{} -> {}", n, s);
            assert_ne!(s, "..", "{} -> {}", n, s);
        }
    }

    #[test]
    fn safe_filenames_never_escape_directory() {
        let dir = temp_dir("names");
        let manager = JsonProfileManager::new(dir.clone());

        let evil = sample_v2("../../evil", None);
        let path = manager.filename_for(&evil);
        assert!(path.starts_with(&dir), "path escaped: {}", path.display());
        let id8: String = evil.id.chars().take(8).collect();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("p..-..-evil-{}.json", id8)
        );

        let reserved = sample_v2("CON", None);
        let path = manager.filename_for(&reserved);
        assert!(path.starts_with(&dir));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_save_creates_directory_and_file() {
        let dir = temp_dir("atomic");
        let manager = JsonProfileManager::new(dir.join("profiles"));
        let profile = sample_v2("Atomic", None);
        manager.save_profile_v2(&profile).unwrap();

        let path = manager.filename_for(&profile);
        assert!(path.exists(), "profile file must exist");
        // No temp files left behind
        let leftovers: Vec<_> = fs::read_dir(dir.join("profiles"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind");

        // Content is valid V2 with schema_version
        let content = fs::read_to_string(&path).unwrap();
        let parsed: LaunchProfileV2 = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.name, "Atomic");
        assert_eq!(parsed.schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(parsed.id, profile.id);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_save_preserves_stable_id_and_filename() {
        let dir = temp_dir("stable");
        let manager = JsonProfileManager::new(dir.clone());
        let profile = sample_v2("Stable", None);
        manager.save_profile_v2(&profile).unwrap();
        let path1 = manager.filename_for(&profile);

        // Save again with the same name but a fresh ID -- the manager must
        // adopt the stable ID of the existing profile (matched by name).
        let mut again = profile.clone();
        again.id = generate_stable_id();
        manager.save_profile_v2(&again).unwrap();

        // The on-disk file is still the original one (adopted ID), and a
        // stale file for the abandoned ID must not linger.
        assert!(path1.exists(), "original file must survive");
        let listed = manager.list_profiles_v2().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, profile.id);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tolerant_listing_skips_malformed_files() {
        let dir = temp_dir("tolerant");
        let manager = JsonProfileManager::new(dir.clone());
        manager.save_profile_v2(&sample_v2("Good", None)).unwrap();
        fs::write(dir.join("broken.json"), "{ not json").unwrap();
        fs::write(dir.join("readme.md"), "hello").unwrap();

        let result = manager.list_profiles_with_diagnostics().unwrap();
        assert_eq!(result.profiles.len(), 1);
        assert_eq!(result.profiles[0].name, "Good");
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error));

        // Legacy API also survives
        let legacy = manager.list_profiles().unwrap();
        assert_eq!(legacy.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_missing_directory_is_tolerant() {
        let dir = temp_dir("missing");
        let manager = JsonProfileManager::new(dir.join("does-not-exist"));
        let result = manager.list_profiles_with_diagnostics().unwrap();
        assert!(result.profiles.is_empty());
        assert!(!result.diagnostics.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_profile_does_not_block_valid_profiles() {
        let dir = temp_dir("mixed");
        let manager = JsonProfileManager::new(dir.clone());
        let alpha = sample_v2("Alpha", None);
        let beta = sample_v2("Beta", None);
        manager.save_profile_v2(&alpha).unwrap();
        manager.save_profile_v2(&beta).unwrap();
        // Overwrite Beta's file (same instance → same filename) with garbage
        let beta_path = manager.filename_for(&beta);
        fs::write(&beta_path, "{{{{").unwrap();

        let result = manager.list_profiles_with_diagnostics().unwrap();
        assert_eq!(result.profiles.len(), 1);
        assert_eq!(result.profiles[0].name, "Alpha");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_file_loads_and_migrates_in_memory() {
        let dir = temp_dir("legacy");
        let manager = JsonProfileManager::new(dir.clone());
        fs::write(
            dir.join("legacy.json"),
            serde_json::to_string_pretty(&legacy_fixture()).unwrap(),
        )
        .unwrap();

        let result = manager.list_profiles_with_diagnostics().unwrap();
        assert_eq!(result.profiles.len(), 1);
        let p = &result.profiles[0];
        assert_eq!(p.name, "Legacy App");
        assert_eq!(p.schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(p.project_root.as_deref(), Some("/proj"));
        // Steps are chained to preserve legacy sequential order
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].id, "act_1");
        assert_eq!(p.steps[1].depends_on, vec!["act_1".to_string()]);
        assert_eq!(p.steps[0].working_directory.as_deref(), Some("./backend"));
        assert!(matches!(p.preferred_ide, Some(PreferredIde::Vscode)));
        // A warning diagnostic mentions the legacy schema
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_legacy_rewrites_files_to_v2() {
        let dir = temp_dir("rewrite");
        let manager = JsonProfileManager::new(dir.clone());
        let legacy_path = dir.join("legacy.json");
        fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&legacy_fixture()).unwrap(),
        )
        .unwrap();

        let report = manager.migrate_legacy().unwrap();
        assert_eq!(report.migrated.len(), 1);
        assert!(report.already_v2.is_empty());

        // The legacy file is gone (relocated to safe naming)
        assert!(!legacy_path.exists());
        // The new file parses as V2
        let profiles = manager.list_profiles_v2().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Legacy App");
        assert_eq!(profiles[0].steps.len(), 2);
        assert_eq!(profiles[0].steps[1].depends_on, vec!["act_1".to_string()]);

        // Second run: everything already V2
        let report2 = manager.migrate_legacy().unwrap();
        assert_eq!(report2.already_v2.len(), 1);
        assert!(report2.migrated.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deterministic_ordering_by_name() {
        let dir = temp_dir("order");
        let manager = JsonProfileManager::new(dir.clone());
        manager.save_profile_v2(&sample_v2("zeta", None)).unwrap();
        manager.save_profile_v2(&sample_v2("Alpha", None)).unwrap();
        manager.save_profile_v2(&sample_v2("beta", None)).unwrap();

        let profiles = manager.list_profiles_v2().unwrap();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "beta", "zeta"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_and_delete_by_name_and_id() {
        let dir = temp_dir("crud");
        let manager = JsonProfileManager::new(dir.clone());
        let p = sample_v2("Crud", Some("/tmp/proj"));
        manager.save_profile_v2(&p).unwrap();

        let by_name = manager.get_profile_v2("Crud").unwrap();
        assert_eq!(by_name.id, p.id);
        let by_id = manager.get_profile_by_id(&p.id).unwrap();
        assert_eq!(by_id.name, "Crud");

        // Case-insensitive fallback
        let by_name_lc = manager.get_profile_v2("crud").unwrap();
        assert_eq!(by_name_lc.id, p.id);

        // find by project path
        assert!(manager.find_by_project_path_v2("/tmp/proj").is_some());
        assert!(manager.find_by_project_path_v2("/other").is_none());

        manager.delete_profile_by_id(&p.id).unwrap();
        assert!(manager.list_profiles_v2().unwrap().is_empty());
        assert!(manager.get_profile_v2("Crud").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_unknown_profile_errors() {
        let dir = temp_dir("delmiss");
        let manager = JsonProfileManager::new(dir.clone());
        assert!(manager.delete_profile_v2("nope").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_save_updates_existing_v2_preserving_graph() {
        let dir = temp_dir("merge");
        let manager = JsonProfileManager::new(dir.clone());

        // Persist a V2 profile with a real dependency graph.
        let mut profile = sample_v2("Graph", None);
        profile.steps = vec![
            LaunchStep {
                id: "compose".to_string(),
                label: "compose".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "docker compose up -d".to_string(),
                    command_spec: None,
                },
                depends_on: vec![],
                working_directory: None,
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
            LaunchStep {
                id: "backend".to_string(),
                label: "backend".to_string(),
                enabled: true,
                kind: StepKind::RunCommand {
                    command: "go run .".to_string(),
                    command_spec: None,
                },
                depends_on: vec!["compose".to_string()],
                working_directory: Some("./backend".to_string()),
                environment: None,
                visibility: None,
                execution_mode: None,
                completion: None,
                timeout: None,
                failure_policy: None,
                retry_policy: None,
                metadata: None,
                extra: Map::new(),
            },
        ];
        manager.save_profile_v2(&profile).unwrap();
        let saved_id = profile.id.clone();

        // Save a legacy profile with the same name -- the step IDs match
        // (backend) so the V2 graph must survive.
        let legacy = LaunchProfile {
            name: "Graph".to_string(),
            description: "updated".to_string(),
            project_path: Some("/proj".to_string()),
            actions: vec![LaunchAction {
                id: "backend".to_string(),
                label: "backend".to_string(),
                enabled: true,
                action_type: ActionType::RunCommand {
                    command: "go run .".to_string(),
                    working_dir: Some("./backend".to_string()),
                    persistent: Some(true),
                },
            }],
            environment_binding_id: None,
            preferred_ide: None,
            schema_version: None,
            id: None,
            steps: None,
        };
        manager.save_profile(&legacy).unwrap();

        let loaded = manager.get_profile_v2("Graph").unwrap();
        // Stable ID preserved; legacy API did not create a duplicate file.
        assert_eq!(loaded.id, saved_id);
        assert_eq!(manager.list_profiles_v2().unwrap().len(), 1);
        // The one surviving step keeps its dependency-free graph; the
        // compose step was removed by the legacy edit (new action list).
        assert_eq!(loaded.steps.len(), 1);
        assert_eq!(loaded.steps[0].id, "backend");
        assert!(loaded.steps[0].depends_on.is_empty());
        assert_eq!(loaded.project_root.as_deref(), Some("/proj"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_fields_survive_save_round_trip() {
        let dir = temp_dir("fwd");
        let manager = JsonProfileManager::new(dir.clone());
        let mut profile = sample_v2("Fwd", None);
        profile
            .extra
            .insert("future_field".to_string(), serde_json::json!({"a": 1}));
        manager.save_profile_v2(&profile).unwrap();

        let loaded = manager.get_profile_v2("Fwd").unwrap();
        assert_eq!(loaded.extra.get("future_field").unwrap()["a"], 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_save_preserves_candidate_ports_on_wait_steps() {
        let dir = temp_dir("merge_ports");
        let manager = JsonProfileManager::new(dir.clone());

        // V2 profile with a wait step that carries fallback ports.
        let mut profile = sample_v2("Ports", None);
        profile.steps = vec![LaunchStep {
            id: "wait".to_string(),
            label: "wait".to_string(),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: "127.0.0.1".to_string(),
                port: 5173,
                candidate_ports: vec![5174, 5175],
            },
            depends_on: vec![],
            working_directory: None,
            environment: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: Some(60),
            failure_policy: None,
            retry_policy: None,
            metadata: None,
            extra: Map::new(),
        }];
        manager.save_profile_v2(&profile).unwrap();

        // A legacy-API save cannot express candidate_ports, but the merge
        // must carry them over from the existing V2 step.
        let legacy = LaunchProfile {
            name: "Ports".to_string(),
            description: "updated".to_string(),
            project_path: Some("/proj".to_string()),
            actions: vec![LaunchAction {
                id: "wait".to_string(),
                label: "wait".to_string(),
                enabled: true,
                action_type: ActionType::WaitForPort {
                    host: "127.0.0.1".to_string(),
                    port: 5173,
                    timeout_secs: 60,
                },
            }],
            environment_binding_id: None,
            preferred_ide: None,
            schema_version: None,
            id: None,
            steps: None,
        };
        manager.save_profile(&legacy).unwrap();

        let loaded = manager.get_profile_v2("Ports").unwrap();
        match &loaded.steps[0].kind {
            StepKind::WaitForPort {
                candidate_ports, ..
            } => assert_eq!(candidate_ports, &vec![5174, 5175]),
            _ => panic!("expected WaitForPort"),
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
