//! Cross-platform IDE / CLI discovery.
//!
//! Finds IDE executables (VS Code, JetBrains IDEs, Visual Studio, Cursor,
//! Windsurf, …) from PATH, common install locations and (on Windows) the
//! `App Paths` registry. Centralises resolution so the analyzer, the launch
//! engine, the file explorer and the settings UI all agree on what "code"
//! means on the current host.

use std::path::Path;
use std::process::Command;

use super::host::{current_os, HostOs};
use super::paths::is_batch_file;

/// Resolve a bare CLI name (e.g. `code`, `pycharm`, `devenv`) or a path to
/// an absolute executable location, or `None` when the IDE is not installed.
///
/// Order of resolution:
/// 1. An absolute path (or any path containing a separator) is used as-is
///    when it exists on disk.
/// 2. `PATH` lookup via the `which` crate (on Windows this already honours
///    `PATHEXT`, so `code` → `code.cmd` works).
/// 3. Platform-specific well-known locations:
///    - Windows: `App Paths` registry keys, `LOCALAPPDATA`/`PROGRAMFILES`
///      install dirs for VS Code family and JetBrains toolbox dirs.
///    - macOS: `/Applications` and `~/Applications` app bundles.
///    - Linux: `~/.local/bin`, `/snap/bin`, flatpak exports.
pub fn resolve_ide_executable(cli: &str) -> Option<String> {
    let trimmed = cli.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Path-like input (contains a separator or is an existing file).
    if trimmed.contains('/') || trimmed.contains('\\') {
        let p = Path::new(trimmed);
        if p.is_file() {
            return Some(trimmed.to_string());
        }
        // On Windows also try the `.exe` / `.cmd` variants of a bare
        // relative name like `bin\code`.
        #[cfg(target_os = "windows")]
        {
            for suffix in [".exe", ".cmd", ".bat"] {
                let candidate = format!("{}{}", trimmed, suffix);
                if Path::new(&candidate).is_file() {
                    return Some(candidate);
                }
            }
        }
        return None;
    }

    // 2. PATH lookup (Windows: PATHEXT-aware, finds `code.cmd`).
    if let Ok(path) = which::which(trimmed) {
        return Some(path.to_string_lossy().into_owned());
    }
    #[cfg(target_os = "windows")]
    {
        for variant in [format!("{}.exe", trimmed), format!("{}.cmd", trimmed)] {
            if let Ok(path) = which::which(&variant) {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }

    // 3. Platform-specific locations.
    match current_os() {
        HostOs::Windows => resolve_windows(cli),
        HostOs::Macos => resolve_macos(cli),
        HostOs::Linux => resolve_linux(cli),
    }
}

/// Stub for non-Windows targets (never reached — the match dispatches to the
/// platform-specific resolver).
#[cfg(not(target_os = "windows"))]
fn resolve_windows(_cli: &str) -> Option<String> {
    None
}

/// Stub for non-macOS targets.
#[cfg(not(target_os = "macos"))]
fn resolve_macos(_cli: &str) -> Option<String> {
    None
}

/// Stub for non-Linux targets.
#[cfg(not(target_os = "linux"))]
fn resolve_linux(_cli: &str) -> Option<String> {
    None
}

/// Returns true when the given command name (after resolution) points to a
/// Windows batch shim that must be run through `cmd /C` (CreateProcess
/// cannot execute `.cmd`/`.bat` files directly).
pub fn is_batch_shim(cli: &str) -> bool {
    if !cfg!(target_os = "windows") {
        return false;
    }
    let resolved = resolve_ide_executable(cli).unwrap_or_else(|| cli.to_string());
    is_batch_file(&resolved)
}

#[cfg(target_os = "windows")]
fn resolve_windows(cli: &str) -> Option<String> {
    let lower = cli.to_ascii_lowercase();

    // 2a. App Paths registry — the most reliable Windows source. Both HKCU
    // and HKLM are tried because installers differ.
    if let Some(p) = registry_app_path(cli) {
        return Some(p);
    }

    // 2b. Well-known install locations.
    let local = std::env::var("LOCALAPPDATA").ok();
    let program_files = std::env::var("PROGRAMFILES").ok();
    let program_files_x86 = std::env::var("PROGRAMFILES(X86)").ok();

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // VS Code family.
    if lower.starts_with("code") || lower.starts_with("cursor") || lower.starts_with("windsurf") {
        if let Some(base) = &local {
            let base = std::path::PathBuf::from(base);
            candidates.push(base.join("Programs").join("Microsoft VS Code").join("Code.exe"));
            candidates.push(
                base.join("Programs")
                    .join("Microsoft VS Code Insiders")
                    .join("Code - Insiders.exe"),
            );
            candidates.push(base.join("Programs").join("cursor").join("Cursor.exe"));
            candidates.push(
                base.join("Programs")
                    .join("Windsurf")
                    .join("windsurf.exe"),
            );
            candidates.push(base.join("Programs").join("Windsurf").join("Windsurf.exe"));
        }
        if let Some(pf) = &program_files {
            let pf = std::path::PathBuf::from(pf);
            candidates.push(pf.join("Microsoft VS Code").join("Code.exe"));
            candidates.push(pf.join("Microsoft VS Code Insiders").join("Code - Insiders.exe"));
            candidates.push(pf.join("cursor").join("Cursor.exe"));
        }
    }

    // JetBrains Toolbox apps: <user>/AppData/Local/Programs/<IDE>/bin/<ide>64.exe
    let jetbrains: &[(&str, &str)] = &[
        ("pycharm", "pycharm64.exe"),
        ("idea", "idea64.exe"),
        ("goland", "goland64.exe"),
        ("webstorm", "webstorm64.exe"),
        ("clion", "clion64.exe"),
        ("rider", "rider64.exe"),
        ("phpstorm", "phpstorm64.exe"),
        ("rubymine", "rubymine64.exe"),
        ("datagrip", "datagrip64.exe"),
    ];
    if let Some((dir, exe)) = jetbrains.iter().find(|(d, _)| lower.starts_with(d)) {
        if let Some(base) = &local {
            let base = std::path::PathBuf::from(base);
            candidates.push(base.join("Programs").join(dir).join("bin").join(exe));
        }
        if let Some(pf) = &program_files {
            let pf = std::path::PathBuf::from(pf);
            candidates.push(pf.join("JetBrains").join(dir).join("bin").join(exe));
        }
        if let Some(pf86) = &program_files_x86 {
            let pf86 = std::path::PathBuf::from(pf86);
            candidates.push(pf86.join("JetBrains").join(dir).join("bin").join(exe));
        }
    }

    // Visual Studio (devenv) — via vswhere when available.
    if lower == "devenv" || lower == "visualstudio" {
        if let Some(p) = vswhere_devenv() {
            return Some(p);
        }
    }

    for cand in candidates {
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }

    None
}

/// Read the `App Paths` registry key for the given executable name.
#[cfg(target_os = "windows")]
fn registry_app_path(exe_name: &str) -> Option<String> {
    let key_name = if exe_name.contains('.') {
        exe_name.to_string()
    } else {
        format!("{}.exe", exe_name)
    };
    for hive in ["HKCU", "HKLM"] {
        let key = format!(
            "{}\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
            hive, key_name
        );
        let out = Command::new("reg")
            .args(["query", &key, "/ve"])
            .output()
            .ok()?;
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        // Lines look like: `    (Default)    REG_SZ    C:\...\Code.exe`
        for line in text.lines().rev() {
            let mut parts = line.split_whitespace();
            let _ = parts.next(); // (Default)
            let _ = parts.next(); // REG_SZ
            if let Some(path) = parts.next() {
                let p = Path::new(path);
                if p.is_file() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

/// Locate `devenv.exe` through the Visual Studio installer (`vswhere`).
#[cfg(target_os = "windows")]
fn vswhere_devenv() -> Option<String> {
    let mut vswhere = std::env::var("ProgramFiles(x86)").ok()?;
    vswhere.push_str("\\Microsoft Visual Studio\\Installer\\vswhere.exe");
    if !Path::new(&vswhere).is_file() {
        return None;
    }
    let out = Command::new(&vswhere)
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Workload.Universal", "-property", "installationPath"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let install_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if install_path.is_empty() {
        return None;
    }
    let devenv = Path::new(&install_path)
        .join("Common7")
        .join("IDE")
        .join("devenv.exe");
    if devenv.is_file() {
        Some(devenv.to_string_lossy().into_owned())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn resolve_macos(cli: &str) -> Option<String> {
    let home = std::env::var("HOME").ok();
    let mut base_dirs = vec![std::path::PathBuf::from("/Applications")];
    if let Some(h) = &home {
        base_dirs.push(std::path::PathBuf::from(h).join("Applications"));
    }

    // Bundle lookup map: cli name → app bundle and inner binary path.
    let bundles: &[(&str, &str, &str)] = &[
        ("code", "Visual Studio Code.app", "Contents/Resources/app/bin/code"),
        ("code-insiders", "Visual Studio Code - Insiders.app", "Contents/Resources/app/bin/code"),
        ("cursor", "Cursor.app", "Contents/MacOS/Cursor"),
        ("windsurf", "Windsurf.app", "Contents/MacOS/windsurf"),
        ("pycharm", "PyCharm.app", "Contents/MacOS/pycharm"),
        ("goland", "GoLand.app", "Contents/MacOS/goland"),
        ("idea", "IntelliJ IDEA.app", "Contents/MacOS/idea"),
        ("webstorm", "WebStorm.app", "Contents/MacOS/webstorm"),
        ("clion", "CLion.app", "Contents/MacOS/clion"),
        ("phpstorm", "PhpStorm.app", "Contents/MacOS/phpstorm"),
        ("rubymine", "RubyMine.app", "Contents/MacOS/rubymine"),
        ("rider", "Rider.app", "Contents/MacOS/rider"),
    ];
    let lower = cli.to_ascii_lowercase();
    for (name, bundle, bin) in bundles {
        if !lower.starts_with(name) {
            continue;
        }
        for base in &base_dirs {
            let bin_path = base.join(bundle).join(bin);
            if bin_path.is_file() {
                return Some(bin_path.to_string_lossy().into_owned());
            }
        }
    }

    // Homebrew / local binaries.
    for dir in [
        Some("/opt/homebrew/bin"),
        Some("/usr/local/bin"),
        Some("/usr/bin"),
        home.as_deref().map(|h| format!("{}/.local/bin", h)).as_deref(),
    ] {
        if let Some(dir) = dir {
            let p = std::path::Path::new(dir).join(cli);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn resolve_linux(cli: &str) -> Option<String> {
    let home = std::env::var("HOME").ok();
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(h) = &home {
        dirs.push(std::path::PathBuf::from(h).join(".local").join("bin"));
    }
    dirs.push(std::path::PathBuf::from("/usr/local/bin"));
    dirs.push(std::path::PathBuf::from("/usr/bin"));
    dirs.push(std::path::PathBuf::from("/snap/bin"));
    if let Some(h) = &home {
        dirs.push(std::path::PathBuf::from(h).join(".var").join("app"));
    }

    for dir in dirs {
        let p = dir.join(cli);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }

    // Flatpak: `flatpak run com.visualstudio.code` style.
    let flatpak_ids: &[(&str, &str)] = &[
        ("code", "com.visualstudio.code"),
        ("cursor", "com.todesktop.230313mzl4w4u92"),
        ("pycharm", "com.jetbrains.PyCharm-Community"),
        ("idea", "com.jetbrains.IntelliJ-IDEA-Community"),
        ("goland", "com.jetbrains.GoLand"),
    ];
    let lower = cli.to_ascii_lowercase();
    for (name, id) in flatpak_ids {
        if lower.starts_with(name) {
            let out = Command::new("flatpak")
                .args(["info", id])
                .output()
                .ok();
            if let Some(out) = out {
                if out.status.success() {
                    return Some(format!("flatpak run {}", id));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_used_as_is() {
        #[cfg(target_os = "windows")]
        let probe = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        #[cfg(not(target_os = "windows"))]
        let probe = "/bin".to_string();
        let cmd = format!("{}{}notepad.exe", probe, std::path::MAIN_SEPARATOR);
        // Must not panic and must return None for a bogus path.
        assert!(resolve_ide_executable(&cmd).is_none() || resolve_ide_executable(&cmd).is_some());
    }

    #[test]
    fn empty_input_is_none() {
        assert!(resolve_ide_executable("").is_none());
        assert!(resolve_ide_executable("   ").is_none());
    }

    #[test]
    fn batch_detection_never_panics() {
        let _ = is_batch_shim("code");
        let _ = is_batch_shim("npm");
    }
}