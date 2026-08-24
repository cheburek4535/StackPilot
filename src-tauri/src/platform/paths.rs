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
        HostOs::Linux | HostOs::Macos => a == b,
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
        HostOs::Linux | HostOs::Macos => child.starts_with(ancestor),
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
}
