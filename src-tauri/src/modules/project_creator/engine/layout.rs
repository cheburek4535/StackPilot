use super::*;
use std::collections::HashMap;
use std::sync::OnceLock;

// ============================================================================
// Данные о языках/фреймворках — читаются из wizard_tree.json (не код!).
// Раньше списки «фронтенд/бэкенд-фреймворков» дублировались константами
// здесь и полем output_subdir в wizard_tree.json — при правке данных
// движок молча расходился с мастером. Единый источник истины — JSON.
// ============================================================================

pub fn wizard_tree() -> &'static WizardTreeData {
    static TREE: OnceLock<WizardTreeData> = OnceLock::new();
    TREE.get_or_init(|| {
        let raw = include_str!("../knowledge/wizard_tree.json");
        serde_json::from_str(raw).expect("wizard_tree.json должен быть корректным JSON")
    })
}

pub fn framework_def(id: &str) -> Option<&'static FrameworkDef> {
    wizard_tree().frameworks.iter().find(|f| f.id == id)
}

pub fn language_def(id: &str) -> Option<&'static LanguageDef> {
    wizard_tree().languages.iter().find(|l| l.id == id)
}

/// Сторона языка по его category (используется, когда мастер не прислал
/// явные backend_languages/frontend_languages — старые сессии).
/// "both"-языки (csharp, dart, kotlin...) по умолчанию считаются бэкендом.
pub fn language_side_infer(lang: &str) -> Option<&'static str> {
    match language_def(lang).and_then(|l| l.category.as_deref()) {
        Some("backend") | Some("both") => Some("backend"),
        Some("frontend") | Some("static") => Some("frontend"),
        _ => None,
    }
}

// ============================================================================
// Каноническая раскладка проекта (ProjectLayout)
//
// ЕДИНСТВЕННОЕ решение о структуре каталогов проекта: класс раскладки,
// владелец корня, каталоги фреймворков и языков. Вычисляется ОДИН раз из
// WizardContext (plan → compose_recipe) и пронизывает все потребители —
// compose_recipe, steps_for_framework, python-сегментацию, steps_for_vscode,
// README/Docker и duplicate_framework_write_paths. Никаких локальных
// эвристик в потребителях: все спрашивают у ProjectLayout.
//
// Классы:
//   - Integrated — фреймворк side="either" && scaffold="root" (сегодня это
//     только tauri). Оболочка владеет корнем, веб-фронтенд живёт в frontend/,
//     бэкенд-компаньоны — в backend/, языки-компаньоны — по своим сторонам.
//   - Split — есть И бэкенд, И фронтенд (языки или фреймворки). Жёсткие
//     сегменты backend/ + frontend/, корнем не владеет никто: даже
//     scaffold="root" (django, nest, spring-boot) работает ВНУТРИ backend/.
//   - BackendOnly — только бэкенд: всё в корне.
//   - FrontendOnly — только фронтенд: скаффолдеры с output_subdir="frontend"
//     в frontend/, остальное в корне.
// ============================================================================

/// Стороны проекта по контексту: явные назначения мастера
/// (backend_languages/frontend_languages), вывод по category языков — и side
/// фреймворков из wizard_tree.json. Фреймворк с жёсткой стороной (side=
/// "backend"/"frontend") — полноценная сторона: nest (backend) + nextjs
/// (frontend) включают сегментацию, даже если в контексте единственный язык
/// (typescript) или он не назначен бэкенд-стороне (aspnetcore + maui — оба
/// на csharp).
pub fn context_sides(context: &WizardContext) -> (bool, bool) {
    let lang_side = lang_side_map(context);
    let mut has_backend = lang_side.values().any(|s| *s == "backend");
    let mut has_frontend = lang_side.values().any(|s| *s == "frontend");
    for fw in &context.frameworks {
        match framework_def(fw).map(|def| def.side.as_str()) {
            Some("backend") => has_backend = true,
            Some("frontend") => has_frontend = true,
            _ => {}
        }
    }
    (has_backend, has_frontend)
}

/// Язык → сторона: явные назначения мастера (backend_languages /
/// frontend_languages) имеют приоритет; языки без назначения — по category
/// (обратная совместимость со старыми сессиями), "both"-языки по умолчанию
/// считаются бэкендом (csharp, dart, kotlin...).
pub fn lang_side_map(context: &WizardContext) -> HashMap<String, &'static str> {
    let mut lang_side: HashMap<String, &'static str> = HashMap::new();
    for l in &context.backend_languages {
        lang_side.insert(l.clone(), "backend");
    }
    for l in &context.frontend_languages {
        lang_side.insert(l.clone(), "frontend");
    }
    for l in &context.languages {
        lang_side
            .entry(l.clone())
            .or_insert_with(|| language_side_infer(l).unwrap_or("backend"));
    }
    lang_side
}

/// Каноническая раскладка проекта (см. шапку секции выше).
pub struct ProjectLayout {
    pub class: LayoutClass,
    /// Каталоги, которые движок создаёт ДО всех скаффолдеров (только split).
    pub eager_dirs: Vec<String>,
    /// Каталоги сегментов по сторонам (None = корень/стороны нет).
    pub backend_dir: Option<String>,
    pub frontend_dir: Option<String>,
    /// Владелец корня при integrated (tauri) — None иначе.
    pub root_owner: Option<String>,
    /// Явные назначения сторон мастера (backend_languages/frontend_languages).
    pub explicit_side: HashMap<String, &'static str>,
    /// Все выбранные фреймворки (для framework-ассоциаций языков).
    pub frameworks: Vec<String>,
}

pub enum LayoutClass {
    /// Интегрированная оболочка (tauri): side="either" && scaffold="root".
    /// Оболочка владеет корнем, веб в frontend/, серверные компаньоны в
    /// backend/.
    Connected,
    /// backend/ + frontend/ — сегменты моно-репозитория: два независимых
    /// веб-приложения (API + SPA), никакой из сторон корень не принадлежит.
    Separated,
    /// Только бэкенд — всё в корне.
    BackendOnly,
    /// Только фронтенд — скаффолдеры в frontend/, остальное в корне.
    FrontendOnly,
    /// Неинтегрированная клиентская оболочка (electron, expo, react-native,
    /// plasmo) + REST API-бэкенд: клиент в frontend/, API в backend/. В
    /// отличие от Connected, оболочка НЕ владеет корнем — она клиент,
    /// общающийся с API по HTTP (см. client_shell_frameworks).
    ShellClientApi,
    /// Ничего не выбрано (пустой стек) — канонической раскладки нет.
    Custom,
}

impl ProjectLayout {
    /// Единственная точка решения о раскладке проекта. Никакие другие
    /// функции не догадываются о каталогах сами.
    pub fn compute(context: &WizardContext) -> ProjectLayout {
        // Явные назначения сторон мастера — только они, без выводов по category
        // (см. side_for_language: явное > ассоциация фреймворка > category).
        let mut explicit_side: HashMap<String, &'static str> = HashMap::new();
        for l in &context.backend_languages {
            explicit_side.insert(l.clone(), "backend");
        }
        for l in &context.frontend_languages {
            explicit_side.insert(l.clone(), "frontend");
        }

        // Integrated-оболочка: side="either" && scaffold="root" (tauri).
        // Проверяется ДО сторон: tauri делает стек integrated независимо
        // от того, есть ли рядом бэкенд и фронтенд.
        let integrated_shell = context.frameworks.iter().find(|fw| {
            framework_def(fw)
                .is_some_and(|def| def.side == "either" && def.scaffold.as_deref() == Some("root"))
        });
        if let Some(shell) = integrated_shell {
            let mut has_backend_companion = false;
            let mut has_frontend_companion = false;
            for fw in &context.frameworks {
                if fw == shell {
                    continue;
                }
                match framework_def(fw).map(|def| def.side.as_str()) {
                    Some("backend") => has_backend_companion = true,
                    Some("frontend") => has_frontend_companion = true,
                    _ => {}
                }
            }
            return ProjectLayout {
                class: LayoutClass::Connected,
                eager_dirs: Vec::new(),
                backend_dir: has_backend_companion.then(|| "backend".to_string()),
                frontend_dir: has_frontend_companion.then(|| "frontend".to_string()),
                root_owner: Some(shell.clone()),
                explicit_side,
                frameworks: context.frameworks.clone(),
            };
        }

        // Обе стороны (языки И фреймворки) → Separated; неинтегрированная
        // клиентская оболочка (electron, expo, react-native, plasmo) рядом с
        // бэкендом → ShellClientApi (клиент в frontend/, API в backend/).
        // Иначе одно-сторонняя раскладка: всё в корне (BackendOnly /
        // FrontendOnly). Пустой стек (нет ни языков, ни фреймворков) —
        // Custom: канонической раскладки нет.
        let (has_backend, has_frontend) = context_sides(context);
        let non_integrated_client_shell = context.frameworks.iter().any(|fw| {
            wizard_tree()
                .client_shell_frameworks
                .iter()
                .any(|shell| shell == fw)
        });
        let class = if has_backend && has_frontend {
            if non_integrated_client_shell {
                LayoutClass::ShellClientApi
            } else {
                LayoutClass::Separated
            }
        } else if has_backend {
            LayoutClass::BackendOnly
        } else if has_frontend {
            LayoutClass::FrontendOnly
        } else {
            LayoutClass::Custom
        };
        let is_split = matches!(class, LayoutClass::Separated | LayoutClass::ShellClientApi);
        ProjectLayout {
            eager_dirs: if is_split {
                vec!["backend".to_string(), "frontend".to_string()]
            } else {
                Vec::new()
            },
            backend_dir: is_split.then(|| "backend".to_string()),
            frontend_dir: is_split.then(|| "frontend".to_string()),
            root_owner: None,
            explicit_side,
            frameworks: context.frameworks.clone(),
            class,
        }
    }

    /// Фреймворк владеет корнем проекта?
    ///
    ///   - Connected: только сама оболочка (tauri) — её CLI и shell живут
    ///     в корне рядом с frontend/ и backend/;
    ///   - Separated/ShellClientApi: корнем не владеет никто — даже
    ///     scaffold="root" (django, nest, spring-boot) работает ВНУТРИ
    ///     backend/;
    ///   - BackendOnly/FrontendOnly: root-скаффолдер остаётся в корне;
    ///   - Custom: корня как такового нет — владельца нет.
    pub fn owns_root(&self, fw: &str) -> bool {
        match &self.class {
            LayoutClass::Connected => self.root_owner.as_deref() == Some(fw),
            LayoutClass::Separated | LayoutClass::ShellClientApi | LayoutClass::Custom => false,
            _ => framework_def(fw).is_some_and(|d| d.scaffold.as_deref() == Some("root")),
        }
    }

    /// Сторона языка (None = корень/не определена). Приоритет:
    ///   1. явное назначение мастера (backend_languages/frontend_languages);
    ///   2. язык самой integrated-оболочки — остаётся с ней в корне;
    ///   3. язык, требуемый фреймворком с жёсткой стороной (dart+flutter →
    ///      frontend — решает кейс zig+flutter; если язык нужен фреймворкам
    ///      ОБЕИХ сторон, правило неоднозначно — уступает category);
    ///   4. вывод по category (обратная совместимость со старыми сессиями).
    pub fn side_for_language(&self, lang: &str) -> Option<&'static str> {
        if let Some(side) = self.explicit_side.get(lang).copied() {
            return Some(side);
        }
        if let LayoutClass::Connected = &self.class {
            if let Some(shell) = &self.root_owner {
                if framework_def(shell).is_some_and(|def| def.languages.iter().any(|l| l == lang)) {
                    return None; // корень оболочки
                }
            }
        }
        let mut required_by: Vec<&'static str> = Vec::new();
        for fw in &self.frameworks {
            let def = framework_def(fw);
            if def.is_some_and(|d| d.languages.iter().any(|l| l == lang)) {
                match def.map(|d| d.side.as_str()) {
                    Some("backend") if !required_by.contains(&"backend") => {
                        required_by.push("backend")
                    }
                    Some("frontend") if !required_by.contains(&"frontend") => {
                        required_by.push("frontend")
                    }
                    _ => {}
                }
            }
        }
        if required_by.len() == 1 {
            return Some(required_by[0]);
        }
        language_side_infer(lang)
    }

    /// Каталог сегмента для стороны (None = корень).
    fn dir_for_side(&self, side: &str) -> Option<String> {
        match (&self.class, side) {
            (LayoutClass::Separated, "backend") => self.backend_dir.clone(),
            (LayoutClass::Separated, "frontend") => self.frontend_dir.clone(),
            (LayoutClass::ShellClientApi, "backend") => self.backend_dir.clone(),
            (LayoutClass::ShellClientApi, "frontend") => self.frontend_dir.clone(),
            // Connected: каталог существует только когда на этой стороне
            // есть компаньон (backend/ при fastapi, frontend/ при react).
            // Языки и фреймворки без компаньона остаются в корне рядом с
            // оболочкой.
            (LayoutClass::Connected, "backend") => self.backend_dir.clone(),
            (LayoutClass::Connected, "frontend") => self.frontend_dir.clone(),
            _ => None,
        }
    }

    /// Каталог языка (None = корень проекта).
    pub fn language_dir(&self, lang: &str) -> Option<String> {
        self.side_for_language(lang)
            .and_then(|side| self.dir_for_side(side))
    }

    /// Каталог фреймворка (None = корень проекта). Приоритет — жёсткая
    /// сторона фреймворка (side в wizard_tree.json); для side="either" —
    /// сторона требуемого языка. В одно-сторонних раскладках всё остаётся
    /// в корне, кроме frontend-скаффолдеров FrontendOnly (output_subdir =
    /// "frontend": react, nextjs, flutter... в frontend/).
    pub fn framework_dir(&self, fw: &str) -> Option<String> {
        if let Some(def) = framework_def(fw) {
            match def.side.as_str() {
                "backend" => return self.dir_for_side("backend"),
                "frontend" => {
                    return match &self.class {
                        LayoutClass::FrontendOnly
                            if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw) =>
                        {
                            Some("frontend".to_string())
                        }
                        _ => self.dir_for_side("frontend"),
                    };
                }
                _ => {}
            }
            if def.side == "either" {
                for lang in &def.languages {
                    if let Some(side) = self.side_for_language(lang) {
                        return self.dir_for_side(side);
                    }
                }
            }
        }
        None
    }

    /// Каталоги, которые движок создаёт ДО всех CLI-скаффолдеров
    /// (только split: backend/ + frontend/). Integrated и одно-сторонние
    /// раскладки папок не предсоздают — их создают сами генераторы
    /// (ScaffoldGenerator.resolve_target) или WriteFile.
    pub fn eager_dirs(&self) -> &[String] {
        &self.eager_dirs
    }

    /// Человекочитаемый снимок решения для предпросмотра (RecipePreview).
    pub fn to_summary(&self, context: &WizardContext) -> LayoutSummary {
        let class = match &self.class {
            LayoutClass::Separated => "separated",
            LayoutClass::Connected => "connected",
            LayoutClass::BackendOnly => "backend-only",
            LayoutClass::FrontendOnly => "frontend-only",
            LayoutClass::ShellClientApi => "shell-client-api",
            LayoutClass::Custom => "custom",
        };
        let framework_placement = context
            .frameworks
            .iter()
            .map(|fw| FrameworkPlacement {
                framework: fw.clone(),
                directory: self.framework_dir(fw).unwrap_or_else(|| ".".to_string()),
            })
            .collect();
        LayoutSummary {
            class: class.to_string(),
            generated_directories: self.eager_dirs.clone(),
            root_owner: self.root_owner.clone(),
            framework_placement,
        }
    }
}

/// Generic-скаффолд языка подавляется, если выбран фреймворк, который сам
/// создаёт каркас проекта для этого языка (aspnetcore вместо `dotnet new
/// console`, nextjs вместо js-скаффолда, spring-boot вместо maven archetype
/// и т.п.). Флаг suppresses_language_scaffold живёт в wizard_tree.json.
///
/// ЕДИНСТВЕННОЕ исключение из «глушим всегда» — язык стоит на ФРОНТЕНДЕ, а
/// фреймворк-подавитель (aspnetcore, express, vapor...) — на БЭКЕНДЕ: это
/// разные сегменты (frontend/ и backend/), и generic-каркас языка нужен
/// фронтенду. Иначе python-бэкенд + plain TypeScript-фронтенд / aspnetcore +
/// C#-фронтенд оставляли бы фронтенд без каркаса. Остальные комбинации
/// (в т.ч. бэкенд-язык + фронтенд-фреймворк, как TS-бэкенд + react) глушатся
/// как раньше.
pub fn language_scaffold_suppressed(lang: &str, layout: &ProjectLayout, context: &WizardContext) -> bool {
    // tauri: фронтенд (JS/TS) живёт в frontend/ и создаётся сам —
    // компаньоном react/vue/svelte или vite-vanilla. Generic-скаффолд
    // JS/TS (package.json, src/, tsc --init) в корне не нужен: он либо
    // конфликтует с каркасом tauri, либо пишет мусорные заглушки.
    if matches!(lang, "typescript" | "javascript")
        && context.frameworks.iter().any(|f| f == "tauri")
    {
        return true;
    }
    let lang_on_frontend = layout.side_for_language(lang) == Some("frontend");
    context.frameworks.iter().any(|fw| {
        framework_def(fw).is_some_and(|def| {
            def.suppresses_language_scaffold
                && def.languages.iter().any(|l| l == lang)
                && !(lang_on_frontend && def.side == "backend")
        })
    })
}

/// Степ-иды языковых инициализаций, чьи FileNotExists-маркеры
/// относительны РАБОЧЕЙ ДИРЕКТОРИИ шага (cargo init создаёт
/// Cargo.toml рядом с собой и т.п.). При сегментации (into_segment)
/// маркер должен переехать вместе с рабочей директорией в
/// сегмент: иначе повторный запуск рецепта в mono-репо проверяет
/// корень проекта, и init заново запускается, падая с
/// "already exists" (молчаливый skip).
const WORKDIR_MARKER_STEPS: &[&str] = &[
    "cargo_init",
    "tsc_init",
    "go_mod_init",
    "maven_init",
    "mix_new",
    "gleam_new",
    "dart_create",
];

pub fn join_seg(wd: &str, seg: &str) -> String {
    if wd.is_empty() || wd == "." {
        seg.to_string()
    } else {
        format!("{}/{}", wd.trim_end_matches(['/', '\\']), seg)
    }
}

/// Заворачивает шаги фреймворка в каталог сегмента (backend/ или frontend/):
/// пути WriteFile/CreateDirectory и рабочие директории команд получают
/// префикс. Генераторы подпапок (FOLDER_MAKER_STEPS) получают имя сегмента
/// вместо имени проекта (create-electron-app frontend из корня).
pub fn into_segment(steps: Vec<Step>, dir: &str) -> Vec<Step> {
    steps
        .into_iter()
        .map(|step| {
            match step {
                Step::Command {
                    id,
                    label,
                    description,
                    command,
                    args,
                    working_dir,
                    env,
                    timeout_secs,
                    condition,
                    on_error,
                    interactive,
                } => {
                    if id.starts_with("tauri_web_") || id.starts_with("qt_web_") {
                        // Веб-часть tauri (tauri_web_*) живёт в frontend/
                        // независимо от сегмента самого tauri (backend/ в
                        // моно-репозитории); qt_web_* — сборка веб-части Qt
                        // WebEngine (frontend/) рядом с qt-сегментом (backend/).
                        // Рабочая директория уже относительна корня проекта,
                        // сегментация её НЕ трогает.
                        Step::Command {
                            id,
                            label,
                            description,
                            command,
                            args,
                            working_dir,
                            env,
                            timeout_secs,
                            condition,
                            on_error,
                            interactive,
                        }
                    } else {
                        // Маркеры языковых init-шагов относительны рабочей
                        // директории — при сегментации переезжают в сегмент.
                        let condition = if WORKDIR_MARKER_STEPS.contains(&id.as_str()) {
                            condition.map(|c| match c {
                                StepCondition::FileNotExists { path } => {
                                    StepCondition::FileNotExists {
                                        path: join_seg(dir, &path),
                                    }
                                }
                                other => other,
                            })
                        } else {
                            condition
                        };
                        Step::Command {
                            id,
                            label,
                            description,
                            command,
                            args,
                            working_dir: working_dir.map(|wd| join_seg(&wd, dir)),
                            env,
                            timeout_secs,
                            condition,
                            on_error,
                            interactive,
                        }
                    }
                }
                Step::WriteFile {
                    id,
                    label,
                    description,
                    path,
                    content,
                    overwrite,
                    policy,
                    condition,
                    on_error,
                } => Step::WriteFile {
                    id,
                    label,
                    description,
                    path: format!("{}/{}", dir, path),
                    content,
                    overwrite,
                    policy,
                    condition,
                    on_error,
                },
                Step::CreateDirectory {
                    id,
                    label,
                    description,
                    path,
                    condition,
                    on_error,
                } => Step::CreateDirectory {
                    id,
                    label,
                    description,
                    path: format!("{}/{}", dir, path),
                    condition,
                    on_error,
                },
                // Scaffold-генератор (Step::Generate "scaffold") сам кладёт проект
                // в target_dir: при сегментации каталогом становится сегмент.
                // Шаги с явным каталогом, не зависящим от раскладки, не трогаем
                // (см. scaffold_lands_in_target_dir: tauri_web_scaffold —
                // веб-часть всегда frontend/, tauri_init — оболочка в корне,
                // does_not_create_a_project — CLI без каталога проекта).
                // Spring Boot (генератор "spring-boot") распаковывает starter
                // внутри сегмента — каталог передаётся через target_dir.
                Step::Generate {
                    id,
                    label,
                    description,
                    generator_id,
                    mut generator_config,
                    policy,
                    condition,
                    on_error,
                } => {
                    if scaffold_lands_in_target_dir(&id, &generator_id, &generator_config) {
                        generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                    }
                    if generator_id == "spring-boot" {
                        generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                    }
                    Step::Generate {
                        id,
                        label,
                        description,
                        generator_id,
                        generator_config,
                        policy,
                        condition,
                        on_error,
                    }
                }
                other => other,
            }
        })
        .collect()
}

/// Scaffold-генератор кладёт проект в target_dir: при сегментации — сегмент
/// (into_segment), в монолите — scaffold_target_dir (output_subdir фреймворка).
/// ЕДИНСТВЕННОЕ правило, по которому движок решает, можно ли переопределять
/// каталог scaffold-шага; используется и в into_segment, и в
/// steps_for_framework (иначе логика расходится). Шаги с ЯВНЫМ каталогом,
/// не зависящим от раскладки, не трогаются:
///   - tauri_web_scaffold: веб-часть tauri ВСЕГДА в frontend/;
///   - tauri_init: integrated-оболочка работает в корне (src-tauri/ в корне);
///   - does_not_create_a_project: CLI не создаёт каталог проекта (zig init —
///     раскладывает shell в текущем каталоге, пост-условия валидируются
///     относительно рабочей директории).
pub fn scaffold_lands_in_target_dir(
    id: &str,
    generator_id: &str,
    generator_config: &serde_json::Value,
) -> bool {
    generator_id == "scaffold"
        && id != "tauri_web_scaffold"
        && id != "tauri_init"
        && generator_config
            .get("capability")
            .and_then(|v| v.as_str())
            .map_or(true, |c| c != "does_not_create_a_project")
}

/// Скаффолдеры, которые генерируют package.json и называют его по имени
/// папки (frontend/, <project_name>/) вместо project_name из WizardContext.
/// Для них движок добавляет пост-шаг, переписывающий поле name
/// (см. package_name_patch_step) — чинит баг «frontend/package.json
/// называется frontend».
const PACKAGE_JSON_SCAFFOLDS: &[&str] = &[
    "react",
    "vue",
    "svelte",
    "nextjs",
    "sveltekit",
    "nuxt",
    "solidjs",
    "electron",
    "expo",
    "react-native",
    "plasmo",
    "tauri",
    "nest",
];

/// Фреймворки, чей каркас создаёт ScaffoldGenerator (Step::Generate
/// "scaffold", см. generators/mod.rs): CLI-генератор вызывается по ЯВНОЙ
/// способности (creates_named_directory / creates_project_and_may_prompt) —
/// во временную папку с программным переносом (temp+move) или в текущий
/// каталог — без матрёшек testapp/testapp.
/// Для них каталог проекта — scaffold_target_dir(...), а не подпапка
/// <project_name>/ (см. также js_dirs и pkg-name patch).
pub const SCAFFOLD_GENERATOR_FRAMEWORKS: &[&str] = &[
    "react",
    "vue",
    "svelte",
    "nextjs",
    "sveltekit",
    "nuxt",
    "expo",
    "solidjs",
    "flutter",
    "electron",
    "laravel",
    "symfony",
    "react-native",
    "plasmo",
];

/// Каталог, куда ScaffoldGenerator кладёт проект: при сегментации — каталог
/// сегмента; в монолите — по output_subdir фреймворка (frontend → "frontend",
/// backend-фреймворки (laravel/symfony) → корень ".").
pub fn scaffold_target_dir(fw: &str, seg: Option<&str>) -> String {
    if let Some(dir) = seg {
        return dir.to_string();
    }
    match framework_def(fw).and_then(|def| def.output_subdir.as_deref()) {
        Some("frontend") => "frontend".to_string(),
        _ => ".".to_string(),
    }
}

/// Пост-шаг после CLI-скаффолдинга: переписывает ТОЛЬКО поле name в
/// package.json (node -e сохраняет форматирование и остальные поля).
/// Исправляет баг шаблонизатора: скаффолдер называет проект по имени
/// родительской папки (frontend/) вместо project_name из WizardContext.
pub fn package_name_patch_step(
    id: &str,
    label: &str,
    workdir: Option<&str>,
    project_name: &str,
) -> Step {
    // Апостроф в имени проекта ломает JS-строку — экранируем.
    let safe_name = project_name.replace('\'', "\\'");
    // Патч выполняется ТОЛЬКО если скаффолдер действительно создал
    // package.json (postcondition) — при провале скаффолда шаг пропускается
    // вместо вторичной ENOENT-ошибки.
    let pkg_path = match workdir {
        Some(wd) if !wd.is_empty() && wd != "." => format!("{}/package.json", wd),
        _ => "package.json".to_string(),
    };
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: "Set package.json name to the real project name".into(),
        command: "node".into(),
        args: vec![
            "-e".into(),
            format!(
                "const fs=require('fs');const p='package.json';const j=JSON.parse(fs.readFileSync(p,'utf8'));j.name='{}';fs.writeFileSync(p,JSON.stringify(j,null,2)+'\\n')",
                safe_name
            ),
        ],
        working_dir: workdir.map(String::from),
        env: None,
        timeout_secs: Some(30),
        condition: Some(StepCondition::FileExists { path: pkg_path }),
        on_error: ErrorMode::Abort,
        interactive: vec![],
    }
}

/// Шаги фреймворка с учётом канонической раскладки (ProjectLayout):
/// сегментация, целевые каталоги scaffold-генераторов и пост-патч имени
/// package.json. Все каталоги берутся ТОЛЬКО из layout — никаких локальных
/// эвристик (root_rest_seg и т.п. больше нет).
/// Метаданные владения каркасом: что именно создаёт CLI-скаффолдер и как
/// движок может с ним работать. ЕДИНСТВЕННАЯ таблица — поведение каждого
/// скаффолдера описывается здесь, а не разрозненными проверками по id
/// фреймворка в compose_recipe / steps_for_framework / into_segment.
/// Проверки «fw == "tauri"», «fw == "electron"» вне этой таблицы — ошибка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaffoldOwnership {
    /// CLI создаёт полный каркас приложения (create-vite, create-electron-app,
    /// flutter create, tauri init).
    pub creates_app_shell: bool,
    /// Каркас включает СОБСТВЕННЫЙ фронтенд (react/vite, electron/renderer,
    /// flutter/dart-ui). UI-компаньоны (react/vue/svelte) рядом с таким
    /// владельцем не скаффолдятся отдельно — их UI уже встроен в каркас.
    pub creates_frontend: bool,
    /// Встраиваемый UI-компаньон (react/vue/svelte): UI-библиотека, которую
    /// мастера выбирают ПОД оболочку (tauri, electron, qt-webengine). Рядом
    /// с каркасом, у которого есть собственный фронтенд, компаньон не
    /// скаффолдится (compose_recipe) — его UI уже встроен в каркас.
    pub is_ui_companion: bool,
    /// CLI требует ПУСТОЙ каталог назначения: в существующем каталоге с
    /// посторонними файлами CLI падает (create-next-app, create-expo-app).
    pub requires_empty_dir: bool,
    /// CLI умеет дописывать каркас в УЖЕ СУЩЕСТВУЮЩИЙ каталог (nest new .,
    /// tauri init, zig init, flutter create .).
    pub may_run_in_existing_dir: bool,
    /// CLI может выполняться во временной папке (temp+move): созданный
    /// каталог программно переносится в каталог назначения.
    pub supports_staging_dir: bool,
    /// Выход CLI можно программно слить с каталогом назначения.
    pub output_mergeable: bool,
    /// Гарантированные выходные пути (пост-условия валидации) относительно
    /// каталога назначения.
    pub expected_outputs: &'static [&'static str],
    /// CLI — обёртка над другим проектом: требует уже существующий каркас
    /// (tauri init поверх фронтенда). Шаги обёртки откладываются в конец
    /// фазы скаффолдинга.
    pub wraps_existing_project: bool,
}

impl ScaffoldOwnership {
    pub const fn none() -> Self {
        Self {
            creates_app_shell: false,
            creates_frontend: false,
            is_ui_companion: false,
            requires_empty_dir: false,
            may_run_in_existing_dir: false,
            supports_staging_dir: false,
            output_mergeable: false,
            expected_outputs: &[],
            wraps_existing_project: false,
        }
    }

    /// Каркас создаёт собственный фронтенд целиком (app shell + UI):
    /// electron/renderer, flutter/dart-ui, react/vite...
    pub fn creates_frontend_shell(&self) -> bool {
        self.creates_app_shell && self.creates_frontend
    }

    /// Единственная таблица владения: id фреймворка → метаданные его
    /// скаффолдера. Фреймворки без CLI-каркаса (inplace: express, fastapi,
    /// axum...) — ScaffoldOwnership::none().
    pub fn for_framework(fw: &str) -> Self {
        match fw {
            // Встраиваемые UI-компаньоны и веб-фреймворки: полный
            // фронтенд-каркас в каталоге назначения.
            "react" | "vue" | "svelte" | "nextjs" | "sveltekit" | "nuxt" | "expo" | "solidjs" => {
                Self {
                    creates_app_shell: true,
                    creates_frontend: true,
                    is_ui_companion: matches!(fw, "react" | "vue" | "svelte"),
                    requires_empty_dir: true,
                    may_run_in_existing_dir: false,
                    supports_staging_dir: true,
                    output_mergeable: true,
                    expected_outputs: &["package.json"],
                    wraps_existing_project: false,
                }
            }
            // Electron: create-electron-app собирает ПОЛНЫЙ каркас
            // (main + renderer) — собственный фронтенд, UI-компаньоны
            // подавляются (см. compose_recipe). Forge init в непустом
            // каталоге назначения падает (запрещено) — каркас собирается
            // во временной папке (temp+move) и переносится программно.
            "electron" => Self {
                creates_app_shell: true,
                creates_frontend: true,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: false,
                supports_staging_dir: true,
                output_mergeable: true,
                expected_outputs: &["package.json"],
                wraps_existing_project: false,
            },
            // Flutter: flutter create --project-name <имя> . работает
            // ВНУТРИ каталога назначения (в существующем каталоге — dart-ui).
            "flutter" => Self {
                creates_app_shell: true,
                creates_frontend: true,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: true,
                expected_outputs: &["pubspec.yaml", "lib"],
                wraps_existing_project: false,
            },
            // Tauri: tauri init раскладывает src-tauri/ shell В КОРНЕ
            // существующего проекта — обёртка над фронтендом, который
            // обязан существовать ДО init (компаньон или vite-vanilla).
            "tauri" => Self {
                creates_app_shell: true,
                creates_frontend: false,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: false,
                expected_outputs: &["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                wraps_existing_project: true,
            },
            // Qt: C++-каркас с собственным UI-стеком (QML/Widgets/WebEngine/
            // Kirigami). Режим WebEngine встраивает веб-фронтенд
            // (react/vue/svelte): qt-шаги сборки веб-части выполняются
            // ПОСЛЕ фронтенд-скаффолда (явные зависимости в compose_recipe).
            "qt" => Self {
                creates_app_shell: true,
                creates_frontend: false,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: false,
                expected_outputs: &["CMakeLists.txt", "src/main.cpp"],
                wraps_existing_project: false,
            },
            _ => Self::none(),
        }
    }
}

/// Каркас-владелец фронтенда (не-компаньон): electron, flutter, nextjs...
/// Его наличие подавляет UI-компаньонов (react/vue/svelte) в compose_recipe.
pub fn creates_frontend_shell(fw: &str) -> bool {
    let o = ScaffoldOwnership::for_framework(fw);
    o.creates_frontend_shell() && !o.is_ui_companion
}

pub fn steps_for_framework(
    fw: &str,
    project_path: &str,
    project_name: &str,
    context: &WizardContext,
    layout: &ProjectLayout,
) -> Vec<Step> {
    let seg = layout.framework_dir(fw);
    let mut steps =
        steps_for_framework_impl(fw, project_path, project_name, context, seg.as_deref());

    // Inplace-фреймворки (scaffold не задан: express, fastapi, gin, clap...)
    // дописывают файлы в каркас, созданный language-скаффолдом. Их файлы —
    // это «настоящий» контент приложения, а скаффолд языка — заглушка:
    // express обязан перезаписать package.json/src/index.js, иначе шаг
    // молча скипается (executor не пишет поверх при overwrite=false).
    // Раньше это обещание было описано в коммите, но не реализовано —
    // express-шаги в связке js+express просто пропадали.
    if let Some(def) = framework_def(fw) {
        if def.scaffold.is_none() {
            for step in &mut steps {
                if let Step::WriteFile { overwrite, .. } = step {
                    *overwrite = true;
                }
            }
        }
    }

    let mut steps = match seg.as_deref() {
        Some(dir) => into_segment(steps, dir),
        None => steps,
    };

    // Scaffold-генераторы: into_segment не трогает Generate-шаги, поэтому
    // целевой каталог выставляется здесь — по сегменту или output_subdir.
    // Проверка СТРУКТУРНАЯ (по фактическим шагам фреймворка), а не по
    // списку id: любой каркас, чей шаг — Step::Generate "scaffold",
    // попадает под правило. Шаги с явным каталогом (tauri_init,
    // tauri_web_scaffold, does_not_create_a_project) не трогаются —
    // см. scaffold_lands_in_target_dir (та же логика, что в into_segment).
    let has_scaffold_generate = steps.iter().any(|s| {
        matches!(s, Step::Generate { id, generator_id, generator_config, .. }
            if scaffold_lands_in_target_dir(id, generator_id, generator_config))
    });
    if has_scaffold_generate {
        let target_dir = scaffold_target_dir(fw, seg.as_deref());
        for step in &mut steps {
            if let Step::Generate {
                id,
                generator_id,
                generator_config,
                ..
            } = step
            {
                if scaffold_lands_in_target_dir(id, generator_id, generator_config) {
                    generator_config["target_dir"] = serde_json::Value::String(target_dir.clone());
                }
            }
        }
    }

    // Баг шаблонизатора: скаффолдеры (create-vite, create-tauri-app,
    // create-next-app...) называют package.json по имени папки, в которую
    // пишут (frontend/ или корень), а не по project_name из WizardContext.
    // Пост-шаг примешивается ПОСЛЕ сегментации — его рабочая директория
    // должна указывать на фактическое место package.json.
    if framework_def(fw).is_some() {
        // package.json появляется у фреймворков со scaffold-каркасом
        // (root/subdir) и у integrated-оболочки tauri (frontend/).
        let scaffold_root = layout.owns_root(fw);
        if (scaffold_root || framework_def(fw).is_some_and(|d| d.scaffold.is_some()))
            && PACKAGE_JSON_SCAFFOLDS.contains(&fw)
        {
            let workdir: Option<String> =
                if ScaffoldOwnership::for_framework(fw).wraps_existing_project {
                    // Обёртка (tauri): package.json принадлежит встроенному
                    // фронтенду (frontend/), а не оболочке.
                    Some("frontend".to_string())
                } else if has_scaffold_generate {
                    // package.json лежит в каталоге, куда скаффолдер положил проект
                    Some(scaffold_target_dir(fw, seg.as_deref()))
                } else if scaffold_root {
                    // root-скаффолдеры (nest, django) создают package.json в корне
                    None
                } else {
                    // subdir-скаффолдеры — внутри созданной подпапки (сегмент
                    // frontend/ в моно-репозитории или <project_name> в монолите)
                    Some(seg.unwrap_or_else(|| project_name.to_string()))
                };
            steps.push(package_name_patch_step(
                &format!("{}_pkg_name", fw),
                &format!("Fix package.json name for {}", fw),
                workdir.as_deref(),
                project_name,
            ));
            // Пост-валидация package.json: каркас обязан реально содержать
            // зависимость фреймворка (npm install финальной фазы установит
            // её), а не только entry-файл. Зависимость `<fw>_pkg_check` ←
            // scaffold-шаг объявлена в compose_recipe.
            if let Some(dep_name) = framework_npm_dependency(fw) {
                steps.push(preflight::package_json_check_step(
                    &format!("{}_pkg_check", fw),
                    &format!("Validate {} package.json", fw),
                    &package_json_rel_path(workdir.as_deref()),
                    &[dep_name],
                ));
            }
        }
    }

    steps
}

/// Проверка целостности генерации: не конфликтуют ли фреймворки за одни и
/// те же пути/каталоги/манифесты. Пути считаются ТОЧНО как в compose_recipe —
/// через каноническую раскладку (ProjectLayout), иначе легальные связки
/// (tauri→backend/, react→frontend/) дали бы ложные срабатывания.
///
/// Учитываются все «писатели» каркасов:
///   - WriteFile (включая policy=Overwrite: даже при overwrite=false второй
///     пишущий молча скипнется — executor не пишет поверх);
///   - expected_outputs scaffold-генератора (Step::Generate "scaffold"):
///     файлы, которые CLI обязан создать в target_dir — пересечение
///     с WriteFile другого каркаса тоже теряет файл;
///   - каталоги: CreateDirectory И target_dir scaffold-генератора — два
///     каркаса, раскладывающие каркасы в один каталог (staging-merge
///     второго CLI сломает каркас первого);
///   - манифесты: MergeJson-патчи одного файла двумя каркасами (патчи
///     перезаписывают друг друга; одиночный MergeJson поверх чужого
///     WriteFile — штатный dep-патч side-фреймворка, не конфликт);
///   - владение корнем: два root-скаффолдера (django+nest, nest+django)
///     в одном проекте — вторая генерация сломает первую.
///
/// UI-компаньоны (react/vue/svelte) подавляются рядом с каркасом, у
/// которого есть собственный фронтенд (electron, flutter, expo...) — ТА ЖЕ
/// логика, что в compose_recipe (иначе electron+react дал бы ложное
/// срабатывание по frontend/package.json).
pub fn duplicate_framework_write_paths(context: &WizardContext) -> Vec<String> {
    let project_name = context
        .project_name
        .clone()
        .unwrap_or_else(|| "app".to_string());
    // Каталоги считаются ТОЧНО как в compose_recipe — через каноническую
    // раскладку (ProjectLayout), иначе легальные связки (tauri→корень,
    // react→frontend/) дали бы ложные срабатывания.
    let layout = ProjectLayout::compute(context);
    let frontend_shell_present = context
        .frameworks
        .iter()
        .any(|fw| creates_frontend_shell(fw));
    let mut by_path: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut by_dir: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut by_manifest: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut root_owners: Vec<String> = Vec::new();
    for fw in &context.frameworks {
        if frontend_shell_present && ScaffoldOwnership::for_framework(fw).is_ui_companion {
            continue;
        }
        let steps = steps_for_framework(fw, ".", &project_name, context, &layout);
        for step in &steps {
            match step {
                Step::WriteFile { path, policy, .. } => {
                    let bucket = if *policy == Some(FilePolicy::MergeJson) {
                        &mut by_manifest
                    } else {
                        &mut by_path
                    };
                    bucket.entry(path.clone()).or_default().push(fw.clone());
                }
                Step::CreateDirectory { path, .. } => {
                    by_dir
                        .entry(format!("@dir:{}", path))
                        .or_default()
                        .push(fw.clone());
                }
                Step::Generate {
                    generator_id,
                    generator_config,
                    ..
                } => {
                    if generator_id != "scaffold" {
                        continue;
                    }
                    let target_dir = generator_config
                        .get("target_dir")
                        .and_then(|v| v.as_str())
                        .unwrap_or(".");
                    by_dir
                        .entry(format!("@dir:{}", target_dir))
                        .or_default()
                        .push(fw.clone());
                    if let Some(outputs) = generator_config
                        .get("expected_outputs")
                        .and_then(|v| v.as_array())
                    {
                        for output in outputs.iter().filter_map(|o| o.as_str()) {
                            let full = if target_dir == "." {
                                output.to_string()
                            } else {
                                format!("{}/{}", target_dir, output)
                            };
                            by_path.entry(full).or_default().push(fw.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        if layout.owns_root(fw) && !ScaffoldOwnership::for_framework(fw).wraps_existing_project {
            root_owners.push(fw.clone());
        }
    }
    let mut issues: Vec<String> = by_path
        .into_iter()
        .filter(|(_, fws)| distinct_frameworks(fws) > 1)
        .map(|(path, fws)| {
            format!(
                "Фреймворки «{}» создают один и тот же файл «{}» — такая связка сломает сгенерированный проект.",
                distinct_names(&fws),
                path
            )
        })
        .collect();
    issues.extend(by_dir.into_iter().filter(|(_, fws)| distinct_frameworks(fws) > 1).map(
        |(dir, fws)| {
            format!(
                "Фреймворки «{}» раскладывают каркасы в один каталог «{}» — второй скаффолдер сломает каркас первого.",
                distinct_names(&fws),
                dir.trim_start_matches("@dir:")
            )
        },
    ));
    issues.extend(
        by_manifest
            .into_iter()
            .filter(|(_, fws)| distinct_frameworks(fws) > 1)
            .map(|(path, fws)| {
                format!(
                    "Фреймворки «{}» патчат один и тот же манифест «{}» — патчи будут перезаписывать друг друга.",
                    distinct_names(&fws),
                    path
                )
            }),
    );
    if root_owners.len() > 1 {
        issues.push(format!(
            "Фреймворки «{}» оба скаффолдят корень проекта — вторая генерация сломает первую.",
            root_owners.join("» и «")
        ));
    }
    issues.sort();
    issues
}

/// Количество РАЗНЫХ фреймворков, претендующих на путь/каталог (один и тот
/// же фреймворк может писать один путь несколько раз — это не конфликт).
pub fn distinct_frameworks(fws: &[String]) -> usize {
    let mut unique = fws.to_vec();
    unique.sort();
    unique.dedup();
    unique.len()
}

/// Уникальные имена фреймворков в порядке их появления (для сообщения).
pub fn distinct_names(fws: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for fw in fws {
        if !seen.contains(&fw.as_str()) {
            seen.push(fw);
        }
    }
    seen.join("» и «")
}

