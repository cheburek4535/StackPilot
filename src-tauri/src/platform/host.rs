use std::fmt;

/// Typed representation of the host operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostOs {
    Windows,
    Linux,
    Macos,
}

/// Typed representation of the host CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostArch {
    X86,
    X86_64,
    Aarch64,
    Unknown,
}

/// Returns the operating system of the current host.
pub fn current_os() -> HostOs {
    match std::env::consts::OS {
        "windows" => HostOs::Windows,
        "linux" => HostOs::Linux,
        "macos" => HostOs::Macos,
        _ => HostOs::Linux,
    }
}

/// Returns the CPU architecture of the current host.
pub fn current_arch() -> HostArch {
    match std::env::consts::ARCH {
        "x86" => HostArch::X86,
        "x86_64" => HostArch::X86_64,
        "aarch64" => HostArch::Aarch64,
        _ => HostArch::Unknown,
    }
}

impl fmt::Display for HostOs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostOs::Windows => write!(f, "windows"),
            HostOs::Linux => write!(f, "linux"),
            HostOs::Macos => write!(f, "macos"),
        }
    }
}

impl fmt::Display for HostArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostArch::X86 => write!(f, "x86"),
            HostArch::X86_64 => write!(f, "x86_64"),
            HostArch::Aarch64 => write!(f, "aarch64"),
            HostArch::Unknown => write!(f, "unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_os_returns_known_variant() {
        let os = current_os();
        assert!(
            matches!(os, HostOs::Windows | HostOs::Linux | HostOs::Macos),
            "unexpected OS: {os}"
        );
    }

    #[test]
    fn current_arch_returns_known_variant() {
        let arch = current_arch();
        assert!(
            matches!(
                arch,
                HostArch::X86 | HostArch::X86_64 | HostArch::Aarch64 | HostArch::Unknown
            ),
            "unexpected arch: {arch}"
        );
    }

    #[test]
    fn os_display_matches_const() {
        assert_eq!(HostOs::Windows.to_string(), "windows");
        assert_eq!(HostOs::Linux.to_string(), "linux");
        assert_eq!(HostOs::Macos.to_string(), "macos");
    }

    #[test]
    fn arch_display_matches_const() {
        assert_eq!(HostArch::X86.to_string(), "x86");
        assert_eq!(HostArch::X86_64.to_string(), "x86_64");
        assert_eq!(HostArch::Aarch64.to_string(), "aarch64");
        assert_eq!(HostArch::Unknown.to_string(), "unknown");
    }

    #[test]
    fn current_os_matches_std_env_consts() {
        let os = current_os();
        let expected = match std::env::consts::OS {
            "windows" => HostOs::Windows,
            "linux" => HostOs::Linux,
            "macos" => HostOs::Macos,
            _ => HostOs::Linux,
        };
        assert_eq!(os, expected);
    }
}
