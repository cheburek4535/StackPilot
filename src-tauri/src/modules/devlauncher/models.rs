use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    RunCommand {
        command: String,
        working_dir: Option<String>,
        /// When true, the process is long-running and should be shown in a
        /// native terminal window. When false/absent, it is one-shot and its
        /// output is captured into the app's log viewer. `None` keeps the
        /// default (one-shot, piped).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persistent: Option<bool>,
    },
    OpenApplication {
        path: String,
        #[serde(default)]
        args: Option<String>,
        /// Structured argument list. When present, takes precedence over the
        /// legacy `args` string (which uses `split_whitespace` and cannot
        /// express quoted arguments). Front-end serialized profiles that lack
        /// this field deserialize as `None` via `#[serde(default)]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args_list: Option<Vec<String>>,
    },
    OpenUrl {
        url: String,
    },
    WaitForUrl {
        url: String,
        timeout_secs: u64,
    },
    WaitForPort {
        host: String,
        port: u16,
        timeout_secs: u64,
    },
    Delay {
        seconds: u64,
    },
    ExecuteScript {
        script: String,
        shell: Option<String>,
    },
}

impl ActionType {
    /// True if the action starts a long-running process that should remain
    /// tracked for its full lifetime (as opposed to a one-shot command that
    /// finishes and returns).
    pub fn is_persistent(&self) -> bool {
        matches!(
            self,
            ActionType::RunCommand {
                persistent: Some(true),
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchAction {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub action_type: ActionType,
}

/// A user-preferred IDE to open the project in before running actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PreferredIde {
    Vscode,
    Pycharm,
    Goland,
    Idea,
    Webstorm,
    Xcode,
    VisualStudio,
    /// Custom executable path or command name.
    Custom(String),
}

impl PreferredIde {
    /// Returns the CLI executable name used to open a folder in the IDE.
    pub fn cli_name(&self) -> &str {
        match self {
            PreferredIde::Vscode => "code",
            PreferredIde::Pycharm => "pycharm",
            PreferredIde::Goland => "goland",
            PreferredIde::Idea => "idea",
            PreferredIde::Webstorm => "webstorm",
            PreferredIde::Xcode => "xed",
            PreferredIde::VisualStudio => "devenv",
            PreferredIde::Custom(name) => name,
        }
    }

    /// Human-readable label for the UI.
    pub fn label(&self) -> &str {
        match self {
            PreferredIde::Vscode => "VS Code",
            PreferredIde::Pycharm => "PyCharm",
            PreferredIde::Goland => "GoLand",
            PreferredIde::Idea => "IntelliJ IDEA",
            PreferredIde::Webstorm => "WebStorm",
            PreferredIde::Xcode => "Xcode",
            PreferredIde::VisualStudio => "Visual Studio",
            PreferredIde::Custom(name) => name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfile {
    pub name: String,
    pub description: String,
    pub project_path: Option<String>,
    pub actions: Vec<LaunchAction>,
    /// Optional environment binding ID. When set, the launch engine resolves
    /// the binding into an EnvironmentOverlay and applies it to RunCommand,
    /// ExecuteScript, and OpenApplication actions. Absent/None preserves
    /// old host-environment behavior exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_binding_id: Option<String>,
    /// Preferred IDE to auto-launch the project in before running actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide: Option<PreferredIde>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionStatus {
    Success { message: String },
    Failed { error: String },
    Skipped { reason: String },
}
