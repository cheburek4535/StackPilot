use std::path::{Path, PathBuf};

use super::host::HostOs;

/// Normalize a path for comparison: on Windows, forward slashes become
/// backslashes and comparison is case-insensitive. On Unix, paths are
/// compared literally.
pub fn paths_eq(a: &Path, b: &Path, os: HostOs) -> bool {
    match os {
        HostOs::Windows => {
            let a_s = a.to_string_lossy().replace('/', "\\").to_lowercase();
            let b_s = b.to_string_lossy().replace('/', "\\").to_lowercase();
            a_s == b_s
        }
        HostOs::Macos => {
            // Standard macOS filesystems (APFS and HFS+) are case-insensitive by default.
            let a_s = a.to_string_lossy().replace('\\', "/").to_lowercase();
            let b_s = b.to_string_lossy().replace('\\', "/").to_lowercase();
            a_s == b_s
        }
        HostOs::Linux => a == b,
    }
}

/// Check if `child` is a descendant of `ancestor` using platform-aware
/// comparison.
pub fn is_descendant(child: &Path, ancestor: &Path, os: HostOs) -> bool {
    match os {
        HostOs::Windows => {
            let child_s = child.to_string_lossy().replace('/', "\\").to_lowercase();
            let ancestor_s = ancestor.to_string_lossy().replace('/', "\\").to_lowercase();
            // Ensure ancestor ends with separator for proper prefix check
            let ancestor_with_sep = if ancestor_s.ends_with('\\') {
                ancestor_s.clone()
            } else {
                format!("{ancestor_s}\\")
            };
            child_s.starts_with(&ancestor_with_sep) || child_s == ancestor_s
        }
        HostOs::Macos => {
            let child_s = child.to_string_lossy().replace('\\', "/").to_lowercase();
            let ancestor_s = ancestor.to_string_lossy().replace('\\', "/").to_lowercase();
            let ancestor_with_sep = if ancestor_s.ends_with('/') {
                ancestor_s.clone()
            } else {
                format!("{ancestor_s}/")
            };
            child_s.starts_with(&ancestor_with_sep) || child_s == ancestor_s
        }
        HostOs::Linux => child.starts_with(ancestor),
    }
}

/// Resolve an executable name to an absolute path using the `which` crate.
///
/// Returns `None` if the executable is not found in PATH.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

/// Returns the executable suffix for the given OS (e.g., `.exe` on Windows).
pub fn executable_suffix(os: HostOs) -> &'static str {
    match os {
        HostOs::Windows => ".exe",
        HostOs::Linux | HostOs::Macos => "",
    }
}

/// Append the platform-appropriate executable suffix if the name does not
/// already end with one.
pub fn ensure_executable_suffix(name: &str, os: HostOs) -> String {
    let suffix = executable_suffix(os);
    if suffix.is_empty() || name.ends_with(suffix) {
        name.to_string()
    } else {
        format!("{name}{suffix}")
    }
}

/// Check if a path looks like it points to a batch file (`.cmd` or `.bat`).
pub fn is_batch_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

/// Windows app-execution-alias directories (Microsoft Store stubs).
///
/// The Store aliases (`%LOCALAPPDATA%\Microsoft\WindowsApps` and
/// `%ProgramFiles%\WindowsApps`) contain reparse-point `.exe` stubs
/// (`python.exe`, `python3.exe`, ...) that are *not* real executables:
/// when launched they open the Microsoft Store (or fail with exit code
/// 9009 and "Python was not found" in non-interactive sessions).
///
/// A binary that resolves into one of these directories must NOT be
/// treated as an installation of the tool — detection code uses this to
/// distinguish a genuine broken install (PathBroken) from a fake stub
/// that merely shadows the tool name in PATH (Missing).
pub fn is_windows_store_alias(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(local).join("Microsoft").join("WindowsApps"));
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            dirs.push(std::path::PathBuf::from(program_files).join("WindowsApps"));
        }
        let norm = |p: &std::path::Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        let target = norm(path);
        dirs.iter().any(|dir| {
            let ancestor = norm(dir);
            target.starts_with(&ancestor)
                && (target.len() == ancestor.len() || target[ancestor.len()..].starts_with('\\'))
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

/// Linux snap-заглушка (`/snap/bin/<tool>` — симлинк на `/usr/bin/snap`).
///
/// Snap создаёт в `/snap/bin` врапперы даже для сломанных/частично
/// удалённых snap-пакетов: запуск такого файла не запускает инструмент,
/// а пытается доустановить snap (в GUI-сессии падает с «sudo: A terminal
/// is required to authenticate» и кодом 255). Это НЕ установка
/// инструмента: считать молчащую заглушку «сломанным PATH» — ложь
/// (пример: `/snap/bin/dotnet` при рабочем `~/.dotnet/dotnet`).
///
/// Заглушка определяется по каталогу `/snap/bin` и по симлинку на
/// `snap`/`snapd` (на случай нестандартной установки snapd).
pub fn is_snap_stub(path: &std::path::Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        if path.starts_with("/snap/bin") {
            return true;
        }
        if let Ok(target) = std::fs::read_link(path) {
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == "snap" || name == "snapd" {
                return true;
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

/// macOS Xcode Command Line Tools shims (`/usr/bin/git`, `/usr/bin/python3`, etc.).
///
/// On macOS, Apple installs stub binaries in `/usr/bin/` for developer tools
/// (`git`, `python3`, `clang`, `gcc`, `make`, etc.). When Xcode / Command Line Tools
/// are not installed, invoking these stubs displays a macOS GUI dialog prompting
/// the user to install them, and fails in non-interactive/CLI execution.
///
/// If developer tools are not active (e.g. `xcode-select -p` fails), any binary
/// located in `/usr/bin` that corresponds to an Apple developer shim is NOT
/// an active installation of the tool.
pub fn is_macos_clt_stub(path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        if !path.starts_with("/usr/bin") {
            return false;
        }
        let Some(file_name) = path.file_name().and_then(|f| f.to_str()) else {
            return false;
        };
        const CLT_SHIMS: &[&str] = &[
            "git", "python3", "python", "clang", "clang++", "gcc", "g++",
            "make", "lldb", "svn", "ar", "as", "nm", "ranlib", "size", "strings", "strip",
        ];
        if !CLT_SHIMS.contains(&file_name) {
            return false;
        }
        static CLT_ACTIVE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0); // 0=untested, 1=active, 2=inactive
        let status = CLT_ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
        if status == 1 {
            return false;
        } else if status == 2 {
            return true;
        }

        let is_active = std::process::Command::new("/usr/bin/xcode-select")
            .arg("-p")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);

        CLT_ACTIVE.store(if is_active { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
        !is_active
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

/// Register `stkpil` CLI across platforms:
/// - Windows: Adds `stkpil.exe` to `App Paths` and user `PATH` environment variable.
/// - Unix (Linux/macOS): Symlinks `stkpil` into `~/.local/bin/stkpil` (or `/usr/local/bin`).
pub fn register_stkpil_in_app_paths() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
        use winreg::RegKey;

        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                let mut stkpil_path = dir.join("stkpil.exe");
                if !stkpil_path.exists() {
                    let in_resources = dir.join("resources").join("stkpil.exe");
                    if in_resources.exists() {
                        stkpil_path = in_resources;
                    }
                }

                if stkpil_path.exists() {
                    let stkpil_dir = stkpil_path.parent().unwrap_or(dir);
                    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

                    // 1. App Paths registration (works in cmd, Win+R, ShellExecute)
                    if let Ok((key, _)) = hkcu.create_subkey(
                        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\stkpil.exe",
                    ) {
                        let _ = key.set_value("", &stkpil_path.to_string_lossy().to_string());
                        let _ = key.set_value("Path", &stkpil_dir.to_string_lossy().to_string());
                    }

                    // 2. User Environment PATH (works in PowerShell, Git Bash, VS Code terminal)
                    if let Ok(env_key) = hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE) {
                        let current_path: String = env_key.get_value("Path").unwrap_or_default();
                        let dir_str = stkpil_dir.to_string_lossy().to_string();
                        let already_present = current_path
                            .split(';')
                            .any(|p| p.trim().eq_ignore_ascii_case(&dir_str));

                        if !already_present {
                            let new_path = if current_path.is_empty() {
                                dir_str
                            } else if current_path.ends_with(';') {
                                format!("{}{}", current_path, dir_str)
                            } else {
                                format!("{};{}", current_path, dir_str)
                            };
                            let _ = env_key.set_value("Path", &new_path);
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                let candidates = [
                    dir.join("stkpil"),
                    dir.join("resources").join("stkpil"),
                    dir.join("../Resources").join("stkpil"),
                    dir.join("../MacOS").join("stkpil"),
                ];

                let found = candidates.into_iter().find(|p| p.exists());

                if let Some(stkpil_binary) = found {
                    if let Ok(home) = std::env::var("HOME") {
                        let local_bin = PathBuf::from(home).join(".local").join("bin");
                        let _ = std::fs::create_dir_all(&local_bin);
                        let symlink_path = local_bin.join("stkpil");

                        // Remove existing dead/outdated symlink if present
                        if symlink_path.is_symlink() || symlink_path.exists() {
                            let _ = std::fs::remove_file(&symlink_path);
                        }

                        #[cfg(unix)]
                        {
                            let _ = std::os::unix::fs::symlink(&stkpil_binary, &symlink_path);

                            // Ensure executable permissions
                            use std::os::unix::fs::PermissionsExt;
                            if let Ok(metadata) = std::fs::metadata(&stkpil_binary) {
                                let mut perms = metadata.permissions();
                                perms.set_mode(0o755);
                                let _ = std::fs::set_permissions(&stkpil_binary, perms);
                            }
                        }
                    }

                    #[cfg(target_os = "macos")]
                    {
                        // On macOS, ~/.local/bin is often not in default PATH, while /usr/local/bin
                        // usually is. If /usr/local/bin is writable, symlink stkpil there too.
                        let usr_local_bin = std::path::PathBuf::from("/usr/local/bin");
                        if usr_local_bin.is_dir() {
                            let symlink_path = usr_local_bin.join("stkpil");
                            if symlink_path.is_symlink() || symlink_path.exists() {
                                let _ = std::fs::remove_file(&symlink_path);
                            }
                            let _ = std::os::unix::fs::symlink(&stkpil_binary, &symlink_path);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Returns the standard application data directory for StackPilot:
/// - Windows: `%APPDATA%\com.cheburek4535.stackpilot`
/// - macOS: `~/Library/Application Support/com.cheburek4535.stackpilot`
/// - Linux: `$XDG_DATA_HOME/com.cheburek4535.stackpilot` or `~/.local/share/com.cheburek4535.stackpilot`
pub fn get_app_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("com.cheburek4535.stackpilot");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("com.cheburek4535.stackpilot");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(data_home).join("com.cheburek4535.stackpilot");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("com.cheburek4535.stackpilot");
        }
    }
    std::env::temp_dir().join("com.cheburek4535.stackpilot")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_eq_windows_case_insensitive() {
        assert!(paths_eq(
            Path::new(r"C:\Users\test"),
            Path::new(r"C:\Users\test"),
            HostOs::Windows,
        ));
        assert!(paths_eq(
            Path::new(r"C:\Users\Test"),
            Path::new(r"C:\Users\test"),
            HostOs::Windows,
        ));
        assert!(paths_eq(
            Path::new(r"C:/Users/test"),
            Path::new(r"C:\Users\test"),
            HostOs::Windows,
        ));
    }

    #[test]
    fn paths_eq_unix_case_sensitive() {
        assert!(paths_eq(
            Path::new("/usr/bin"),
            Path::new("/usr/bin"),
            HostOs::Linux,
        ));
        assert!(!paths_eq(
            Path::new("/usr/Bin"),
            Path::new("/usr/bin"),
            HostOs::Linux,
        ));
    }

    #[test]
    fn is_descendant_windows() {
        assert!(is_descendant(
            Path::new(r"C:\Users\test\project\src"),
            Path::new(r"C:\Users\test\project"),
            HostOs::Windows,
        ));
        assert!(is_descendant(
            Path::new(r"C:\Users\test\project"),
            Path::new(r"C:\Users\test\project"),
            HostOs::Windows,
        ));
        assert!(!is_descendant(
            Path::new(r"C:\Other\project"),
            Path::new(r"C:\Users\test\project"),
            HostOs::Windows,
        ));
    }

    #[test]
    fn is_descendant_unix() {
        assert!(is_descendant(
            Path::new("/home/user/project/src"),
            Path::new("/home/user/project"),
            HostOs::Linux,
        ));
        assert!(is_descendant(
            Path::new("/home/user/project"),
            Path::new("/home/user/project"),
            HostOs::Linux,
        ));
        assert!(!is_descendant(
            Path::new("/other/project"),
            Path::new("/home/user/project"),
            HostOs::Linux,
        ));
    }

    #[test]
    fn resolve_executable_returns_some_for_common() {
        // On Windows, cmd.exe is always available; on Unix, sh is always available.
        let name = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "sh"
        };
        let result = resolve_executable(name);
        assert!(result.is_some(), "{name} should be resolvable");
        let path = result.unwrap();
        assert!(path.exists(), "resolved path should exist: {path:?}");
    }

    #[test]
    fn resolve_executable_returns_none_for_nonexistent() {
        let result = resolve_executable("this_program_definitely_does_not_exist_xyz_98765");
        assert!(result.is_none());
    }

    #[test]
    fn executable_suffix_platform() {
        let os = crate::platform::host::current_os();
        match os {
            HostOs::Windows => assert_eq!(executable_suffix(HostOs::Windows), ".exe"),
            HostOs::Linux | HostOs::Macos => assert_eq!(executable_suffix(HostOs::Linux), ""),
        }
    }

    #[test]
    fn ensure_executable_suffix_no_double() {
        assert_eq!(
            ensure_executable_suffix("node.exe", HostOs::Windows),
            "node.exe"
        );
        assert_eq!(
            ensure_executable_suffix("node", HostOs::Windows),
            "node.exe"
        );
        assert_eq!(ensure_executable_suffix("node", HostOs::Linux), "node");
    }

    #[test]
    fn is_batch_file_detection() {
        assert!(is_batch_file("npx.cmd"));
        assert!(is_batch_file("script.bat"));
        assert!(is_batch_file("C:\\tools\\run.CMD"));
        assert!(!is_batch_file("node"));
        assert!(!is_batch_file("node.exe"));
        assert!(!is_batch_file(""));
    }

    /// Snap-заглушки (/snap/bin/*) — не установка инструмента:
    /// молчащая заглушка не даёт права на «PATH сломан».
    #[test]
    fn snap_stub_detection() {
        #[cfg(target_os = "linux")]
        {
            assert!(is_snap_stub(Path::new("/snap/bin/dotnet")));
            assert!(is_snap_stub(Path::new("/snap/bin/flutter")));
            assert!(!is_snap_stub(Path::new("/usr/bin/dotnet")));
            assert!(!is_snap_stub(Path::new("/home/user/.dotnet/dotnet")));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!is_snap_stub(Path::new("/snap/bin/dotnet")));
            assert!(!is_snap_stub(Path::new("C:\\tools\\dotnet.exe")));
        }
    }

    #[test]
    fn store_alias_detection_windows_paths() {
        // Реальные файлы не трогаем: проверяем чистое сравнение путей.
        #[cfg(target_os = "windows")]
        {
            let stub = PathBuf::from(format!(
                "{}\\Microsoft\\WindowsApps\\python.exe",
                std::env::var("LOCALAPPDATA").unwrap_or_default()
            ));
            assert!(
                is_windows_store_alias(&stub),
                "LOCALAPPDATA\\Microsoft\\WindowsApps\\python.exe — Store alias"
            );
            let prog = PathBuf::from(format!(
                "{}\\WindowsApps\\python3.exe",
                std::env::var("ProgramFiles").unwrap_or_default()
            ));
            assert!(
                is_windows_store_alias(&prog),
                "ProgramFiles\\WindowsApps\\python3.exe — Store alias"
            );
        }
        // Не-Windows пути и поддельные префиксы не распознаются.
        assert!(!is_windows_store_alias(Path::new(r"C:\Windows\System32\python.exe")));
        assert!(!is_windows_store_alias(Path::new(r"C:\WindowsApps\python.exe")));
        assert!(!is_windows_store_alias(Path::new(r"C:\Users\WindowsAppsX\python.exe")));
    }

    #[test]
    fn paths_eq_macos_case_insensitive() {
        assert!(paths_eq(
            Path::new("/Applications/Visual Studio Code.app"),
            Path::new("/applications/visual studio code.app"),
            HostOs::Macos,
        ));
        assert!(paths_eq(
            Path::new("/Users/Alex/Project"),
            Path::new("/users/alex/project"),
            HostOs::Macos,
        ));
    }

    #[test]
    fn is_descendant_macos() {
        assert!(is_descendant(
            Path::new("/Users/Alex/Project/src"),
            Path::new("/users/alex/project"),
            HostOs::Macos,
        ));
        assert!(is_descendant(
            Path::new("/Users/Alex/Project"),
            Path::new("/users/alex/project"),
            HostOs::Macos,
        ));
        assert!(!is_descendant(
            Path::new("/Users/Other/Project"),
            Path::new("/users/alex/project"),
            HostOs::Macos,
        ));
    }

    #[test]
    fn macos_clt_stub_non_macos_returns_false() {
        #[cfg(not(target_os = "macos"))]
        {
            assert!(!is_macos_clt_stub(Path::new("/usr/bin/git")));
            assert!(!is_macos_clt_stub(Path::new("/usr/bin/python3")));
        }
    }
}
