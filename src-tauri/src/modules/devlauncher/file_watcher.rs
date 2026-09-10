use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

/// Watches a project directory for source file changes and emits Tauri events
/// so the frontend can auto-restart affected services.
pub struct FileWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watching: AtomicBool,
    watched_path: Mutex<Option<PathBuf>>,
}

/// Directories always skipped when watching.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    ".venv",
    "__pycache__",
    ".next",
    "dist",
    "build",
    ".nuxt",
    ".output",
    "coverage",
];

/// File extensions considered "source" (trigger restart).
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "svelte", "vue", "html", "css", "scss", "rs", "go", "py", "java",
    "kt", "rb", "cs", "swift", "toml", "yaml", "yml", "json",
];

impl FileWatcher {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
            watching: AtomicBool::new(false),
            watched_path: Mutex::new(None),
        }
    }

    /// Start watching `path`. Emits `devlauncher:file_changed` events via
    /// the Tauri app handle. Only the most recently changed file is reported
    /// (debounced internally by notify).
    pub fn start(&self, path: PathBuf, app: tauri::AppHandle) -> Result<(), String> {
        self.stop();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            let event = match res {
                Ok(e) => e,
                Err(_) => return,
            };
            // Only care about modify / create events (not access / remove).
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {}
                _ => return,
            }

            for path in event.paths {
                if !is_source_file(&path) {
                    continue;
                }
                let _ = app.emit(
                    "devlauncher:file_changed",
                    FileChangeEvent {
                        path: path.to_string_lossy().to_string(),
                    },
                );
                break; // one event per debounce window is enough
            }
        })
        .map_err(|e| format!("Failed to create file watcher: {}", e))?;

        watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch path {}: {}", path.display(), e))?;

        *self.watcher.lock().expect("watcher lock poisoned; critical section is infallible") =
            Some(watcher);
        *self
            .watched_path
            .lock()
            .expect("watched_path lock poisoned; critical section is infallible") =
            Some(path);
        self.watching.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stop watching and release resources.
    pub fn stop(&self) {
        *self.watcher.lock().expect("watcher lock poisoned; critical section is infallible") = None;
        *self
            .watched_path
            .lock()
            .expect("watched_path lock poisoned; critical section is infallible") =
            None;
        self.watching.store(false, Ordering::SeqCst);
    }

    pub fn is_watching(&self) -> bool {
        self.watching.load(Ordering::SeqCst)
    }

#[allow(dead_code)]
    pub fn watched_path(&self) -> Option<PathBuf> {
        self.watched_path
            .lock()
            .expect("watched_path lock poisoned; clone is infallible")
            .clone()
    }
}

#[derive(Clone, serde::Serialize)]
pub struct FileChangeEvent {
    pub path: String,
}

fn is_source_file(path: &Path) -> bool {
    // Skip hidden dirs and known non-source dirs
    if let Some(components) = path.components().next() {
        if let std::path::Component::Normal(name) = components {
            if let Some(s) = name.to_str() {
                if s.starts_with('.') || SKIP_DIRS.contains(&s) {
                    return false;
                }
            }
        }
    }

    // Check all path components for skip dirs
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if let Some(s) = name.to_str() {
                if SKIP_DIRS.contains(&s) {
                    return false;
                }
            }
        }
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    SOURCE_EXTENSIONS.contains(&ext.as_str())
}
