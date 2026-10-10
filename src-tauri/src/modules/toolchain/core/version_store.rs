// ============================================================
// Управление версиями инструментов и Shared Tool Store (version_store.rs)
// ============================================================
// Обеспечивает:
//   1. Обнаружение версий рантаймов (Node, Python, Go и др.) из:
//      - StackPilot Shared Store (%LOCALAPPDATA%/StackPilot/toolchains/<tool>/<version>)
//      - Внешних менеджеров версий на хосте (nvm, fnm, pyenv, uv, py launcher)
//      - Системного PATH и известных каталогов
//      - Каталога официальных портативных архивов для скачивания
//   2. Автономное скачивание и распаковку портативных архивов в Shared Store
//      (без прав администратора и без глобального загрязнения PATH)
//   3. Расчёт дедупликации и привязки версий к изолированным окружениям проектов
//   4. Очистку неиспользуемых версий (pruning)

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::modules::project_environment::models::EnvironmentBinding;
use crate::modules::toolchain::models::{
    BoundEnvironmentInfo, ToolEnvironmentUsage, ToolVersionCandidate, ToolVersionSourceKind,
    ToolchainDeduplicationReport,
};

/// Базовая директория для общего хранилища портативных версий StackPilot.
pub fn get_shared_toolchains_dir(custom_base: Option<&Path>) -> PathBuf {
    if let Some(base) = custom_base {
        return base.to_path_buf();
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data).join("StackPilot").join("toolchains");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("StackPilot")
                .join("toolchains");
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("stackpilot")
                .join("toolchains");
        }
    }

    std::env::temp_dir().join("stackpilot").join("toolchains")
}

/// Ищет все доступные версии инструмента (установленные в Shared Store,
/// обнаруженные во внешних менеджерах / системе, и доступные для скачивания).
pub fn list_available_versions(
    tool_id: &str,
    shared_store_dir: &Path,
) -> Vec<ToolVersionCandidate> {
    let normalized = tool_id.trim().to_lowercase();
    let mut candidates: Vec<ToolVersionCandidate> = Vec::new();
    let mut seen_versions: HashSet<String> = HashSet::new();

    // 1. Поиск в StackPilot Shared Store
    for c in scan_shared_store(&normalized, shared_store_dir) {
        if seen_versions.insert(c.version.clone()) {
            candidates.push(c);
        }
    }

    // 2. Поиск во внешних менеджерах версий (fnm, nvm, pyenv, uv, py launcher)
    for c in scan_external_version_managers(&normalized) {
        if seen_versions.insert(c.version.clone()) {
            candidates.push(c);
        }
    }

    // 3. Поиск глобальной системной версии в PATH
    if let Some(c) = scan_system_version(&normalized) {
        if !seen_versions.contains(&c.version) {
            seen_versions.insert(c.version.clone());
            candidates.push(c);
        }
    }

    // 4. Добавление официальных портативных сборок для скачивания (если ещё не установлены)
    for c in get_upstream_download_candidates(&normalized) {
        if !seen_versions.contains(&c.version) {
            candidates.push(c);
        }
    }

    candidates
}

/// Сканирует Shared Store StackPilot для конкретного инструмента.
fn scan_shared_store(tool_id: &str, shared_store_dir: &Path) -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let tool_dir = shared_store_dir.join(tool_id);
    if !tool_dir.is_dir() {
        return list;
    }

    let Ok(entries) = fs::read_dir(&tool_dir) else {
        return list;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let ver_name = entry.file_name().to_string_lossy().into_owned();
            if let Some(exe_path) = find_binary_in_tool_dir(tool_id, &path) {
                list.push(ToolVersionCandidate {
                    version: ver_name.clone(),
                    display_label: format!("{} (StackPilot Store)", ver_name),
                    source_kind: ToolVersionSourceKind::SharedStore,
                    source_name: "StackPilot Store".to_string(),
                    executable_path: Some(exe_path.to_string_lossy().into_owned()),
                    channel: Some("local".to_string()),
                    is_installed: true,
                    download_url: None,
                    size_mb: None,
                });
            }
        }
    }

    list
}

/// Ищет версии во внешних менеджерах на машине пользователя.
fn scan_external_version_managers(tool_id: &str) -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();

    match tool_id {
        "node" | "nodejs" => {
            // A. fnm
            list.extend(scan_fnm_versions());
            // B. nvm-windows
            list.extend(scan_nvm_windows_versions());
            // C. nvm (unix)
            list.extend(scan_nvm_unix_versions());
            // D. volta
            list.extend(scan_volta_versions());
        }
        "python" | "python3" => {
            // A. Windows Python Launcher (py -0p)
            #[cfg(target_os = "windows")]
            list.extend(scan_py_launcher_versions());
            // B. uv
            list.extend(scan_uv_python_versions());
            // C. pyenv-win / pyenv
            list.extend(scan_pyenv_versions());
            // D. Стандартные пути установки Python в Windows
            list.extend(scan_windows_standard_python_dirs());
        }
        "go" | "golang" => {
            // Go sdk installations (~/sdk/go*)
            list.extend(scan_go_sdk_versions());
        }
        _ => {}
    }

    list
}

/// Сканирует fnm каталоги
fn scan_fnm_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let mut candidate_dirs = Vec::new();

    if let Ok(appdata) = std::env::var("APPDATA") {
        candidate_dirs.push(PathBuf::from(appdata).join("fnm").join("node-versions"));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        candidate_dirs.push(PathBuf::from(&userprofile).join(".local").join("share").join("fnm").join("node-versions"));
        candidate_dirs.push(PathBuf::from(&userprofile).join(".fnm").join("node-versions"));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidate_dirs.push(PathBuf::from(&home).join(".local").join("share").join("fnm").join("node-versions"));
        candidate_dirs.push(PathBuf::from(&home).join(".fnm").join("node-versions"));
    }

    for base in candidate_dirs {
        if !base.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let ver = name.trim_start_matches('v').to_string();
                    let exe = if cfg!(target_os = "windows") {
                        p.join("installation").join("node.exe")
                    } else {
                        p.join("installation").join("bin").join("node")
                    };
                    let real_exe = if exe.is_file() {
                        exe
                    } else if cfg!(target_os = "windows") && p.join("node.exe").is_file() {
                        p.join("node.exe")
                    } else if p.join("bin").join("node").is_file() {
                        p.join("bin").join("node")
                    } else {
                        continue;
                    };

                    list.push(ToolVersionCandidate {
                        version: ver.clone(),
                        display_label: format!("v{} (fnm)", ver),
                        source_kind: ToolVersionSourceKind::ExternalManager,
                        source_name: "fnm".to_string(),
                        executable_path: Some(real_exe.to_string_lossy().into_owned()),
                        channel: None,
                        is_installed: true,
                        download_url: None,
                        size_mb: None,
                    });
                }
            }
        }
    }

    list
}

/// Сканирует nvm-windows
fn scan_nvm_windows_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let mut nvm_roots = Vec::new();

    if let Ok(nvm_home) = std::env::var("NVM_HOME") {
        nvm_roots.push(PathBuf::from(nvm_home));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        nvm_roots.push(PathBuf::from(appdata).join("nvm"));
    }
    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        nvm_roots.push(PathBuf::from(prog_files).join("nvm"));
    }

    for root in nvm_roots {
        if !root.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('v') || name.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                        let node_exe = p.join("node.exe");
                        if node_exe.is_file() {
                            let ver = name.trim_start_matches('v').to_string();
                            list.push(ToolVersionCandidate {
                                version: ver.clone(),
                                display_label: format!("v{} (nvm-windows)", ver),
                                source_kind: ToolVersionSourceKind::ExternalManager,
                                source_name: "nvm-windows".to_string(),
                                executable_path: Some(node_exe.to_string_lossy().into_owned()),
                                channel: None,
                                is_installed: true,
                                download_url: None,
                                size_mb: None,
                            });
                        }
                    }
                }
            }
        }
    }

    list
}

/// Сканирует nvm (Unix)
fn scan_nvm_unix_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let Ok(home) = std::env::var("HOME") else {
        return list;
    };
    let nvm_ver_dir = PathBuf::from(home).join(".nvm").join("versions").join("node");
    if !nvm_ver_dir.is_dir() {
        return list;
    }

    if let Ok(entries) = fs::read_dir(&nvm_ver_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let node_bin = p.join("bin").join("node");
                if node_bin.is_file() {
                    let ver = name.trim_start_matches('v').to_string();
                    list.push(ToolVersionCandidate {
                        version: ver.clone(),
                        display_label: format!("v{} (nvm)", ver),
                        source_kind: ToolVersionSourceKind::ExternalManager,
                        source_name: "nvm".to_string(),
                        executable_path: Some(node_bin.to_string_lossy().into_owned()),
                        channel: None,
                        is_installed: true,
                        download_url: None,
                        size_mb: None,
                    });
                }
            }
        }
    }

    list
}

/// Сканирует volta
fn scan_volta_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.is_empty() {
        return list;
    }
    let volta_dir = PathBuf::from(home).join(".volta").join("tools").join("image").join("node");
    if !volta_dir.is_dir() {
        return list;
    }

    if let Ok(entries) = fs::read_dir(&volta_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let exe = if cfg!(target_os = "windows") {
                    p.join("node.exe")
                } else {
                    p.join("bin").join("node")
                };
                if exe.is_file() {
                    list.push(ToolVersionCandidate {
                        version: name.clone(),
                        display_label: format!("v{} (volta)", name),
                        source_kind: ToolVersionSourceKind::ExternalManager,
                        source_name: "volta".to_string(),
                        executable_path: Some(exe.to_string_lossy().into_owned()),
                        channel: None,
                        is_installed: true,
                        download_url: None,
                        size_mb: None,
                    });
                }
            }
        }
    }

    list
}

/// Windows `py -0p` сканер
#[cfg(target_os = "windows")]
fn scan_py_launcher_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let mut cmd = Command::new("py");
    cmd.arg("-0p");

    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                // Пример: " -V:3.12 * C:\Python312\python.exe" или " -V:3.11   C:\Python311\python.exe"
                if trimmed.starts_with("-V:") {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let ver_part = parts[0].trim_start_matches("-V:");
                        let exe_part = parts.last().unwrap_or(&"");
                        let p = Path::new(exe_part);
                        if p.is_file() {
                            list.push(ToolVersionCandidate {
                                version: ver_part.to_string(),
                                display_label: format!("Python {} (py launcher)", ver_part),
                                source_kind: ToolVersionSourceKind::ExternalManager,
                                source_name: "py launcher".to_string(),
                                executable_path: Some(p.to_string_lossy().into_owned()),
                                channel: None,
                                is_installed: true,
                                download_url: None,
                                size_mb: None,
                            });
                        }
                    }
                }
            }
        }
    }

    list
}

/// Сканирует uv (astral-sh) python установки
fn scan_uv_python_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let mut uv_dirs = Vec::new();

    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        uv_dirs.push(PathBuf::from(localappdata).join("uv").join("python"));
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        uv_dirs.push(PathBuf::from(&home).join(".local").join("share").join("uv").join("python"));
    }

    for dir in uv_dirs {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    // cpython-3.12.7-windows-x86_64-none
                    let exe = if cfg!(target_os = "windows") {
                        p.join("python.exe").is_file().then(|| p.join("python.exe"))
                            .or_else(|| p.join("install").join("python.exe").is_file().then(|| p.join("install").join("python.exe")))
                    } else {
                        p.join("bin").join("python3").is_file().then(|| p.join("bin").join("python3"))
                            .or_else(|| p.join("install").join("bin").join("python3").is_file().then(|| p.join("install").join("bin").join("python3")))
                    };

                    if let Some(exe_path) = exe {
                        let ver = extract_simple_version(&name);
                        list.push(ToolVersionCandidate {
                            version: ver.clone(),
                            display_label: format!("Python {} (uv)", ver),
                            source_kind: ToolVersionSourceKind::ExternalManager,
                            source_name: "uv".to_string(),
                            executable_path: Some(exe_path.to_string_lossy().into_owned()),
                            channel: None,
                            is_installed: true,
                            download_url: None,
                            size_mb: None,
                        });
                    }
                }
            }
        }
    }

    list
}

/// Сканирует pyenv
fn scan_pyenv_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let mut pyenv_dirs = Vec::new();

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        pyenv_dirs.push(PathBuf::from(&userprofile).join(".pyenv").join("pyenv-win").join("versions"));
        pyenv_dirs.push(PathBuf::from(&userprofile).join(".pyenv").join("versions"));
    }
    if let Ok(home) = std::env::var("HOME") {
        pyenv_dirs.push(PathBuf::from(&home).join(".pyenv").join("versions"));
    }

    for dir in pyenv_dirs {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let exe = if cfg!(target_os = "windows") {
                        p.join("python.exe")
                    } else {
                        p.join("bin").join("python")
                    };
                    if exe.is_file() {
                        list.push(ToolVersionCandidate {
                            version: name.clone(),
                            display_label: format!("Python {} (pyenv)", name),
                            source_kind: ToolVersionSourceKind::ExternalManager,
                            source_name: "pyenv".to_string(),
                            executable_path: Some(exe.to_string_lossy().into_owned()),
                            channel: None,
                            is_installed: true,
                            download_url: None,
                            size_mb: None,
                        });
                    }
                }
            }
        }
    }

    list
}

/// Сканирует стандартные каталоги Python на Windows
fn scan_windows_standard_python_dirs() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let mut bases = Vec::new();
        if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
            bases.push(PathBuf::from(localappdata).join("Programs").join("Python"));
        }
        bases.push(PathBuf::from("C:\\"));

        for base in bases {
            if !base.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.to_lowercase().starts_with("python3") && p.is_dir() {
                        let exe = p.join("python.exe");
                        if exe.is_file() {
                            let ver = extract_simple_version(&name);
                            list.push(ToolVersionCandidate {
                                version: ver.clone(),
                                display_label: format!("Python {} ({})", ver, name),
                                source_kind: ToolVersionSourceKind::ExternalManager,
                                source_name: "Local Python".to_string(),
                                executable_path: Some(exe.to_string_lossy().into_owned()),
                                channel: None,
                                is_installed: true,
                                download_url: None,
                                size_mb: None,
                            });
                        }
                    }
                }
            }
        }
    }
    list
}

/// Сканирует Go SDK (~/sdk/go*)
fn scan_go_sdk_versions() -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.is_empty() {
        return list;
    }
    let sdk_dir = PathBuf::from(home).join("sdk");
    if !sdk_dir.is_dir() {
        return list;
    }

    if let Ok(entries) = fs::read_dir(&sdk_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("go1.") && p.is_dir() {
                let exe = if cfg!(target_os = "windows") {
                    p.join("bin").join("go.exe")
                } else {
                    p.join("bin").join("go")
                };
                if exe.is_file() {
                    let ver = name.trim_start_matches("go").to_string();
                    list.push(ToolVersionCandidate {
                        version: ver.clone(),
                        display_label: format!("Go {} (go sdk)", ver),
                        source_kind: ToolVersionSourceKind::ExternalManager,
                        source_name: "go sdk".to_string(),
                        executable_path: Some(exe.to_string_lossy().into_owned()),
                        channel: None,
                        is_installed: true,
                        download_url: None,
                        size_mb: None,
                    });
                }
            }
        }
    }

    list
}

/// Сканирует глобальную версию из системного PATH
fn scan_system_version(tool_id: &str) -> Option<ToolVersionCandidate> {
    let binary_name = match tool_id {
        "node" | "nodejs" => "node",
        "python" | "python3" => {
            if cfg!(target_os = "windows") {
                "python"
            } else {
                "python3"
            }
        }
        "go" | "golang" => "go",
        "rust" | "cargo" => "cargo",
        _ => tool_id,
    };

    let exe_path = crate::platform::command_resolver::resolve_executable(binary_name, None)?;
    let mut cmd = Command::new(&exe_path);
    cmd.arg(if tool_id == "go" || tool_id == "golang" { "version" } else { "--version" });
    cmd.stdin(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let ver = extract_simple_version(&text);
    if ver.is_empty() {
        return None;
    }

    Some(ToolVersionCandidate {
        version: ver.clone(),
        display_label: format!("v{} (Системная ОС)", ver),
        source_kind: ToolVersionSourceKind::System,
        source_name: "Системная ОС".to_string(),
        executable_path: Some(exe_path.to_string_lossy().into_owned()),
        channel: Some("system".to_string()),
        is_installed: true,
        download_url: None,
        size_mb: None,
    })
}

/// Официальные портативные версии, доступные для автономного скачивания
fn get_upstream_download_candidates(tool_id: &str) -> Vec<ToolVersionCandidate> {
    let mut list = Vec::new();

    match tool_id {
        "node" | "nodejs" => {
            let os_name = if cfg!(target_os = "windows") {
                "win-x64.zip"
            } else if cfg!(target_os = "macos") {
                "darwin-x64.tar.gz"
            } else {
                "linux-x64.tar.gz"
            };

            list.push(ToolVersionCandidate {
                version: "22.11.0".to_string(),
                display_label: "Node.js 22.11.0 (Current)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Node.js Dist (Portable)".to_string(),
                executable_path: None,
                channel: Some("current".to_string()),
                is_installed: false,
                download_url: Some(format!("https://nodejs.org/dist/v22.11.0/node-v22.11.0-{}", os_name)),
                size_mb: Some(35),
            });
            list.push(ToolVersionCandidate {
                version: "20.18.0".to_string(),
                display_label: "Node.js 20.18.0 (LTS Рекомендуется)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Node.js Dist (Portable)".to_string(),
                executable_path: None,
                channel: Some("lts".to_string()),
                is_installed: false,
                download_url: Some(format!("https://nodejs.org/dist/v20.18.0/node-v20.18.0-{}", os_name)),
                size_mb: Some(32),
            });
            list.push(ToolVersionCandidate {
                version: "18.20.4".to_string(),
                display_label: "Node.js 18.20.4 (Maintenance)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Node.js Dist (Portable)".to_string(),
                executable_path: None,
                channel: Some("lts".to_string()),
                is_installed: false,
                download_url: Some(format!("https://nodejs.org/dist/v18.20.4/node-v18.20.4-{}", os_name)),
                size_mb: Some(30),
            });
        }
        "python" | "python3" => {
            // Python standalone builds (astral-sh / indygreg standalone)
            let (py_os, ext) = if cfg!(target_os = "windows") {
                ("x86_64-pc-windows-msvc-shared-install_only", "tar.gz")
            } else if cfg!(target_os = "macos") {
                ("aarch64-apple-darwin-install_only", "tar.gz")
            } else {
                ("x86_64-unknown-linux-gnu-install_only", "tar.gz")
            };

            list.push(ToolVersionCandidate {
                version: "3.12.7".to_string(),
                display_label: "Python 3.12.7 (Рекомендуется)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Python Standalone (Portable)".to_string(),
                executable_path: None,
                channel: Some("stable".to_string()),
                is_installed: false,
                download_url: Some(format!(
                    "https://github.com/astral-sh/python-build-standalone/releases/download/20241002/cpython-3.12.7%2B20241002-{}.{}",
                    py_os, ext
                )),
                size_mb: Some(38),
            });
            list.push(ToolVersionCandidate {
                version: "3.11.9".to_string(),
                display_label: "Python 3.11.9".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Python Standalone (Portable)".to_string(),
                executable_path: None,
                channel: Some("stable".to_string()),
                is_installed: false,
                download_url: Some(format!(
                    "https://github.com/astral-sh/python-build-standalone/releases/download/20241002/cpython-3.11.9%2B20241002-{}.{}",
                    py_os, ext
                )),
                size_mb: Some(36),
            });
        }
        "go" | "golang" => {
            let (go_os, ext) = if cfg!(target_os = "windows") {
                ("windows-amd64", "zip")
            } else if cfg!(target_os = "macos") {
                ("darwin-amd64", "tar.gz")
            } else {
                ("linux-amd64", "tar.gz")
            };

            list.push(ToolVersionCandidate {
                version: "1.23.2".to_string(),
                display_label: "Go 1.23.2 (Latest)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Go Official Dist".to_string(),
                executable_path: None,
                channel: Some("stable".to_string()),
                is_installed: false,
                download_url: Some(format!("https://go.dev/dl/go1.23.2.{}.{}", go_os, ext)),
                size_mb: Some(65),
            });
            list.push(ToolVersionCandidate {
                version: "1.22.8".to_string(),
                display_label: "Go 1.22.8 (LTS)".to_string(),
                source_kind: ToolVersionSourceKind::UpstreamDownload,
                source_name: "Go Official Dist".to_string(),
                executable_path: None,
                channel: Some("lts".to_string()),
                is_installed: false,
                download_url: Some(format!("https://go.dev/dl/go1.22.8.{}.{}", go_os, ext)),
                size_mb: Some(63),
            });
        }
        _ => {}
    }

    list
}

/// Скачивает и распаковывает портативную версию инструмента в Shared Store.
pub async fn download_and_install_to_shared_store(
    tool_id: &str,
    version: &str,
    download_url: &str,
    shared_store_dir: &Path,
) -> Result<String, String> {
    let normalized = tool_id.trim().to_lowercase();
    let target_dir = shared_store_dir.join(&normalized).join(version);

    if target_dir.is_dir() {
        if let Some(existing_exe) = find_binary_in_tool_dir(&normalized, &target_dir) {
            return Ok(existing_exe.to_string_lossy().into_owned());
        }
    }

    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Не удалось создать папку хранилища {}: {}", target_dir.display(), e))?;

    // 1. Скачивание во временный файл через curl
    let ext = if download_url.ends_with(".zip") {
        "zip"
    } else if download_url.ends_with(".tar.gz") || download_url.ends_with(".tgz") {
        "tar.gz"
    } else {
        "archive"
    };

    let temp_archive = std::env::temp_dir().join(format!(
        "sp_dl_{}_{}_{}.{}",
        normalized,
        version,
        std::process::id(),
        ext
    ));

    log::info!("Загрузка портативной версии {} {} из {}", normalized, version, download_url);

    let mut cmd = tokio::process::Command::new("curl");
    cmd.args([
        "-fSL",
        "--retry",
        "2",
        "-A",
        "StackPilot/1.4",
        "-o",
        temp_archive.to_string_lossy().as_ref(),
        "--",
        download_url,
    ]);

    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);

    let status = cmd
        .status()
        .await
        .map_err(|e| format!("Ошибка запуска curl для загрузки {}: {}", download_url, e))?;

    if !status.success() {
        let _ = fs::remove_file(&temp_archive);
        return Err(format!("curl завершился с ошибкой при загрузке {}", download_url));
    }

    // 2. Распаковка архива
    let extract_res = extract_archive_to_dir(&temp_archive, &target_dir).await;
    let _ = fs::remove_file(&temp_archive);
    extract_res?;

    // 3. Поиск и возврат исполняемого файла
    if let Some(exe) = find_binary_in_tool_dir(&normalized, &target_dir) {
        Ok(exe.to_string_lossy().into_owned())
    } else {
        Err(format!(
            "Распаковка завершена, но бинарник для {} не найден в {}",
            normalized,
            target_dir.display()
        ))
    }
}

/// Распаковывает архив в целевую директорию
async fn extract_archive_to_dir(archive_path: &Path, target_dir: &Path) -> Result<(), String> {
    let path_str = archive_path.to_string_lossy();

    #[cfg(target_os = "windows")]
    {
        if path_str.ends_with(".zip") {
            let mut ps_cmd = tokio::process::Command::new("powershell");
            ps_cmd.args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &format!(
                    "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                    archive_path.to_string_lossy(),
                    target_dir.to_string_lossy()
                ),
            ]);
            crate::platform::suppress_child_console_async(&mut ps_cmd);
            let status = ps_cmd.status().await.map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("Expand-Archive завершился с ошибкой"));
            }
            return Ok(());
        }
    }

    // tar для tar.gz или tar на всех платформах
    let mut tar_cmd = tokio::process::Command::new("tar");
    tar_cmd.args([
        "-xf",
        archive_path.to_string_lossy().as_ref(),
        "-C",
        target_dir.to_string_lossy().as_ref(),
    ]);

    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut tar_cmd);

    let status = tar_cmd.status().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("tar завершился с ошибкой при распаковке"));
    }

    Ok(())
}

/// Ищет основной исполняемый файл в распакованном каталоге
pub fn find_binary_in_tool_dir(tool_id: &str, dir: &Path) -> Option<PathBuf> {
    let fallback_exe = format!("{}.exe", tool_id);
    let candidates: Vec<&str> = match tool_id {
        "node" | "nodejs" => {
            if cfg!(target_os = "windows") {
                vec!["node.exe", "*/node.exe", "bin/node.exe"]
            } else {
                vec!["node", "bin/node", "*/bin/node"]
            }
        }
        "python" | "python3" => {
            if cfg!(target_os = "windows") {
                vec!["python.exe", "*/python.exe", "install/python.exe"]
            } else {
                vec!["python3", "bin/python3", "*/bin/python3", "install/bin/python3"]
            }
        }
        "go" | "golang" => {
            if cfg!(target_os = "windows") {
                vec!["bin/go.exe", "go/bin/go.exe", "*/bin/go.exe"]
            } else {
                vec!["bin/go", "go/bin/go", "*/bin/go"]
            }
        }
        _ => {
            if cfg!(target_os = "windows") {
                vec![fallback_exe.as_str(), "bin/*.exe"]
            } else {
                vec![tool_id, "bin/*"]
            }
        }
    };

    // 1. Прямая проверка в корне
    for cand in &candidates {
        if !cand.contains('*') {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    // 2. Проверка первого вложенного уровня каталогов (архивы часто имеют top-level папку node-v20-win/)
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let sub = entry.path();
            if sub.is_dir() {
                for cand in &candidates {
                    let cleaned = cand.trim_start_matches("*/");
                    let p = sub.join(cleaned);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }

    None
}

/// Формирует отчёт об использовании версий и дедупликации по всем окружениям
pub fn calculate_deduplication_report(
    shared_store_dir: &Path,
    environments: &[EnvironmentBinding],
) -> ToolchainDeduplicationReport {
    // Карта: (tool_id, version, executable_path) -> список окружений
    let mut usage_map: HashMap<(String, String, String), Vec<BoundEnvironmentInfo>> = HashMap::new();

    for env in environments {
        for (tool_id, tool_override) in &env.tool_overrides {
            if let Some(ref exe) = tool_override.executable_path {
                let ver = tool_override.version.clone().unwrap_or_else(|| "unknown".to_string());
                let key = (tool_id.clone(), ver, exe.clone());
                let info = BoundEnvironmentInfo {
                    binding_id: env.binding_id.clone(),
                    name: env.name.clone().unwrap_or_else(|| env.binding_id.clone()),
                    bound_projects: env.bound_projects.clone(),
                };
                usage_map.entry(key).or_default().push(info);
            }
        }
    }

    let shared_canonical = shared_store_dir.to_string_lossy().to_lowercase();
    let mut usages = Vec::new();
    let mut total_shared_versions = 0;
    let mut total_bindings_using_shared = 0;
    let mut total_disk_used_bytes = 0u64;
    let mut total_disk_saved_bytes = 0u64;

    for ((t_id, ver, exe), bound_envs) in usage_map {
        let is_in_shared = exe.to_lowercase().contains(&shared_canonical)
            || exe.to_lowercase().contains("stackpilot\\toolchains");

        let source_kind = if is_in_shared {
            ToolVersionSourceKind::SharedStore
        } else if exe.to_lowercase().contains(".nvm") || exe.to_lowercase().contains("fnm") || exe.to_lowercase().contains("pyenv") || exe.to_lowercase().contains("uv") {
            ToolVersionSourceKind::ExternalManager
        } else {
            ToolVersionSourceKind::System
        };

        // Рассчитываем размер папки версии на диске
        let size_bytes = if let Some(parent) = Path::new(&exe).parent() {
            calculate_dir_size(parent)
        } else {
            0
        };

        if is_in_shared {
            total_shared_versions += 1;
            total_bindings_using_shared += bound_envs.len();
            total_disk_used_bytes += size_bytes;

            // Если версия шерится N окружениями (N > 1), то сэкономлено (N - 1) * размер
            if bound_envs.len() > 1 {
                total_disk_saved_bytes += (bound_envs.len() as u64 - 1) * size_bytes;
            }
        }

        usages.push(ToolEnvironmentUsage {
            tool_id: t_id,
            version: ver,
            executable_path: exe,
            source_kind,
            bound_environments: bound_envs,
            size_bytes,
        });
    }

    ToolchainDeduplicationReport {
        total_shared_versions,
        total_bindings_using_shared,
        total_disk_used_bytes,
        total_disk_saved_bytes,
        usages,
    }
}

/// Удаляет из Shared Store версии, на которые не ссылается ни одно окружение
pub fn prune_unused_versions(
    shared_store_dir: &Path,
    environments: &[EnvironmentBinding],
) -> Result<Vec<String>, String> {
    if !shared_store_dir.is_dir() {
        return Ok(Vec::new());
    }

    // Собираем все используемые executable_path
    let mut used_paths: HashSet<String> = HashSet::new();
    for env in environments {
        for tool_override in env.tool_overrides.values() {
            if let Some(ref exe) = tool_override.executable_path {
                used_paths.insert(exe.to_lowercase());
            }
        }
    }

    let mut deleted = Vec::new();
    let Ok(tool_entries) = fs::read_dir(shared_store_dir) else {
        return Ok(deleted);
    };

    for tool_entry in tool_entries.flatten() {
        let tool_path = tool_entry.path();
        if !tool_path.is_dir() {
            continue;
        }
        let tool_name = tool_entry.file_name().to_string_lossy().into_owned();

        if let Ok(ver_entries) = fs::read_dir(&tool_path) {
            for ver_entry in ver_entries.flatten() {
                let ver_path = ver_entry.path();
                if !ver_path.is_dir() {
                    continue;
                }
                let ver_name = ver_entry.file_name().to_string_lossy().into_owned();

                // Проверяем, есть ли хотя бы один используемый файл внутри этой директории
                let is_used = used_paths.iter().any(|up| up.starts_with(&ver_path.to_string_lossy().to_lowercase()));

                if !is_used {
                    if let Ok(_) = fs::remove_dir_all(&ver_path) {
                        deleted.push(format!("{}/{}", tool_name, ver_name));
                    }
                }
            }
        }
    }

    Ok(deleted)
}

fn calculate_dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(meta) = p.metadata() {
                    total += meta.len();
                }
            } else if p.is_dir() {
                total += calculate_dir_size(&p);
            }
        }
    }
    total
}

fn extract_simple_version(raw: &str) -> String {
    let parts: Vec<&str> = raw
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|s| !s.is_empty())
        .collect();

    for part in parts {
        if part.contains('.') {
            return part.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_version_works() {
        assert_eq!(extract_simple_version("v20.18.0"), "20.18.0");
        assert_eq!(extract_simple_version("Python 3.12.7"), "3.12.7");
        assert_eq!(extract_simple_version("go version go1.23.2 windows/amd64"), "1.23.2");
    }

    #[test]
    fn upstream_candidates_exist_for_node_and_python() {
        let node_cands = get_upstream_download_candidates("node");
        assert!(!node_cands.is_empty());
        assert!(node_cands.iter().any(|c| c.version == "20.18.0"));

        let py_cands = get_upstream_download_candidates("python");
        assert!(!py_cands.is_empty());
        assert!(py_cands.iter().any(|c| c.version.starts_with("3.12")));
    }
}
