use crate::modules::devlauncher::models::*;
use std::fs;
use std::path::PathBuf;

pub trait ProfileManager: Send + Sync {
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String>;
    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String>;
    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String>;
    fn delete_profile(&self, name: &str) -> Result<(), String>;
    /// Find an existing profile whose project_path matches the given path.
    fn find_by_project_path(&self, path: &str) -> Option<LaunchProfile>;
}

pub struct JsonProfileManager {
    profiles_dir: PathBuf,
}

impl JsonProfileManager {
    pub fn new(profiles_dir: PathBuf) -> Self {
        Self { profiles_dir }
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{}.json", name))
    }
}

impl ProfileManager for JsonProfileManager {
    fn list_profiles(&self) -> Result<Vec<LaunchProfile>, String> {
        let mut profiles = Vec::new();
        let entries = fs::read_dir(&self.profiles_dir)
            .map_err(|e| format!("Failed to read profiles dir: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                let content = fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
                let profile: LaunchProfile = serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
                profiles.push(profile);
            }
        }
        Ok(profiles)
    }

    fn get_profile(&self, name: &str) -> Result<LaunchProfile, String> {
        let path = self.profile_path(name);
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read profile '{}': {}", name, e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse profile '{}': {}", name, e))
    }

    fn save_profile(&self, profile: &LaunchProfile) -> Result<(), String> {
        let path = self.profile_path(&profile.name);
        let content = serde_json::to_string_pretty(profile)
            .map_err(|e| format!("Failed to serialize profile: {}", e))?;
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write profile '{}': {}", profile.name, e))
    }

    fn delete_profile(&self, name: &str) -> Result<(), String> {
        let path = self.profile_path(name);
        fs::remove_file(&path).map_err(|e| format!("Failed to delete profile '{}': {}", name, e))
    }

    fn find_by_project_path(&self, path: &str) -> Option<LaunchProfile> {
        let profiles = self.list_profiles().ok()?;
        profiles.into_iter().find(|p| {
            p.project_path
                .as_deref()
                .map(|pp| normalize_path(pp) == normalize_path(path))
                .unwrap_or(false)
        })
    }
}

/// Normalize a path for comparison: convert backslashes to forward slashes
/// and strip trailing separators.
fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}
