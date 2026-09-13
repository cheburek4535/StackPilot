// ============================================================
// Docker authorization state — devlauncher
// ============================================================
// Small file-backed store that answers one question: has the user
// EVER got Docker working through a profile run?
//
//   - `confirmed` starts false (fresh installs, never-run Docker).
//   - It is flipped to true the moment ANY docker-involved step
//     (`wait_for_docker`, a docker `run_command`/compose bootstrap)
//     reaches `Succeeded`. Failed attempts never count — a single
//     successful Docker run proves the daemon booted, which in turn
//     proves the Docker Desktop first-run flow (sign in / accept the
//     service agreement) was completed.
//
// Until `confirmed`, every Docker failure is treated as a likely
// "the user must authorize in Docker Desktop" signal, and both the
// backend (error hint) and the frontend (callouts, "try again")
// act accordingly.
//
// `installed_via_stackpilot` is NOT persisted — it is derived live
// from the ToolchainManager state: docker present in the `tools`
// map means our own installer put it there; the `adopted` map is
// for manually tracked installs of external origin.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::modules::toolchain::ToolchainState;

/// Persistent payload of `docker_auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerAuthData {
    /// True once at least one docker-involved step has ever succeeded.
    #[serde(default)]
    pub confirmed: bool,
}

/// Read-only view returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerAuthStateView {
    pub confirmed: bool,
    pub installed_via_stackpilot: bool,
}

/// File-backed store (data_dir/devlauncher/docker_auth.json).
pub struct DockerAuthStore {
    path: PathBuf,
    data: Mutex<DockerAuthData>,
}

/// Human-readable hint appended to docker step failures while the user's
/// Docker authorization is unconfirmed. Keep it actionable and short: the
/// text lands inside a step error line and inside diagnostics.
pub const DOCKER_AUTH_HINT: &str = concat!(
    "Если Docker Desktop только что установлен — откройте его и ",
    "авторизуйтесь (Sign in) или примите условия использования, ",
    "затем нажмите «Попробовать снова»."
);

impl DockerAuthStore {
    /// Loads the store from `dir` (usually data_dir/devlauncher).
    /// Missing or unreadable file → clean default (`confirmed: false`).
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("docker_auth.json");
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            data: Mutex::new(data),
        }
    }

    /// Whether Docker authorization is confirmed (≥1 successful docker run).
    pub fn confirmed(&self) -> bool {
        self.data
            .lock()
            .expect("docker_auth poisoned")
            .confirmed
    }

    /// Set the confirmed flag, persisting to disk when it changes.
    pub fn set_confirmed(&self, value: bool) {
        {
            let mut data = self.data.lock().expect("docker_auth poisoned");
            if data.confirmed == value {
                return;
            }
            data.confirmed = value;
        }
        self.save();
    }

    /// Atomic-ish write: serialize to memory then write the file. Logged
    /// (not fatal) — a failed write must never break a running orchestration.
    fn save(&self) {
        let data = self.data.lock().expect("docker_auth poisoned");
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("[devlauncher] docker_auth dir create failed: {e}");
                return;
            }
        }
        match serde_json::to_string_pretty(&*data) {
            Ok(raw) => {
                if let Err(e) = std::fs::write(&self.path, raw) {
                    log::error!("[devlauncher] docker_auth.json write failed: {e}");
                }
            }
            Err(e) => log::error!("[devlauncher] docker_auth.json serialize failed: {e}"),
        }
    }

    /// Whether the ToolchainManager installed Docker itself (state.json
    /// `tools` map has a docker entry). NOT the `adopted` map — adopted
    /// tools are tracked manual installs of external origin.
    pub fn is_installed_via_stackpilot(toolchain: &ToolchainState) -> bool {
        toolchain
            .metadata()
            .lock()
            .expect("metadata poisoned")
            .data()
            .tools
            .contains_key("docker")
    }

    /// Combined read-only view for the frontend.
    pub fn view(&self, toolchain: &ToolchainState) -> DockerAuthStateView {
        DockerAuthStateView {
            confirmed: self.confirmed(),
            installed_via_stackpilot: Self::is_installed_via_stackpilot(toolchain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sp-dockerauth-{tag}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_to_unconfirmed_when_missing() {
        let store = DockerAuthStore::load(&temp_dir("missing"));
        assert!(!store.confirmed());
    }

    #[test]
    fn set_confirmed_persists_across_reload() {
        let dir = temp_dir("persist");
        let store = DockerAuthStore::load(&dir);
        assert!(!store.confirmed());
        store.set_confirmed(true);
        assert!(store.confirmed());

        let reloaded = DockerAuthStore::load(&dir);
        assert!(reloaded.confirmed(), "confirmed flag must survive restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_gives_default() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join("docker_auth.json"), "{ not json").unwrap();
        let store = DockerAuthStore::load(&dir);
        assert!(!store.confirmed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}