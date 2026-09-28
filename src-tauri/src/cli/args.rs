use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "stkpil",
    author = "cheburek4535",
    version = "1.2.5",
    about = "🚀 StackPilot CLI — Developer toolchain, project scaffold engine, and workspace manager",
    long_about = "StackPilot CLI gives you full terminal access to create projects with verified technology stacks, manage toolchains, and launch development environments."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new project interactively or with stack flags
    #[command(alias = "new", alias = "c")]
    Create(CreateArgs),

    /// List available project types, frameworks, and tools in the knowledge base
    #[command(alias = "ls")]
    List(ListArgs),

    /// Check installed developer tools and runtimes, and interactively install/update packages
    #[command(alias = "check", alias = "doc")]
    Doctor(DoctorArgs),

    /// Install one or more developer tools and runtimes
    #[command(alias = "i", alias = "add")]
    Install(InstallArgs),

    /// Update one or more developer tools or all outdated packages
    #[command(alias = "upgrade", alias = "up")]
    Update(UpdateArgs),

    /// Check status and versions of installed tools
    #[command(alias = "st", alias = "info")]
    Status(StatusArgs),

    /// Run and orchestrate development services for a project (analog of DevLauncher)
    #[command(alias = "dev", alias = "start", alias = "run")]
    Launch(LaunchArgs),

    /// Analyze a project repository and display detected stack, commands, and ports
    #[command(alias = "scan")]
    Analyze(AnalyzeArgs),

    /// Open a project directory in your configured editor (VS Code, Cursor, etc.)
    Open(OpenArgs),

    /// Print version information
    #[command(alias = "-version", alias = "--version", alias = "-v")]
    Version,
}

#[derive(Args, Debug, Clone)]
pub struct CreateArgs {
    /// Destination path for the project (defaults to current directory in interactive mode)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Project name (derived from directory name if not specified)
    #[arg(short = 'n', long = "name", value_name = "NAME")]
    pub name: Option<String>,

    /// Project type (e.g. fullstack, rest-api, frontend, desktop-app, cli-tool)
    #[arg(short = 'T', long = "type", value_name = "TYPE")]
    pub project_type: Option<String>,

    /// Backend framework (e.g. django, fastapi, actix, axum, nest, gin, express)
    #[arg(short = 'b', long = "backend", value_name = "FRAMEWORK")]
    pub backend: Option<String>,

    /// Frontend framework (e.g. react, vue, svelte, nextjs, astro)
    #[arg(short = 'f', long = "frontend", value_name = "FRAMEWORK")]
    pub frontend: Option<String>,

    /// Tools, databases, and infrastructure (e.g. postgresql, redis, airflow, terraform)
    #[arg(
        short = 'i',
        short_alias = 't',
        long = "infra",
        alias = "tools",
        value_name = "TOOL",
        num_args = 1..
    )]
    pub tools: Vec<String>,

    /// Specific programming language(s) (e.g. python, typescript, rust, go)
    #[arg(short = 'l', long = "lang", alias = "language", value_name = "LANG", num_args = 1..)]
    pub languages: Vec<String>,

    /// Open project in IDE after creation (default: "code" / VS Code if no editor is given)
    #[arg(short = 'o', long = "open", value_name = "IDE", num_args = 0..=1, default_missing_value = "code")]
    pub open: Option<String>,

    /// Initialize a git repository in the generated project
    #[arg(long = "git", default_value_t = true, action = clap::ArgAction::Set)]
    pub git: bool,

    /// Include Docker configuration and compose setup
    #[arg(long = "docker")]
    pub docker: bool,

    /// Preview recipe steps and file tree without writing anything to disk
    #[arg(long = "dry-run", alias = "preview")]
    pub dry_run: bool,

    /// Force non-interactive execution with defaults (answer Yes to all prompts)
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    /// Force interactive wizard mode even if some flags are specified
    #[arg(long = "interactive")]
    pub interactive: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter list to: types, frameworks, tools, or languages
    #[arg(value_name = "CATEGORY", default_value = "all")]
    pub category: String,
}

#[derive(Args, Debug)]
pub struct OpenArgs {
    /// Directory of the project to open (defaults to current directory)
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,

    /// Specific editor binary/command to use (defaults to "code" for VS Code)
    #[arg(short = 'e', long = "editor", default_value = "code")]
    pub editor: String,
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Automatically install or update missing and outdated tools without asking
    #[arg(long = "fix")]
    pub fix: bool,

    /// Filter tools by category (languages, package-managers, databases, devops, all)
    #[arg(short = 'c', long = "category", value_name = "CATEGORY", default_value = "all")]
    pub category: String,

    /// Non-interactive execution (answer Yes to all prompts)
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    /// Output diagnostic results as JSON
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InstallArgs {
    /// One or more tools/packages to install (e.g. python, gleam, docker, rust)
    #[arg(value_name = "TOOL", required = true, num_args = 1..)]
    pub tools: Vec<String>,

    /// Confirm installation automatically without prompting
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,

    /// Reinstall tool even if it is already installed
    #[arg(long = "reinstall", alias = "force")]
    pub force_reinstall: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Specific tools to update (if empty or --all, checks and updates all outdated tools)
    #[arg(value_name = "TOOL")]
    pub tools: Vec<String>,

    /// Update all installed tools with available updates
    #[arg(short = 'a', long = "all")]
    pub all: bool,

    /// Confirm update automatically without prompting
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StatusArgs {
    /// Specific tool to check (e.g. docker, python; omit to list summary of all tools)
    #[arg(value_name = "TOOL")]
    pub tool: Option<String>,

    /// Filter by category (e.g. language, package_manager, database, container, vcs)
    #[arg(short = 'c', long = "category", value_name = "CATEGORY")]
    pub category: Option<String>,

    /// Output status as JSON
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LaunchArgs {
    /// Path to the project to launch (defaults to current directory)
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,

    /// Specific saved profile name to run
    #[arg(short = 'p', long = "profile", value_name = "NAME")]
    pub profile: Option<String>,

    /// Launch all detected services without interactive selection
    #[arg(short = 'a', long = "all", short_alias = 'y', alias = "yes")]
    pub all: bool,

    /// Force interactive checklist to select which services to start
    #[arg(short = 's', long = "select")]
    pub select: bool,

    /// Do not automatically open browser URLs or Swagger pages
    #[arg(long = "no-open")]
    pub no_open: bool,

    /// Show what services and commands would be launched without running them
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AnalyzeArgs {
    /// Path to the project repository to analyze (defaults to current directory)
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,

    /// Output analysis details as JSON
    #[arg(long = "json")]
    pub json: bool,
}
