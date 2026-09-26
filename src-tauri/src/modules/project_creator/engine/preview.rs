use crate::modules::project_creator::engine::build_dependencies;
use crate::modules::project_creator::models::*;
use std::collections::HashSet;

// ============================================================================
// Предпросмотр файловой структуры проекта
// ============================================================================

/// Одна известная запись в выходе внешнего инструмента.
///
/// Уровень достоверности назначается ЧЕСТНО:
///   - Expected — файл/каталог гарантированно появляется (стабильно у всех
///     версий инструмента), но содержимое зависит от версии CLI;
///   - Unknown — появление зависит от версии/шаблона/настроек CLI — не
///     обещаем, только показываем возможность.
struct ToolOutput {
    path: &'static str,
    is_dir: bool,
    certainty: FileCertainty,
    /// Дополнение к стандартному предупреждению (версионные оговорки).
    warning: Option<&'static str>,
}

const fn tool_out(
    path: &'static str,
    is_dir: bool,
    certainty: FileCertainty,
    warning: Option<&'static str>,
) -> ToolOutput {
    ToolOutput {
        path,
        is_dir,
        certainty,
        warning,
    }
}

/// NestJS CLI (`@nestjs/cli new`) — стабильный каркас с 2018 года.
const NEST_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("nest-cli.json", false, FileCertainty::Expected, None),
    tool_out("tsconfig.json", false, FileCertainty::Expected, None),
    tool_out("tsconfig.build.json", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("test", true, FileCertainty::Expected, None),
    tool_out(
        "eslint.config.mjs",
        false,
        FileCertainty::Expected,
        Some("May be .eslintrc.js in older CLI versions"),
    ),
    tool_out(
        ".prettierrc",
        false,
        FileCertainty::Expected,
        Some("Format may differ by CLI version"),
    ),
    tool_out(".gitignore", false, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// create-next-app (флаги рецепта: --app --tailwind --eslint --no-src-dir).
const NEXTJS_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("app", true, FileCertainty::Expected, None),
    tool_out("public", true, FileCertainty::Expected, None),
    tool_out("tsconfig.json", false, FileCertainty::Expected, None),
    tool_out("next-env.d.ts", false, FileCertainty::Expected, None),
    tool_out(
        "next.config.mjs",
        false,
        FileCertainty::Expected,
        Some("File name may be next.config.js in older versions"),
    ),
    tool_out(
        "eslint.config.mjs",
        false,
        FileCertainty::Expected,
        Some("Lint config format depends on the CLI version (may be .eslintrc.json)"),
    ),
    tool_out(
        "postcss.config.mjs",
        false,
        FileCertainty::Expected,
        Some("File name may be postcss.config.js in older versions"),
    ),
    tool_out(
        "tailwind.config.ts",
        false,
        FileCertainty::Unknown,
        Some("Only with Tailwind v3; v4+ configures Tailwind inside CSS"),
    ),
    tool_out(".gitignore", false, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// create-vite (react/vue/svelte шаблоны).
const VITE_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("index.html", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("public", true, FileCertainty::Expected, None),
    tool_out(
        "vite.config.ts",
        false,
        FileCertainty::Expected,
        Some("File name may be vite.config.js for JS templates"),
    ),
    tool_out("tsconfig.json", false, FileCertainty::Expected, None),
    tool_out(
        "tsconfig.app.json",
        false,
        FileCertainty::Expected,
        Some("Split tsconfig layout, Vite 5+"),
    ),
    tool_out(
        "tsconfig.node.json",
        false,
        FileCertainty::Expected,
        Some("Split tsconfig layout, Vite 5+"),
    ),
    tool_out(".gitignore", false, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// nuxi init (шаблон minimal).
const NUXT_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("nuxt.config.ts", false, FileCertainty::Expected, None),
    tool_out("app", true, FileCertainty::Expected, None),
    tool_out("public", true, FileCertainty::Expected, None),
    tool_out(
        "tsconfig.json",
        false,
        FileCertainty::Expected,
        Some("Nuxt 3+ is TypeScript-first"),
    ),
    tool_out(".gitignore", false, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// sv create (шаблон minimal).
const SVELTEKIT_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out(
        "svelte.config.js",
        false,
        FileCertainty::Expected,
        Some("May be .ts with TypeScript"),
    ),
    tool_out(
        "vite.config.js",
        false,
        FileCertainty::Expected,
        Some("May be .ts with TypeScript"),
    ),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("static", true, FileCertainty::Expected, None),
    tool_out(
        "tsconfig.json",
        false,
        FileCertainty::Expected,
        Some("Only when TypeScript is enabled"),
    ),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// create-solid (SolidStart v2, шаблон basic, --ts).
const SOLIDJS_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("public", true, FileCertainty::Expected, None),
    tool_out("tsconfig.json", false, FileCertainty::Expected, None),
    tool_out("vite.config.ts", false, FileCertainty::Expected, None),
    tool_out("app.config.ts", false, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// create-expo-app (шаблон с expo-router).
const EXPO_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("app", true, FileCertainty::Expected, None),
    tool_out("assets", true, FileCertainty::Expected, None),
    tool_out("app.json", false, FileCertainty::Expected, None),
    tool_out(
        "tsconfig.json",
        false,
        FileCertainty::Expected,
        Some("Only when TypeScript is enabled"),
    ),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// create-electron-app.
const ELECTRON_OUTPUTS: &[ToolOutput] = &[
    tool_out("package.json", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out(
        "forge.config.js",
        false,
        FileCertainty::Expected,
        Some("Config format may differ by create-electron-app version (forge.config.ts in newer versions)"),
    ),
    tool_out(
        "tsconfig.json",
        false,
        FileCertainty::Expected,
        Some("Only with TypeScript"),
    ),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// flutter create.
const FLUTTER_OUTPUTS: &[ToolOutput] = &[
    tool_out("pubspec.yaml", false, FileCertainty::Expected, None),
    tool_out("lib", true, FileCertainty::Expected, None),
    tool_out("test", true, FileCertainty::Expected, None),
    tool_out(
        "analysis_options.yaml",
        false,
        FileCertainty::Expected,
        None,
    ),
    tool_out("android", true, FileCertainty::Expected, None),
    tool_out("ios", true, FileCertainty::Expected, None),
    tool_out(
        "web",
        true,
        FileCertainty::Unknown,
        Some("Only for platforms enabled at creation"),
    ),
    tool_out(
        "windows",
        true,
        FileCertainty::Unknown,
        Some("Only for platforms enabled at creation"),
    ),
    tool_out(
        "linux",
        true,
        FileCertainty::Unknown,
        Some("Only for platforms enabled at creation"),
    ),
    tool_out(
        "macos",
        true,
        FileCertainty::Unknown,
        Some("Only for platforms enabled at creation"),
    ),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// django-admin startproject (внутренняя папка = имя проекта, добавляется
/// отдельно — см. build_project_file_preview).
const DJANGO_OUTPUTS: &[ToolOutput] =
    &[tool_out("manage.py", false, FileCertainty::Expected, None)];

/// composer create-project (laravel/symfony).
const COMPOSER_OUTPUTS: &[ToolOutput] = &[
    tool_out("composer.json", false, FileCertainty::Expected, None),
    tool_out("artisan", false, FileCertainty::Expected, None),
    tool_out("app", true, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("config", true, FileCertainty::Expected, None),
    tool_out("routes", true, FileCertainty::Expected, None),
    tool_out("public", true, FileCertainty::Expected, None),
    tool_out("resources", true, FileCertainty::Expected, None),
    tool_out("database", true, FileCertainty::Expected, None),
    tool_out(
        "README.md",
        false,
        FileCertainty::Expected,
        Some("Will be replaced by the StackPilot README.md at the project root"),
    ),
];

/// Spring Initializr (start.spring.io).
const SPRING_BOOT_OUTPUTS: &[ToolOutput] = &[
    tool_out("pom.xml", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out("mvnw", false, FileCertainty::Expected, None),
    tool_out("mvnw.cmd", false, FileCertainty::Expected, None),
    tool_out(".mvn", true, FileCertainty::Expected, None),
    tool_out("HELP.md", false, FileCertainty::Expected, None),
    tool_out(".gitignore", false, FileCertainty::Expected, None),
    tool_out(
        "target",
        true,
        FileCertainty::Unknown,
        Some("Appears after the first build (mvn package/test)"),
    ),
];

/// cargo init.
const CARGO_INIT_OUTPUTS: &[ToolOutput] = &[
    tool_out("Cargo.toml", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out(
        "Cargo.lock",
        false,
        FileCertainty::Unknown,
        Some("Created on the first build (cargo build)"),
    ),
    tool_out(
        ".gitignore",
        false,
        FileCertainty::Unknown,
        Some("Only when git is available (cargo init --vcs git)"),
    ),
    tool_out(
        ".git",
        true,
        FileCertainty::Unknown,
        Some("Only when git is available (cargo init --vcs git)"),
    ),
];

/// go mod init.
const GO_MOD_OUTPUTS: &[ToolOutput] = &[
    tool_out("go.mod", false, FileCertainty::Expected, None),
    tool_out(
        "go.sum",
        false,
        FileCertainty::Unknown,
        Some("Created on the first build or go mod tidy"),
    ),
];

/// mvn archetype:generate (maven-archetype-quickstart).
const MAVEN_OUTPUTS: &[ToolOutput] = &[
    tool_out("pom.xml", false, FileCertainty::Expected, None),
    tool_out("src", true, FileCertainty::Expected, None),
    tool_out(
        "target",
        true,
        FileCertainty::Unknown,
        Some("Appears after the first build"),
    ),
];

/// dotnet new console (имя .csproj = имя проекта, добавляется отдельно).
const DOTNET_OUTPUTS: &[ToolOutput] = &[
    tool_out("Program.cs", false, FileCertainty::Expected, None),
    tool_out(
        "obj",
        true,
        FileCertainty::Unknown,
        Some("Created on the first build"),
    ),
    tool_out(
        "bin",
        true,
        FileCertainty::Unknown,
        Some("Created on the first build"),
    ),
];

/// zig init.
const ZIG_OUTPUTS: &[ToolOutput] = &[
    tool_out("build.zig", false, FileCertainty::Expected, None),
    tool_out(
        "build.zig.zon",
        false,
        FileCertainty::Expected,
        Some("May be absent in older Zig versions"),
    ),
    tool_out("src", true, FileCertainty::Expected, None),
];

/// tauri init (@tauri-apps/cli или cargo tauri init).
const TAURI_OUTPUTS: &[ToolOutput] = &[
    tool_out("src-tauri", true, FileCertainty::Expected, None),
    tool_out(
        "src-tauri/tauri.conf.json",
        false,
        FileCertainty::Expected,
        None,
    ),
    tool_out("src-tauri/Cargo.toml", false, FileCertainty::Expected, None),
    tool_out("src-tauri/build.rs", false, FileCertainty::Expected, None),
    tool_out(
        "src-tauri/src/main.rs",
        false,
        FileCertainty::Expected,
        None,
    ),
    tool_out("src-tauri/icons", true, FileCertainty::Expected, None),
    tool_out(
        "src-tauri/capabilities",
        true,
        FileCertainty::Expected,
        Some("Created by newer tauri init versions"),
    ),
];

/// Известные стабильные выходы инструмента по команде/аргументам шага.
/// Возвращает (имя инструмента для UI, список известных выходов).
/// Возвращает None для инструментов без стабильной картины — для них
/// остаются expected_outputs и честный маркер «… other files».
fn tool_known_outputs(
    generator_id: &str,
    command: &str,
    args: &[String],
) -> Option<(&'static str, &'static [ToolOutput])> {
    let has = |needle: &str| args.iter().any(|a| a.contains(needle));
    if generator_id == "spring-boot" {
        return Some(("Spring Initializr (start.spring.io)", SPRING_BOOT_OUTPUTS));
    }
    match command {
        "npx" | "npm" => {
            if has("@nestjs/cli") {
                Some(("NestJS CLI (@nestjs/cli)", NEST_OUTPUTS))
            } else if has("create-next-app") {
                Some(("create-next-app", NEXTJS_OUTPUTS))
            } else if has("create-vite") {
                Some(("create-vite", VITE_OUTPUTS))
            } else if has("nuxi") {
                Some(("nuxi (Nuxt)", NUXT_OUTPUTS))
            } else if has("sv") && has("create") {
                Some(("sv create (SvelteKit)", SVELTEKIT_OUTPUTS))
            } else if has("create-solid") {
                Some(("create-solid", SOLIDJS_OUTPUTS))
            } else if has("create-expo-app") {
                Some(("create-expo-app", EXPO_OUTPUTS))
            } else if has("create-electron-app") {
                Some(("create-electron-app", ELECTRON_OUTPUTS))
            } else if has("@tauri-apps/cli") {
                Some(("tauri init (@tauri-apps/cli)", TAURI_OUTPUTS))
            } else {
                None
            }
        }
        "nuxi" => Some(("nuxi (Nuxt)", NUXT_OUTPUTS)),
        "sv" => Some(("sv create (SvelteKit)", SVELTEKIT_OUTPUTS)),
        "flutter" => Some(("flutter create", FLUTTER_OUTPUTS)),
        "django-admin" => Some(("django-admin startproject", DJANGO_OUTPUTS)),
        "composer" => Some(("composer create-project", COMPOSER_OUTPUTS)),
        "cargo" => {
            if has("tauri") {
                Some(("tauri init (cargo)", TAURI_OUTPUTS))
            } else if has("init") {
                Some(("cargo init", CARGO_INIT_OUTPUTS))
            } else {
                None
            }
        }
        "go" => Some(("go mod init", GO_MOD_OUTPUTS)),
        "mvn" => Some(("Maven archetype (mvn archetype:generate)", MAVEN_OUTPUTS)),
        "dotnet" => Some(("dotnet new", DOTNET_OUTPUTS)),
        "zig" => Some(("zig init", ZIG_OUTPUTS)),
        _ => None,
    }
}

/// Соединить относительный каталог и вложенный путь ("backend" + "src" → "backend/src").
fn join_path(base: &str, sub: &str) -> String {
    if base.is_empty() || base == "." {
        sub.to_string()
    } else if sub.is_empty() {
        base.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), sub)
    }
}

/// Относительный рабочий каталог шага от корня проекта ("." — корень).
/// Рабочие каталоги шагов бывают абсолютными (project_path + каталог),
/// относительными ("frontend") или пустыми — всё сводится к одному виду.
fn rel_workdir(plan: &ExecutionPlan, working_dir: Option<&str>) -> String {
    let Some(wd) = working_dir else {
        return ".".to_string();
    };
    if wd.is_empty() || wd == "." {
        return ".".to_string();
    }
    let norm = wd.replace('\\', "/");
    let root = plan.project_path.to_string_lossy().replace('\\', "/");
    if norm == root {
        return ".".to_string();
    }
    if let Some(rest) = norm.strip_prefix(&format!("{}/", root)) {
        return if rest.is_empty() {
            ".".to_string()
        } else {
            rest.to_string()
        };
    }
    // Относительный путь (некоторые шаги указывают working_dir относительно корня).
    wd.replace('\\', "/").trim_start_matches('/').to_string()
}

/// npm install в каталоге — гарантированный выход npm.
fn is_npm_install(command: &str, args: &[String]) -> bool {
    command == "npm" && args.iter().any(|a| a == "install")
}

/// Вставить известные выходы инструмента под каталог `base` + честный маркер
/// «… other files»: инструмент создаёт больше, чем можно предсказать.
fn insert_tool_outputs(
    entries: &mut Vec<FileEntry>,
    base: &str,
    tool: &str,
    outputs: &[ToolOutput],
    expected_count: &mut usize,
    unknown_count: &mut usize,
) {
    for output in outputs {
        let full = join_path(base, output.path);
        let prefix = if output.certainty == FileCertainty::Expected {
            format!("Created by {tool} — the exact content depends on the tool version.")
        } else {
            format!("Created by {tool} — may or may not appear (depends on version/template).")
        };
        let warning = match output.warning {
            Some(note) => format!("{prefix} {note}"),
            None => prefix,
        };
        match output.certainty {
            FileCertainty::Expected => insert_entry(
                entries,
                &full,
                output.is_dir,
                FileCertainty::Expected,
                tool.to_string(),
                None,
                Some(warning),
                expected_count,
            ),
            FileCertainty::Unknown => insert_entry(
                entries,
                &full,
                output.is_dir,
                FileCertainty::Unknown,
                tool.to_string(),
                None,
                Some(warning),
                unknown_count,
            ),
            // Выходы инструментов никогда не бывают Certain — наши файлы
            // (WriteFile) добавляются отдельным механизмом.
            FileCertainty::Certain => {}
        }
    }
    // Честный маркер неучтённого: список известен не полностью.
    let placeholder = format!("… other files from {tool}");
    insert_entry(
        entries,
        &join_path(base, &placeholder),
        false,
        FileCertainty::Unknown,
        tool.to_string(),
        None,
        Some(
            "The tool creates more files than this preview can list — the exact set is \
             not known until generation completes."
                .to_string(),
        ),
        unknown_count,
    );
}

/// Построить дерево файлов предпросмотра из плана выполнения.
/// Walk по шагам плана:
///   - WriteFile → Certain (наши файлы, содержимое точно известно);
///   - CreateDirectory → Certain (dir);
///   - Generate/Command → Expected (гарантированные выходы внешних
///     инструментов: expected_outputs, node_modules, известный стабильный
///     каркас) и Unknown (может появиться, но не обещаем).
pub fn build_project_file_preview(plan: &ExecutionPlan) -> ProjectFilePreview {
    let mut root_entries: Vec<FileEntry> = Vec::new();
    let mut removable_ids: Vec<String> = Vec::new();
    let mut certain_count = 0usize;
    let mut expected_count = 0usize;
    let mut unknown_count = 0usize;
    let mut dir_count = 0usize;

    for step in &plan.steps {
        match step {
            Step::CreateDirectory { id, path, .. } => {
                // Помечаем removable для git/vscode/readme шагов
                if is_removable_step(id) {
                    if !removable_ids.contains(id) {
                        removable_ids.push(id.clone());
                    }
                }
                let dir_name = path.trim_end_matches('/').to_string();
                if dir_name.is_empty() || dir_name == "." {
                    continue;
                }
                insert_entry(
                    &mut root_entries,
                    &dir_name,
                    true,
                    FileCertainty::Certain,
                    "StackPilot generator".into(),
                    None,
                    None,
                    &mut dir_count,
                );
            }
            Step::WriteFile {
                id, path, content, ..
            } => {
                if is_removable_step(id) {
                    if !removable_ids.contains(id) {
                        removable_ids.push(id.clone());
                    }
                }
                insert_entry(
                    &mut root_entries,
                    path,
                    false,
                    FileCertainty::Certain,
                    "StackPilot generator".into(),
                    Some(content.clone()),
                    None,
                    &mut certain_count,
                );
            }
            Step::Generate {
                id,
                generator_id,
                generator_config,
                ..
            } => {
                if is_removable_step(id) {
                    if !removable_ids.contains(id) {
                        removable_ids.push(id.clone());
                    }
                }
                // Извлекаем expected_outputs из конфига генератора
                let outputs = generator_config
                    .get("expected_outputs")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| o.as_str().map(String::from))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let target_dir = generator_config
                    .get("target_dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                let base = if target_dir == "." {
                    ".".to_string()
                } else {
                    target_dir.trim_end_matches('/').to_string()
                };
                let source_label = match generator_id.as_str() {
                    "scaffold" => {
                        let cmd = generator_config
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("CLI tool");
                        format!("{} (external CLI)", cmd)
                    }
                    "cli" => {
                        let cmd = generator_config
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("command");
                        format!("{} (CLI command)", cmd)
                    }
                    "python-venv" => "StackPilot (Python virtual environment)".to_string(),
                    "spring-boot" => "Spring Initializr (spring-boot)".to_string(),
                    "vscode-merge" => "StackPilot (VS Code config)".to_string(),
                    "vscode-folders" => "StackPilot (VS Code folders)".to_string(),
                    "tauri-config" => "StackPilot (Tauri config)".to_string(),
                    "fs-cleanup" => "StackPilot (cleanup)".to_string(),
                    "manifest-check" => "StackPilot (manifest validation)".to_string(),
                    "host-tool-check" => "StackPilot (host tool check)".to_string(),
                    other => format!("Generator: {}", other),
                };
                for output in &outputs {
                    let full_path = join_path(&base, output);
                    insert_entry(
                        &mut root_entries,
                        &full_path,
                        false,
                        FileCertainty::Expected,
                        source_label.clone(),
                        None,
                        Some(format!(
                            "Created by {source_label} — the exact content depends on the tool version."
                        )),
                        &mut expected_count,
                    );
                }
                // python-venv создаёт канонический venv (Unix: генератор
                // вместо `python -m venv`) — каталог обязан попасть в
                // предпросмотр файлов, как и у прежнего Command-шага.
                if generator_id == "python-venv" {
                    if let Some(venv_abs) = generator_config.get("venv_path").and_then(|v| v.as_str())
                    {
                        let venv_rel = rel_workdir(plan, Some(venv_abs));
                        insert_entry(
                            &mut root_entries,
                            &venv_rel,
                            true,
                            FileCertainty::Expected,
                            "python -m venv".into(),
                            None,
                            Some(
                                "Created by the project virtual environment generator — contains \
                                 the virtual environment"
                                    .into(),
                            ),
                            &mut expected_count,
                        );
                    }
                }
                // Известные стабильные выходы инструмента + маркер «… other files».
                let cmd = generator_config
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args: Vec<String> = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some((tool, tool_outputs)) = tool_known_outputs(generator_id, &cmd, &args) {
                    insert_tool_outputs(
                        &mut root_entries,
                        &base,
                        tool,
                        tool_outputs,
                        &mut expected_count,
                        &mut unknown_count,
                    );
                }
                // Для scaffold без expected_outputs — добавляем родительскую папку
                if outputs.is_empty() && generator_id == "scaffold" {
                    let dir = if base == "." {
                        plan.context
                            .project_name
                            .clone()
                            .unwrap_or_else(|| "project".into())
                    } else {
                        base.clone()
                    };
                    insert_entry(
                        &mut root_entries,
                        &dir,
                        true,
                        FileCertainty::Expected,
                        source_label.clone(),
                        None,
                        None,
                        &mut dir_count,
                    );
                }
            }
            Step::Command {
                id,
                command,
                args,
                working_dir,
                ..
            } => {
                if is_removable_step(id) {
                    if !removable_ids.contains(id) {
                        removable_ids.push(id.clone());
                    }
                }
                let wd = rel_workdir(plan, working_dir.as_deref());
                // npm install → node_modules/ + package-lock.json (Expected:
                // каталог гарантирует npm, содержимое не предсказуемо).
                if is_npm_install(command, args) {
                    insert_entry(
                        &mut root_entries,
                        &join_path(&wd, "node_modules"),
                        true,
                        FileCertainty::Expected,
                        "npm install".into(),
                        None,
                        Some(
                            "Created by npm install — contents are managed by npm and \
                             cannot be previewed"
                                .into(),
                        ),
                        &mut expected_count,
                    );
                    insert_entry(
                        &mut root_entries,
                        &join_path(&wd, "package-lock.json"),
                        false,
                        FileCertainty::Expected,
                        "npm install".into(),
                        None,
                        Some(
                            "Created by npm install — exact content depends on installed \
                             versions"
                                .into(),
                        ),
                        &mut expected_count,
                    );
                }
                // python -m venv <путь> → venv/ (Expected: создаём сами;
                // Unix-ветку генератора python-venv предпросмотр обрабатывает
                // отдельной веткой выше).
                if matches!(command.as_str(), "python" | "python3")
                    && args.windows(2).any(|w| w[0] == "-m" && w[1] == "venv")
                {
                    let venv_abs = args.last().cloned().unwrap_or_default();
                    let venv_rel = rel_workdir(plan, Some(&venv_abs));
                    insert_entry(
                        &mut root_entries,
                        &venv_rel,
                        true,
                        FileCertainty::Expected,
                        "python -m venv".into(),
                        None,
                        Some("Created by python -m venv — contains the virtual environment".into()),
                        &mut expected_count,
                    );
                }
                // git init → .git/ (Expected: git init гарантирует каталог).
                if command == "git" && args.first().map(|s| s.as_str()) == Some("init") {
                    insert_entry(
                        &mut root_entries,
                        ".git",
                        true,
                        FileCertainty::Expected,
                        "git init".into(),
                        None,
                        Some("Created by git init — repository metadata".into()),
                        &mut expected_count,
                    );
                }
                // cargo build → target/ (Unknown: только после сборки).
                if command == "cargo" && args.iter().any(|a| a == "build") {
                    insert_entry(
                        &mut root_entries,
                        &join_path(&wd, "target"),
                        true,
                        FileCertainty::Unknown,
                        "cargo build".into(),
                        None,
                        Some("Build output directory — appears after the first build".into()),
                        &mut unknown_count,
                    );
                }
                // go mod init → go.sum (Unknown: после первой сборки/tidy).
                if command == "go" && args.iter().any(|a| a == "mod") {
                    insert_entry(
                        &mut root_entries,
                        &join_path(&wd, "go.sum"),
                        false,
                        FileCertainty::Unknown,
                        "go mod tidy".into(),
                        None,
                        Some("Created on the first build or go mod tidy".into()),
                        &mut unknown_count,
                    );
                }
                // Известные стабильные выходы CLI-команд (nest, django…)
                // + маркер «… other files».
                if let Some((tool, tool_outputs)) = tool_known_outputs("", command, args) {
                    insert_tool_outputs(
                        &mut root_entries,
                        &wd,
                        tool,
                        tool_outputs,
                        &mut expected_count,
                        &mut unknown_count,
                    );
                    // django-admin startproject создаёт внутреннюю папку с именем
                    // проекта — знаем её точно (project_name).
                    if command == "django-admin" && args.iter().any(|a| a == "startproject") {
                        let pname = plan
                            .context
                            .project_name
                            .clone()
                            .unwrap_or_else(|| "project".into());
                        let inner = join_path(&wd, &pname);
                        let inner_src = "django-admin startproject".to_string();
                        let inner_warn = Some(
                            "Created by django-admin startproject — exact content depends \
                             on the Django version"
                                .to_string(),
                        );
                        insert_entry(
                            &mut root_entries,
                            &inner,
                            true,
                            FileCertainty::Expected,
                            inner_src.clone(),
                            None,
                            inner_warn.clone(),
                            &mut expected_count,
                        );
                        for inner_file in [
                            "__init__.py",
                            "settings.py",
                            "urls.py",
                            "asgi.py",
                            "wsgi.py",
                        ] {
                            insert_entry(
                                &mut root_entries,
                                &join_path(&inner, inner_file),
                                false,
                                FileCertainty::Expected,
                                inner_src.clone(),
                                None,
                                inner_warn.clone(),
                                &mut expected_count,
                            );
                        }
                    }
                    // dotnet new console -n <имя> → <имя>.csproj.
                    if command == "dotnet" {
                        let csproj_name = args
                            .windows(2)
                            .find(|w| w[0] == "-n")
                            .and_then(|w| w.get(1))
                            .cloned()
                            .unwrap_or_else(|| "app".to_string());
                        insert_entry(
                            &mut root_entries,
                            &join_path(&wd, &format!("{}.csproj", csproj_name)),
                            false,
                            FileCertainty::Expected,
                            "dotnet new".into(),
                            None,
                            Some(
                                "Created by dotnet new — exact content depends on the \
                                 .NET SDK version"
                                    .into(),
                            ),
                            &mut expected_count,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // Сортируем корневые элементы: сначала папки, потом файлы, по алфавиту
    sort_entries(&mut root_entries);

    ProjectFilePreview {
        files: root_entries,
        layout: plan.layout_summary.clone(),
        removable_step_ids: removable_ids,
        summary: ProjectPreviewSummary {
            certain_count,
            expected_count,
            unknown_count,
            dir_count,
        },
    }
}

/// Шаг является «удаляемым» (опциональным): git_*, vscode_*, readme, merge_inner_vscode.
fn is_removable_step(id: &str) -> bool {
    id.starts_with("git_")
        || id.starts_with("vscode_")
        || id == "readme"
        || id == "merge_inner_vscode"
        || id == "ci_workflow"
        || id == "github_dir"
}

/// Идентификаторы опциональных шагов плана (git_*, vscode_*, readme, ci_*),
/// которые пользователь может удалить без потери ядра проекта.
fn removable_step_ids_of(plan: &ExecutionPlan) -> Vec<String> {
    let mut ids = Vec::new();
    for step in &plan.steps {
        if is_removable_step(&step.id()) && !ids.contains(&step.id()) {
            ids.push(step.id());
        }
    }
    ids
}

/// Применить удаление опциональных шагов к плану: шаги исключаются из
/// выполнения, их файлы не создаются. Предпросмотр и генерация идут строго
/// по одной схеме — один и тот же отфильтрованный план.
///
/// Безопасность: удалить можно только шаги из `removable_step_ids_of`;
/// удалённый шаг не может быть предшественником другого шага (иначе
/// зависимые шаги остались бы без предусловия).
pub fn apply_step_removals(
    mut plan: ExecutionPlan,
    removed: &[String],
) -> Result<ExecutionPlan, String> {
    if removed.is_empty() {
        return Ok(plan);
    }
    let removable = removable_step_ids_of(&plan);
    let ids: HashSet<String> = plan.steps.iter().map(|s| s.id()).collect();
    for id in removed {
        // Шаг уже отсутствует в плане (фича выключена после удаления,
        // повторное удаление) — не ошибка, просто игнорируем.
        if !ids.contains(id) {
            continue;
        }
        if !removable.iter().any(|r| r == id) {
            return Err(format!(
                "Step '{}' is not optional and cannot be removed",
                id
            ));
        }
    }
    for dep in &plan.dependencies {
        if removed.iter().any(|r| r == &dep.prereq_id) {
            return Err(format!(
                "Step '{}' cannot be removed: step '{}' depends on it",
                dep.prereq_id, dep.step_id
            ));
        }
    }
    plan.steps.retain(|s| !removed.contains(&s.id()));
    plan.dependencies = build_dependencies(&plan.steps, &plan.recipe.dependencies);
    Ok(plan)
}

/// Вставить файл/директорию в дерево, создавая промежуточные папки.
/// `warning` — предупреждение, показываемое при открытии файла (Expected/
/// Unknown). Счётчик увеличивается ТОЛЬКО при создании новой записи —
/// повторная вставка того же пути (expected_outputs + known outputs)
/// не завышает статистику.
fn insert_entry(
    entries: &mut Vec<FileEntry>,
    path: &str,
    is_dir: bool,
    certainty: FileCertainty,
    source: String,
    content: Option<String>,
    warning: Option<String>,
    counter: &mut usize,
) {
    let normalized = path.trim_start_matches('/').trim_end_matches('/');
    if normalized.is_empty() || normalized == "." {
        return;
    }
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.is_empty() {
        return;
    }
    insert_entry_recursive(
        entries, &parts, 0, path, is_dir, certainty, source, content, warning, counter,
    );
}

fn insert_entry_recursive(
    entries: &mut Vec<FileEntry>,
    parts: &[&str],
    depth: usize,
    full_path: &str,
    is_dir: bool,
    certainty: FileCertainty,
    source: String,
    content: Option<String>,
    warning: Option<String>,
    counter: &mut usize,
) {
    let part = parts[depth];
    let is_last = depth == parts.len() - 1;

    // Ищем существующую запись по индексу
    let existing_idx = entries.iter().position(|e| e.name == part);

    if let Some(idx) = existing_idx {
        if is_last {
            let existing = &mut entries[idx];
            if !existing.is_dir && is_dir {
                existing.is_dir = true;
            }
            if existing.content.is_none() && content.is_some() {
                existing.content = content;
            }
            // Certain (наши файлы) никогда не понижается и перекрывает
            // предупреждения внешних источников. Expected не понижается
            // до Unknown (вторая вставка не ухудшает честность).
            if certainty == FileCertainty::Certain && existing.certainty != FileCertainty::Certain {
                existing.certainty = FileCertainty::Certain;
                existing.source = source;
                existing.warning = None;
            } else if existing.warning.is_none() && warning.is_some() {
                existing.warning = warning;
            }
            return;
        }
        // Не последний — должен быть директорией
        if !entries[idx].is_dir {
            entries[idx].is_dir = true;
        }
        insert_entry_recursive(
            &mut entries[idx].children,
            parts,
            depth + 1,
            full_path,
            is_dir,
            certainty,
            source,
            content,
            warning,
            counter,
        );
    } else {
        // Создаём новую запись
        let entry = FileEntry {
            path: if is_last {
                full_path.to_string()
            } else {
                parts[..=depth].join("/")
            },
            name: part.to_string(),
            is_dir: !is_last || is_dir,
            certainty: if is_last {
                certainty.clone()
            } else {
                FileCertainty::Certain
            },
            source: if is_last {
                source.clone()
            } else {
                "StackPilot generator".into()
            },
            content: if is_last { content.clone() } else { None },
            warning: if is_last { warning.clone() } else { None },
            children: Vec::new(),
        };
        if is_last {
            *counter += 1;
        }
        entries.push(entry);
        if !is_last {
            let last_idx = entries.len() - 1;
            insert_entry_recursive(
                &mut entries[last_idx].children,
                parts,
                depth + 1,
                full_path,
                is_dir,
                certainty,
                source,
                content,
                warning,
                counter,
            );
        }
    }
}

/// Рекурсивная сортировка: папки перед файлами, внутри — по алфавиту.
fn sort_entries(entries: &mut Vec<FileEntry>) {
    entries.sort_by(|a, b| {
        a.is_dir
            .cmp(&b.is_dir)
            .reverse()
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for entry in entries.iter_mut() {
        if entry.is_dir && !entry.children.is_empty() {
            sort_entries(&mut entry.children);
        }
    }
}

