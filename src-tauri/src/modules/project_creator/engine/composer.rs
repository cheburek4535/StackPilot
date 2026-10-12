use super::*;
use chrono::Local;
use std::collections::HashSet;
use std::path::Path;

// ============================================================================
// Вспомогательные функции — их вы будете наполнять в TZ
// ============================================================================

/// Составить рецепт на основе WizardContext (TZ Task 1).
///
/// Шаги складываются в СТРОГУЮ очередь фаз (см. также `ExecutionPhase`):
///   1. Root Scaffolding — CLI фреймворков со scaffold="root" (tauri,
///      django, spring-boot, nest) запускаются ПЕРВЫМИ в корне проекта.
///      Движок не создаёт для них backend//frontend/ — корнем владеет CLI.
///   2. Subdir Scaffolding — сегменты моно-репозитория, language-скаффолды
///      (cargo init, package.json...) и CLI фреймворков со scaffold="subdir"
///      (create-vite, create-next-app, ...). Если корнем владеет root-скаффолд,
///      компаньоны (nextjs при nest) получают собственный сегмент frontend/
///      и выполняются ВНУТРИ него с аргументом "." — иначе CLI создаёт
///      вложенную папку <project_name>/ (матрешка testapp2/testapp2).
///   3. Установка инструментов — prisma init, alembic init, docker-compose
///      пишутся ПОСЛЕ каркасов (prisma требует package.json).
///   4. git init — чистим вложенные .git от генераторов и создаём корневой
///      репозиторий ДО шаблонизации.
///   5. ФИНАЛЬНАЯ шаблонизация — README.md, docker-compose.yaml, .gitignore
///      рендерятся ПОСЛЕ всех CLI-фреймворков и перезаписывают их версии
///      (overwrite=true), иначе create-next-app/nest new затирают шаблон.
///   6. Финализация — единственная установка зависимостей (npm install
///      ровно один раз на каждый JS-каталог, в самом конце) и стартовый
///      git add/commit со всеми готовыми файлами.
pub fn compose_recipe(
    layout: &ProjectLayout,
    context: &WizardContext,
    folder_name: &str,
    project_path: &Path,
) -> Result<Recipe, String> {
    // project_name может отличаться от folder_name (при auto-rename папки)
    let project_name = context.project_name.as_deref().unwrap_or(folder_name);
    // Неизвестные фреймворки/языки — явная ошибка, а не «echo-заглушка»:
    // движок обязан либо реализовать выбранную технологию, либо отказать с
    // понятной причиной. (html входит в wizard_tree.json, поэтому проходит
    // проверку; его echo-ветка — предмет отдельной ручной доработки.)
    for fw in &context.frameworks {
        if framework_def(fw).is_none() {
            return Err(format!(
                "Framework '{}' is not supported: no implementation is available in this build",
                fw
            ));
        }
    }
    for lang in &context.languages {
        if language_def(lang).is_none() {
            return Err(format!(
                "Language '{}' is not supported: no implementation is available in this build",
                lang
            ));
        }
    }
    let mut steps: Vec<Step> = Vec::new();
    let project_path = project_path.to_str().unwrap_or(".");

    steps.push(Step::CreateDirectory {
        id: "create_root".into(),
        label: "Create project root".into(),
        description: "Ensuring project directory exists".into(),
        path: ".".into(),
        condition: None,
        on_error: ErrorMode::Abort,
    });

    // Фаза 1: Root Scaffolding. Корнем владеет ровно тот, кому это назначила
    // каноническая раскладка (ProjectLayout::owns_root):
    //   - Integrated: обёртки над проектом (tauri — ScaffoldOwnership
    //     wraps_existing_project) — их шаги ОТЛОЖЕНЫ (см. deferred_wrappers
    //     ниже: фронтенд обязан скаффолдиться первым);
    //   - BackendOnly/FrontendOnly: root-скаффолдер (django, nest, spring-boot).
    // В Split корнем не владеет никто — даже scaffold="root" работает внутри
    // своего сегмента (django/nest в backend/).
    // Каталоги, в которых после всех CLI-каркасов нужен РОВНО ОДИН npm install
    // ("." = корень проекта). Скаффолдеры запускаются с --skip-install/
    // --no-install, поэтому node_modules не плодятся на каждом шаге.
    let mut js_dirs: Vec<String> = Vec::new();
    let mut rest_frameworks: Vec<String> = Vec::new();
    for fw in &context.frameworks {
        if layout.owns_root(fw) && !ScaffoldOwnership::for_framework(fw).wraps_existing_project {
            steps.extend(steps_for_framework(
                fw,
                project_path,
                project_name,
                context,
                layout,
            ));
            // Root-JS-фреймворк (nest): работает в корне с --skip-install,
            // его package.json ставится один раз в финальной фазе.
            if is_js_framework(fw) {
                push_unique(&mut js_dirs, ".".to_string());
            }
        } else {
            rest_frameworks.push(fw.clone());
        }
    }

    // Фаза 2: Subdir Scaffolding. Сегменты моно-репозитория (backend + frontend)
    // создаются ТОЛЬКО в split-раскладке (eager-папки); integrated и
    // одно-сторонние раскладки папки не предсоздают — их создают сами
    // генераторы (ScaffoldGenerator) или WriteFile.
    for dir in layout.eager_dirs() {
        steps.push(Step::CreateDirectory {
            id: format!("create_{}_dir", dir),
            label: format!("Create {}/", dir),
            description: format!("Create {} directory for frameworks", dir),
            path: dir.clone(),
            condition: None,
            on_error: ErrorMode::Abort,
        });
    }

    for lang in &context.languages {
        // Фреймворк сам создаёт каркас для этого языка (aspnetcore вместо
        // dotnet new console, nextjs вместо js-скаффолда, tauri вместо
        // cargo init) — generic-шаги языка не нужны и конфликтуют с
        // файлами фреймворка.
        if language_scaffold_suppressed(lang, layout, context) {
            continue;
        }
        let lang_seg = layout.language_dir(lang);
        let mut lang_steps = steps_for_language(lang, project_name, project_path, context);
        if let Some(dir) = &lang_seg {
            lang_steps = into_segment(lang_steps, dir);
        }
        steps.extend(lang_steps);
        // JS-язык без фреймворка-каркаса: package.json ляжет в этот каталог —
        // там нужен финальный npm install.
        if matches!(lang.to_lowercase().as_str(), "typescript" | "javascript") {
            push_unique(&mut js_dirs, lang_seg.unwrap_or_else(|| ".".to_string()));
        }
    }

    // ========================================================================
    // Generic toolchain preflight (preflight.rs): единый слой проверки
    // инструментария вместо пер-фреймворковых патчей.
    //
    // Node: node/npm-префлайт ДО любого npm-скаффолда — отсутствие node/npm
    // останавливает пайплайн, каркасы не «молча пропускаются».
    // ========================================================================
    if context_needs_npm(context) {
        steps.extend(preflight::node_preflight_steps(project_path));
    }

    // Generic host-tool preflight (where/which): dart, cargo, go, dotnet,
    // zig, mix, maven, gradle... — отсутствие обязательного инструмента
    // останавливает пайплайн ДО скаффолдов (Abort), а не валит каркас
    // серединой генерации. node/npm/python/php покрыты отдельно.
    steps.extend(preflight::host_tool_preflight_steps(context));

    // ========================================================================
    // Python: ЕДИНСТВЕННЫЙ канонический venv (по python_segment_dir →
    // ProjectLayout) для всех Python-проектов. Создаётся один раз, маркер
    // проверяется, pip бутстрапится интерпретатором venv, манифест
    // (requirements.txt, записан language-скаффолдом выше) устанавливается
    // РОВНО один раз ДО любых framework-шагов — django-admin/alembic идут
    // только через этот venv. Отдельного «django-venv» больше нет.
    // ========================================================================
    if context.languages.iter().any(|l| l == "python") {
        let python_dir = python_segment_dir(context);
        steps.push(preflight::python_preflight_step(project_path));
        steps.extend(preflight::python_environment_steps(
            project_path,
            &python_dir,
        ));
        // Пост-валидация манифеста: requirements.txt обязан содержать все
        // выбранные зависимости фреймворков и инструментов.
        let manifest_entries = preflight::python_manifest_entries(context);
        let manifest_refs: Vec<&str> = manifest_entries.iter().map(String::as_str).collect();
        steps.push(preflight::manifest_check_step(
            "py_requirements_check",
            "Validate Python requirements manifest",
            &preflight::requirements_path(&python_dir),
            "requirements_txt",
            &manifest_refs,
        ));
    }

    // Обёртки над проектом (ScaffoldOwnership::wraps_existing_project: tauri)
    // откладываются в конец фазы Subdir Scaffolding: пайплайн обёртки обязан
    // выполнять фронтенд-генератор ПЕРВЫМ (vite в frontend/ → npm install →
    // tauri init), иначе init опережает каркас фронтенда.
    // Побочные фреймворки (kind="side": telegraf, aiogram...) выполняются
    // ПОСЛЕ главных (nest, django...): их шаги пишут поверх/патчат каркас
    // главного фреймворка (telegraf → dep-патч package.json, созданного
    // nest), поэтому стабильная перестановка «app-сначала, side-в-конец»
    // обязательна независимо от порядка карточек в мастере.
    let mut main_fws: Vec<String> = Vec::new();
    let mut side_fws: Vec<String> = Vec::new();
    for fw in rest_frameworks {
        if framework_def(&fw).is_some_and(|d| d.kind == "side") {
            side_fws.push(fw);
        } else {
            main_fws.push(fw);
        }
    }
    // Каркас с СОБСТВЕННЫМ фронтендом (electron/renderer, flutter/dart-ui):
    // его UI встроен в каркас, поэтому универсальные UI-компаньоны
    // (react/vue/svelte) рядом с ним не скаффолдятся отдельно — иначе
    // получились бы два несвязанных фронтенда в frontend/ (запрещено).
    // Обёртки (tauri) и C++-каркасы (qt) фронтенд НЕ создают — компаньоны
    // для них скаффолдятся штатно.
    let frontend_shell_present = context
        .frameworks
        .iter()
        .any(|fw| creates_frontend_shell(fw));
    let mut deferred_wrappers: Vec<Step> = Vec::new();
    for fw in main_fws.iter().chain(side_fws.iter()) {
        let ownership = ScaffoldOwnership::for_framework(fw);
        // UI-компаньоны подавляются только при наличии ЧУЖОГО владельца
        // фронтенда: сам по себе react/vue/svelte — обычный каркас.
        if frontend_shell_present && ownership.is_ui_companion {
            continue;
        }
        let fw_steps = steps_for_framework(fw, project_path, project_name, context, layout);
        if is_js_framework(fw) && !fw_steps.is_empty() {
            // Scaffold-фреймворки (Step::Generate "scaffold") кладут
            // package.json в scaffold_target_dir (frontend/), остальные —
            // в сегмент или подпапку <project_name>. Проверка СТРУКТУРНАЯ:
            // по фактическим шагам фреймворка, а не по списку id.
            let dir = if fw_steps.iter().any(
                |s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"),
            ) {
                scaffold_target_dir(fw, layout.framework_dir(fw).as_deref())
            } else {
                layout
                    .framework_dir(fw)
                    .unwrap_or_else(|| project_name.to_string())
            };
            push_unique(&mut js_dirs, dir);
        }
        if ownership.wraps_existing_project {
            // Обёртка инициализируется ПОСЛЕ всех фронтенд-каркасов
            // (см. выше: фронтенд-генератор FIRST → npm install → обёртка).
            deferred_wrappers = fw_steps;
        } else {
            steps.extend(fw_steps);
        }
    }
    steps.extend(deferred_wrappers);

    // Фаза 3: установка инструментов (prisma init требует существующий
    // package.json — выполняется строго после каркасов).
    steps.extend(steps_for_tools(context, project_path));

    // Фаза 4: git init — ДО шаблонизации: убираем вложенные .git, созданные
    // генераторами, и инициализируем корневой репозиторий.
    steps.extend(steps_for_git_init(context, project_path));

    // Фаза 5: ФИНАЛЬНАЯ шаблонизация — ПОСЛЕ выполнения ВСЕХ CLI-фреймворков.
    // README.md, docker-compose.yaml и .gitignore, созданные самими CLI
    // (create-next-app, nest new...), перезаписываются нашими шаблонами
    // (overwrite=true) — иначе шаблон молча теряется.
    steps.extend(steps_for_docker(
        layout,
        context,
        project_path,
        project_name,
    ));
    steps.extend(steps_for_gitignore(context, project_path));
    steps.extend(steps_for_ci(layout, context, project_path, project_name));
    steps.extend(steps_for_readme(
        layout,
        context,
        project_path,
        project_name,
    ));
    steps.extend(steps_for_vscode(layout, context));

    // Слияние вложенных .vscode (frontend/.vscode, backend/.vscode) в корневой
    // .vscode/ с удалением вложенных папок — ПОСЛЕ всех CLI-скаффолдеров и
    // vscode-merge, чтобы конфиги CLI (typescript.tsdk и т.п.) не потерялись.
    // Генератор "vscode-folders" — no-op, если вложенных .vscode нет.
    steps.push(Step::Generate {
        id: "merge_inner_vscode".into(),
        label: "Merge inner .vscode folders".into(),
        description: "Merge frontend/.vscode and backend/.vscode into the root .vscode/".into(),
        generator_id: "vscode-folders".into(),
        generator_config: serde_json::json!({}),
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    // Фаза 6: финализация — единственная установка зависимостей в самом
    // конце (npm install ровно один раз на JS-каталог) и стартовый
    // git add/commit со всеми готовыми файлами.
    steps.extend(steps_for_finalize(
        context,
        &js_dirs,
        project_path,
        project_name,
    ));

    // ========================================================================
    // Явные предусловия шагов (см. StepDependency). Зависимости НЕ выводятся
    // из порядка шагов — каждая пара декларируется явно; план нормализуется
    // в plan() (topo_order_steps: стабильная топологическая сортировка,
    // циклы/самозависимости/висячие предшественники отклоняются).
    // Присутствие пар фильтруется в build_dependencies по фактическому плану:
    // ветки, отсечённые контекстом (компаньон вместо tauri_web_scaffold,
    // отсутствующий nest и т.п.), не дают «висячих» предшественников.
    // ========================================================================
    let has_python = context.languages.iter().any(|l| l == "python");
    let has_django = context.frameworks.iter().any(|f| f == "django");
    let py_dir = python_segment_dir(context);
    // Маркер канонического venv — ТОЧНО как в py_venv_create (preflight.rs):
    // создаётся один раз, зависимые verify/pip-шаги выполняются всегда
    // (пост-условие на месте, даже когда venv переиспользован).
    let venv_marker = preflight::venv_marker_rel(&py_dir);
    let dep = |step: &str, prereq: &str| StepDependency {
        step_id: step.to_string(),
        prereq_id: prereq.to_string(),
        expects_file: String::new(),
    };
    let dep_file = |step: &str, prereq: &str, file: &str| StepDependency {
        step_id: step.to_string(),
        prereq_id: prereq.to_string(),
        expects_file: file.to_string(),
    };
    let mut dependencies: Vec<StepDependency> = Vec::new();

    // NestJS: пост-патч имени package.json (nest_pkg_name) и telegraf-патч
    // (telegraf_pkg_patch) читают package.json, созданный nest_new.
    if context.frameworks.iter().any(|f| f == "nest") {
        dependencies.push(dep("nest_pkg_name", "nest_new"));
        dependencies.push(dep("telegraf_pkg_patch", "nest_new"));
    }

    // Tauri: config-патч — после tauri init; пост-патч имени package.json и
    // npm install — после фронтенд-каркаса. Каркас создаёт ИЛИ
    // tauri_web_scaffold (без компаньона), ИЛИ vite_create (с компаньоном) —
    // декларируются оба, build_dependencies оставит выжившего.
    if context.frameworks.iter().any(|f| f == "tauri") {
        dependencies.push(dep("tauri_config_patch", "tauri_init"));
        dependencies.push(dep("tauri_pkg_name", "tauri_web_scaffold"));
        dependencies.push(dep("tauri_pkg_name", "vite_create"));
        dependencies.push(dep("tauri_web_install", "tauri_web_scaffold"));
    }

    // Qt WebEngine: сборка веб-части — после vite_create; cmake-конфигурация —
    // после CMakeLists.txt (qt_cmake); cmake-сборка — после конфигурации и
    // собранного фронтенда. У qt_cmake_build ДВА предшественника, поэтому
    // expects_file для qt_web_build задаётся явно (условие шага — пост-условие
    // только этого предшественника, авто-вывод в build_dependencies отключён
    // для множественных деклараций).
    if qt_ui_mode(context) == "webengine" {
        dependencies.push(dep("qt_web_build", "vite_create"));
        dependencies.push(dep("qt_cmake_configure", "qt_cmake"));
        dependencies.push(dep_file(
            "qt_cmake_build",
            "qt_web_build",
            "frontend/dist/index.html",
        ));
        dependencies.push(dep("qt_cmake_build", "qt_cmake_configure"));
    }

    // Go: go get / cobra работают в каталоге модуля — строго после
    // go mod init (language-фаза), иначе «go.mod file not found».
    if context.frameworks.iter().any(|f| f == "gin") {
        dependencies.push(dep("get_gin", "go_mod_init"));
    }
    if context.frameworks.iter().any(|f| f == "echo") {
        dependencies.push(dep("get_echo", "go_mod_init"));
    }
    if context.frameworks.iter().any(|f| f == "fiber") {
        dependencies.push(dep("get_fiber", "go_mod_init"));
    }
    if context.frameworks.iter().any(|f| f == "cobra") {
        dependencies.push(dep("get_cobra", "go_mod_init"));
    }

    // Python: pip/alembic/django-admin обязаны видеть готовое каноническое
    // окружение. py_venv_create создаёт маркер один раз; py_venv_verify
    // проверяет его; pip-шаги ВЫПОЛНЯЮТСЯ даже когда venv переиспользован
    // (маркер на месте — FileNotExists-условие скипает только создание).
    // django_start (root-скаффолд, объявлен раньше по фазе) топологически
    // переносится ПОСЛЕ установки манифеста.
    if has_python {
        dependencies.push(dep_file("py_venv_verify", "py_venv_create", &venv_marker));
        dependencies.push(dep_file("py_pip_upgrade", "py_venv_create", &venv_marker));
        dependencies.push(dep_file("py_pip_check", "py_venv_create", &venv_marker));
        dependencies.push(dep("py_pip_install", "py_pip_upgrade"));
        dependencies.push(dep("py_pip_install", "py_pip_check"));
        dependencies.push(dep("py_requirements_check", "py_pip_install"));
        if context.tools.iter().any(|t| t == "alembic") {
            dependencies.push(dep("alembic_init", "py_pip_install"));
            // Патч env.py применяется только после успешного alembic init
            // (без migrations/env.py шаг и так пропускается по условию).
            dependencies.push(dep("alembic_env_patch", "alembic_init"));
        }
        if has_django {
            dependencies.push(dep("django_start", "py_pip_install"));
        }
    }

    // Пост-валидация package.json: check-шаги читают каркас, созданный
    // scaffold-генератором соответствующего JS-фреймворка (nest_new,
    // vite_create и т.п.). Пары фильтруются по фактическому плану в
    // build_dependencies — ветки без каркаса не дают висячих зависимостей.
    for fw in &context.frameworks {
        if framework_npm_dependency(fw).is_some() {
            if let Some(scaffold_id) = scaffold_step_id_for(fw) {
                dependencies.push(dep(&format!("{}_pkg_check", fw), scaffold_id));
            }
        }
    }
    // Prisma: dep-патч package.json строго до prisma init (init читает
    // манифест и добавляет собственные записи).
    if context.tools.iter().any(|t| t == "prisma") {
        dependencies.push(dep("prisma_init", "prisma_deps"));
    }

    // Gradle-каркасы: пост-валидация manifest-файлов строго ПОСЛЕ генерации
    // wrapper'а — иначе gradlew/build-файлы могут ещё не существовать.
    if context.frameworks.iter().any(|f| f == "ktor") {
        dependencies.push(dep("ktor_deps_check", "ktor_gradle_wrapper"));
    }
    if context
        .frameworks
        .iter()
        .any(|f| f == "android" || f == "jetpack-compose")
    {
        dependencies.push(dep("android_build_check", "android_gradle_wrapper"));
    }
    // Terraform: init обязан выполниться до пост-валидации main.tf (проверка
    // не зависит от init, но порядок фиксирует контракт шагов).
    if context.tools.iter().any(|t| t == "terraform") {
        dependencies.push(dep("terraform_check", "terraform_init"));
    }

    Ok(Recipe {
        id: format!("recipe_{}", project_name),
        name: format!("{:?} project", context.project_type),
        description: format!("Full setup for {} project", project_name),
        tags: context.languages.clone(),
        steps,
        dependencies,
    })
}

/// Проекту нужен node/npm (язык JS/TS или фреймворк на них) — перед любым
/// npm-скаффолдом выполняется общий node/npm-префлайт.
pub fn context_needs_npm(context: &WizardContext) -> bool {
    context
        .languages
        .iter()
        .any(|l| matches!(l.as_str(), "typescript" | "javascript"))
        || context.frameworks.iter().any(|f| is_js_framework(f))
}

/// npm-пакет, которым обязан обладать package.json каркаса фреймворка
/// (для пост-валидации manifest-check). None — пакет неочевиден/нет каркаса
/// (tauri).
pub fn framework_npm_dependency(fw: &str) -> Option<&'static str> {
    match fw {
        "react" => Some("react"),
        "vue" => Some("vue"),
        "svelte" => Some("svelte"),
        "nextjs" => Some("next"),
        "sveltekit" => Some("@sveltejs/kit"),
        "nuxt" => Some("nuxt"),
        "solidjs" => Some("@solidjs/start"),
        "electron" => Some("electron"),
        "expo" => Some("expo"),
        "nest" => Some("@nestjs/core"),
        "express" => Some("express"),
        "fastify" => Some("fastify"),
        "telegraf" => Some("telegraf"),
        // RN-каркас декларирует react-native в dependencies, плазменный —
        // plasmo в devDependencies (проверка сканирует оба раздела).
        "react-native" => Some("react-native"),
        "plasmo" => Some("plasmo"),
        "angular" => Some("@angular/core"),
        "vite" => Some("vite"),
        "astro" => Some("astro"),
        "remix" => Some("@remix-run/react"),
        "adonisjs" => Some("@adonisjs/core"),
        _ => None,
    }
}

/// Step-id скаффолда, создающего package.json для фреймворка (для
/// декларации зависимости `<fw>_pkg_check` после него).
pub fn scaffold_step_id_for(fw: &str) -> Option<&'static str> {
    match fw {
        "adonisjs" => Some("adonisjs_create"),
        "nest" => Some("nest_new"),
        "nextjs" => Some("nextjs_create"),
        "nuxt" => Some("nuxt_create"),
        "sveltekit" => Some("sveltekit_create"),
        "react" | "vue" | "svelte" | "vite" => Some("vite_create"),
        "angular" => Some("angular_create"),
        "astro" => Some("astro_create"),
        "remix" => Some("remix_create"),
        "electron" => Some("electron_init"),
        "expo" => Some("expo_init"),
        "solidjs" => Some("solid_init"),
        "react-native" => Some("rn_init"),
        "plasmo" => Some("plasmo_init"),
        _ => None,
    }
}

/// Путь package.json относительно корня проекта для рабочего каталога
/// пост-шага (package_name_patch_step / pkg_check).
pub fn package_json_rel_path(workdir: Option<&str>) -> String {
    match workdir {
        Some(wd) if !wd.is_empty() && wd != "." => format!("{}/package.json", wd),
        _ => "package.json".to_string(),
    }
}

/// Добавить значение в список, если его там ещё нет.
pub fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// JS/TS-фреймворки: создают package.json и нуждаются в npm install.
pub fn is_js_framework(fw: &str) -> bool {
    framework_def(fw).is_some_and(|def| {
        def.languages
            .iter()
            .any(|l| l == "typescript" || l == "javascript")
    })
}

/// Рекурсивно развернуть Parallel и отфильтровать шаги по condition.
pub fn flatten_and_filter(recipe: &Recipe, context: &WizardContext, _project_path: &Path) -> Vec<Step> {
    flatten_steps(&recipe.steps, context)
}

pub fn flatten_steps(steps: &[Step], context: &WizardContext) -> Vec<Step> {
    let mut result = Vec::new();
    fn append(steps: &[Step], context: &WizardContext, result: &mut Vec<Step>) {
        for step in steps {
            if !evaluate_condition(step.condition(), context) {
                continue;
            }
            if let Step::Parallel { steps: inner, .. } = step {
                append(inner, context, result);
            } else {
                result.push(step.clone());
            }
        }
    }
    append(steps, context, &mut result);
    result
}

pub fn evaluate_condition(cond: Option<&StepCondition>, context: &WizardContext) -> bool {
    let Some(cond) = cond else { return true };
    match cond {
        StepCondition::Always => true,
        StepCondition::ContextHas { key, value } => match key.as_str() {
            "language" => context.languages.iter().any(|l| l == value),
            "framework" => context.frameworks.iter().any(|f| f == value),
            "tool" => context.tools.iter().any(|t| t == value),
            // Булевы фичи: значение обязано быть "true"/"false" — флаг
            // сравнивается буквально. (Раньше таких ключей здесь не было —
            // ContextHas всегда возвращал false для них.)
            "docker" => {
                (value == "true" && context.docker) || (value == "false" && !context.docker)
            }
            "testing" => {
                (value == "true" && context.testing) || (value == "false" && !context.testing)
            }
            "git_init" => {
                (value == "true" && context.git_init) || (value == "false" && !context.git_init)
            }
            "vscode_config" => {
                (value == "true" && context.vscode_config)
                    || (value == "false" && !context.vscode_config)
            }
            "ci" => (value == "true" && context.ci) || (value == "false" && !context.ci),
            _ => false,
        },
        StepCondition::ContextMissing { key } => match key.as_str() {
            // «Ключ отсутствует» для списков — пустой список (нет НИ ОДНОГО
            // языка/фреймворка/инструмента), а не «нет элемента с пустой
            // строкой»: прежняя семантика делала ContextMissing всегда
            // истинным для булевых ключей и бессмысленным для списков.
            "language" => context.languages.is_empty(),
            "framework" => context.frameworks.is_empty(),
            "tool" => context.tools.is_empty(),
            "docker" => !context.docker,
            "testing" => !context.testing,
            "git_init" => !context.git_init,
            "vscode_config" => !context.vscode_config,
            "ci" => !context.ci,
            _ => false,
        },
        StepCondition::FileExists { .. } | StepCondition::FileNotExists { .. } => {
            true // план: файл ещё не создан — шаг показывается в превью;
                 // фактическая проверка — в runtime_condition() при выполнении
        }
        StepCondition::TechnologyDetected { name } => {
            context.tools.contains(name) || context.frameworks.contains(name)
        }
        StepCondition::TechnologyNotDetected { name } => {
            !context.tools.contains(name) && !context.frameworks.contains(name)
        }
        StepCondition::FeatureEnabled { feature } => match feature.as_str() {
            "docker" => context.docker,
            "testing" => context.testing,
            "git_init" => context.git_init,
            "vscode_config" => context.vscode_config,
            "ci" => context.ci,
            _ => false,
        },
    }
}

/// Runtime-проверка условия по ФАКТИЧЕСКОМУ состоянию файловой системы
/// проекта (пост-условия скаффолда). Контекстные условия сюда не попадают:
/// они отфильтрованы plan-time в flatten_steps. Пути условий — относительные
/// к корню проекта; небезопасный путь (абсолютный, `..` вне корня) трактуется
/// как невыполненное условие (шаг пропускается — писать/запускать нечего).
pub fn runtime_condition(cond: Option<&StepCondition>, project_path: &Path) -> bool {
    let Some(cond) = cond else { return true };
    match cond {
        StepCondition::FileExists { path } => paths::resolve_in_root(project_path, path)
            .map(|p| p.exists())
            .unwrap_or(false),
        StepCondition::FileNotExists { path } => {
            // Безопасный путь: файл не существует (проверка по нормализованному
            // пути). Недопустимый путь — условие невыполнимо (skip).
            match paths::resolve_in_root(project_path, path) {
                Some(p) => !p.exists(),
                None => false,
            }
        }
        _ => true,
    }
}

/// Привести декларации зависимостей рецепта (Recipe.dependencies) к плану:
/// 1. Отбросить пары, чьи шаги отсутствуют в развёрнутом (flattened) плане —
///    ветки, отсечённые контекстными условиями (компаньон вместо
///    tauri_web_scaffold, отсутствующий nest и т.п.), не порождают
///    «висячих» предшественников.
/// 2. Вывести expects_file из собственного FileExists-условия зависимого
///    шага, если пост-условие не задано явно — НО только когда у зависимого
///    шага ровно ОДНА выжившая декларация: одно условие нельзя однозначно
///    приписать конкретному предшественнику из нескольких (qt_cmake_build
///    зависит и от qt_web_build, и от qt_cmake_configure, а его условие —
///    пост-условие только первого). Дублирующиеся пары (одна и та же
///    dependent→prereq) схлопываются в одну.
pub fn build_dependencies(steps: &[Step], declared: &[StepDependency]) -> Vec<StepDependency> {
    let ids: std::collections::HashSet<String> = steps.iter().map(|s| s.id()).collect();
    let mut survivors: Vec<StepDependency> = Vec::new();
    for dep in declared {
        if !ids.contains(&dep.step_id) {
            continue;
        }
        if !dep.prereq_id.is_empty() && !ids.contains(&dep.prereq_id) {
            continue;
        }
        if survivors
            .iter()
            .any(|s| s.step_id == dep.step_id && s.prereq_id == dep.prereq_id)
        {
            continue;
        }
        survivors.push(dep.clone());
    }
    let mut per_dependent: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for dep in &survivors {
        *per_dependent.entry(dep.step_id.clone()).or_insert(0) += 1;
    }
    survivors
        .into_iter()
        .map(|mut dep| {
            if dep.expects_file.is_empty() && per_dependent.get(&dep.step_id).copied() == Some(1) {
                dep.expects_file = steps
                    .iter()
                    .find(|s| s.id() == dep.step_id)
                    .and_then(|s| s.condition())
                    .and_then(|c| match c {
                        StepCondition::FileExists { path } => Some(path.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
            }
            dep
        })
        .collect()
}

/// Построить план в топологическом порядке: каждый предшественник (явное
/// предусловие) идёт СТРОГО до зависимого шага. Сортировка СТАБИЛЬНА —
/// независимые шаги сохраняют порядок декларации рецепта, поэтому рецепт
/// может декларировать шаги в любом порядке. Отклоняются только
/// по-настоящему невалидные планы: дубликаты id шагов, висячие
/// предшественники, самозависимости и циклы (с полным путём цикла в
/// ошибке). Зависимости НЕ выводятся из порядка шагов — только из
/// деклараций (Recipe.dependencies).
pub fn topo_order_steps(steps: &[Step], deps: &[StepDependency]) -> Result<Vec<Step>, String> {
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, step) in steps.iter().enumerate() {
        if index.insert(step.id(), i).is_some() {
            return Err(format!(
                "Dependency plan error: duplicate step id '{}' in the plan",
                step.id()
            ));
        }
    }
    // Граф «зависимый → предшественник». Чисто файловые предусловия
    // (prereq_id пуст) порядок не определяют — они проверяются только
    // в dependency_skip_reason во время выполнения.
    let mut indegree = vec![0usize; steps.len()];
    let mut prereqs_of: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    let mut dependents_of: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    for dep in deps {
        let dep_idx = *index.get(&dep.step_id).ok_or_else(|| {
            format!(
                "Dependency plan error: step '{}' is not in the plan",
                dep.step_id
            )
        })?;
        if dep.prereq_id.is_empty() {
            continue;
        }
        let pre_idx = *index.get(&dep.prereq_id).ok_or_else(|| {
            format!(
                "Dependency plan error: prerequisite '{}' of '{}' is not in the plan",
                dep.prereq_id, dep.step_id
            )
        })?;
        if pre_idx == dep_idx {
            return Err(format!(
                "Dependency plan error: step '{}' cannot depend on itself",
                dep.step_id
            ));
        }
        indegree[dep_idx] += 1;
        prereqs_of[dep_idx].push(pre_idx);
        dependents_of[pre_idx].push(dep_idx);
    }

    // Стабильный алгоритм Кана: на каждом шаге берётся ПЕРВЫЙ (в порядке
    // декларации) шаг без неудовлетворённых предшественников.
    let mut remaining: Vec<usize> = (0..steps.len()).collect();
    let mut ordered: Vec<Step> = Vec::with_capacity(steps.len());
    while let Some(pos) = remaining.iter().position(|&i| indegree[i] == 0) {
        let i = remaining.remove(pos);
        ordered.push(steps[i].clone());
        for &dependent in &dependents_of[i] {
            indegree[dependent] -= 1;
        }
    }

    if !remaining.is_empty() {
        // Остались только шаги, чьи предшественники не были разблокированы:
        // среди них гарантированно есть настоящий цикл. Извлекаем его полный
        // путь и замыкаем на первом узле: «A -> B -> A».
        let cycle = cycle_path(steps, &remaining, &prereqs_of);
        let mut closed = cycle.clone();
        if let Some(first) = cycle.first() {
            closed.push(first.clone());
        }
        return Err(format!(
            "Dependency cycle detected: {}",
            closed.join(" -> ")
        ));
    }
    Ok(ordered)
}

/// Найти фактический цикл среди оставшихся (не упорядоченных) шагов:
/// глубина по рёбрам «зависимый → предшественник» до возврата в узел,
/// уже находящийся на стеке. Возвращает путь цикла в порядке следования
/// рёбер (без замыкания на первый узел).
pub fn cycle_path(steps: &[Step], remaining: &[usize], prereqs_of: &[Vec<usize>]) -> Vec<String> {
    let in_remaining: std::collections::HashSet<usize> = remaining.iter().copied().collect();
    let mut state: Vec<u8> = vec![0; steps.len()];
    let mut stack: Vec<usize> = Vec::new();
    fn dfs(
        node: usize,
        steps: &[Step],
        prereqs_of: &[Vec<usize>],
        in_remaining: &std::collections::HashSet<usize>,
        state: &mut Vec<u8>,
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[node] = 1;
        stack.push(node);
        for &pre in &prereqs_of[node] {
            if !in_remaining.contains(&pre) {
                continue;
            }
            match state[pre] {
                0 => {
                    if let Some(cycle) = dfs(pre, steps, prereqs_of, in_remaining, state, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    let pos = stack
                        .iter()
                        .position(|&x| x == pre)
                        .expect("DFS invariant: state[pre] == 1 implies pre is on the stack");
                    return Some(stack[pos..].to_vec());
                }
                _ => {}
            }
        }
        state[node] = 2;
        stack.pop();
        None
    }
    for &node in remaining {
        if state[node] == 0 {
            if let Some(cycle) = dfs(
                node,
                steps,
                prereqs_of,
                &in_remaining,
                &mut state,
                &mut stack,
            ) {
                return cycle.iter().map(|&i| steps[i].id()).collect();
            }
        }
    }
    // Недостижимо: раз сортировка не завершилась, среди оставшихся шагов
    // гарантированно есть цикл.
    remaining.iter().map(|&i| steps[i].id()).collect()
}

/// Причина пропуска шага из-за явных предусловий (dependency rules).
/// Возвращает None, когда шаг может выполняться. Проверяет ТОЛЬКО статусы
/// предшественников (в результатах) и файловые пост-условия по фактической
/// файловой системе проекта:
///   - предшественник провалился (Failed) → пропуск с его ошибкой;
///   - предшественник пропущен и пост-условия нет (или оно отсутствует на
///     диске) → пропуск; ИСКЛЮЧЕНИЕ: django-ранний venv — py_venv_create
///     пропущен (маркер venv/pyvenv.cfg уже есть), а py_pip_upgrade
///     выполняется: пост-условие НА МЕСТЕ;
///   - предшественник успешен, но пост-условие отсутствует → предшественник
///     ретроактивно помечается Failed (с путём и рабочей директорией),
///     зависимый шаг пропускается;
///   - чистое файловое предусловие (без предшественника): файл отсутствует
///     → пропуск.
pub fn dependency_skip_reason(
    step: &Step,
    dependencies: &[StepDependency],
    results: &mut Vec<StepResult>,
    project_path: &Path,
) -> Option<String> {
    for dep in dependencies.iter().filter(|d| d.step_id == step.id()) {
        let prereq_result = if dep.prereq_id.is_empty() {
            None
        } else {
            results.iter().rev().find(|r| r.step_id == dep.prereq_id)
        };
        // Файловое пост-условие проверяется по безопасному пути строго
        // внутри корня проекта (те же правила, что у WriteFile/условий):
        // недопустимый путь (абсолютный, `..` за корень) трактуется как
        // отсутствующий файл — шаг пропускается.
        let file_missing = !dep.expects_file.is_empty()
            && !paths::resolve_in_root(project_path, &dep.expects_file)
                .map(|p| p.exists())
                .unwrap_or(false);
        match prereq_result {
            Some(r) => match &r.status {
                StepStatus::Failed { error } => {
                    return Some(format!(
                        "Skipped: prerequisite '{}' failed: {}",
                        dep.prereq_id, error
                    ));
                }
                StepStatus::Skipped { .. } => {
                    if dep.expects_file.is_empty() {
                        return Some(format!(
                            "Skipped: prerequisite '{}' was skipped",
                            dep.prereq_id
                        ));
                    }
                    if file_missing {
                        return Some(format!(
                            "Skipped: '{}' was not created by skipped prerequisite '{}'",
                            dep.expects_file, dep.prereq_id
                        ));
                    }
                    // Предшественник пропущен, но пост-условие на месте
                    // (django-ранний venv) — зависимый шаг выполняется.
                }
                StepStatus::Success { .. } => {
                    if file_missing {
                        // Ретроактивная пометка: команда «успешно» завершилась,
                        // но обещанного файла нет — это ошибка предшественника.
                        if let Some(prereq) = results
                            .iter_mut()
                            .rev()
                            .find(|r| r.step_id == dep.prereq_id)
                        {
                            prereq.status = StepStatus::Failed {
                                error: format!(
                                    "Command completed but expected file '{}' was not created (working directory: {})",
                                    dep.expects_file,
                                    project_path.display()
                                ),
                            };
                        }
                        return Some(format!(
                            "Skipped: expected output '{}' was not created by '{}'",
                            dep.expects_file, dep.prereq_id
                        ));
                    }
                }
                _ => {}
            },
            None => {
                // Чистое файловое предусловие (нет предшественника): файл
                // обязан существовать на момент запуска шага.
                if file_missing {
                    return Some(format!(
                        "Skipped: required file '{}' does not exist (project root)",
                        dep.expects_file
                    ));
                }
            }
        }
    }
    None
}

pub fn chrono_event_time() -> String {
    let local_time = Local::now();
    local_time.format("%H::%M:%S").to_string()
}

pub fn steps_for_language(
    lang: &str,
    project_name: &str,
    project_path: &str,
    context: &WizardContext,
) -> Vec<Step> {
    let split_command = |line: &str| -> (String, Vec<String>) {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut quote = None;
        for ch in line.chars() {
            match (quote, ch) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => current.push(c),
                (None, '\'' | '"') => quote = Some(ch),
                (None, c) if c.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                (None, c) => current.push(c),
            }
        }
        if !current.is_empty() {
            words.push(current);
        }
        let mut iter = words.into_iter();
        let program = iter.next().unwrap_or_else(|| "echo".to_string());
        (program, iter.collect())
    };

    // Вспомогательная функция для команды с рабочей директорией
    let cmd = |id: &str, label: &str, desc: &str, command: &str| -> Step {
        let (program, args) = split_command(command);
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: program,
            args,
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(60),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    };

    // Команда инициализации языка + FileNotExists-условие на маркерный файл,
    // который она создаёт: повторный запуск рецепта НЕ пересоздаёт проект
    // (cargo init, go mod init и т.п. уже отработали в прошлый раз).
    let cmd_once = |id: &str, label: &str, desc: &str, command: &str, marker: &str| -> Step {
        let mut step = cmd(id, label, desc, command);
        match &mut step {
            Step::Command { condition, .. } => {
                *condition = Some(StepCondition::FileNotExists {
                    path: marker.to_string(),
                });
            }
            _ => unreachable!("cmd() always builds Step::Command"),
        }
        step
    };

    // Создание директории — частая операция
    let mkdir = |id: &str, path: &str| -> Step {
        Step::CreateDirectory {
            id: id.to_string(),
            label: format!("Create {}", path),
            description: format!("Create {} directory", path),
            path: path.to_string(),
            condition: None,
            on_error: ErrorMode::Abort,
        }
    };

    match lang.to_lowercase().as_str() {
        "rust" => vec![
            cmd_once("cargo_init", "Init Cargo project",
                &format!("Initialize new Rust project '{}'", project_name),
                &format!("cargo init --name {}", project_name),
                "Cargo.toml"),
        ],

        "typescript" | "javascript" => {
            let mut steps = vec![
                mkdir("create_src", "src"),
                // package.json с базовыми полями
                Step::WriteFile {
                    id: "package_json".into(),
                    label: "Create package.json".into(),
                    description: "Initialize package.json with project metadata".into(),
                    path: "package.json".into(),
                    content: format!(
    r#"{{
  "name": "{}",
  "version": "1.0.0",
  "description": "",
  "main": "src/index.{}",
  "scripts": {{
    "start": "node src/index.{}",
    "dev": "node --watch src/index.{}"
  }},
  "keywords": [],
  "author": "",
  "license": "ISC"
}}"#,
                        project_name,
                        if lang == "typescript" { "ts" } else { "js" },
                        if lang == "typescript" { "ts" } else { "js" },
                        if lang == "typescript" { "ts" } else { "js" }
                    ),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ];

            // Для TypeScript добавляем tsconfig.json и инициализацию
            if lang == "typescript" {
                steps.push(cmd_once("tsc_init", "Init TypeScript",
                    "Generate tsconfig.json",
                    "npx -p typescript tsc --init --target ES2022 --module commonjs --outDir dist --rootDir src",
                    "tsconfig.json"));
                steps.push(Step::WriteFile {
                    id: "ts_src_index".into(),
                    label: "Create src/index.ts".into(),
                    description: "Create entry point for TypeScript".into(),
                    path: "src/index.ts".into(),
                    content: "console.log('Hello from TypeScript!');\n".into(),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            } else {
                steps.push(Step::WriteFile {
                    id: "js_src_index".into(),
                    label: "Create src/index.js".into(),
                    description: "Create entry point for JavaScript".into(),
                    path: "src/index.js".into(),
                    content: "console.log('Hello from Node.js!');\n".into(),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }

            steps
        },

        "python" => {
    // ЕДИНЫЙ детерминированный requirements.txt для всего стека
    let requirements = preflight::python_manifest(context);

    // Список поддерживаемых реляционных БД
    let sql_dbs: HashSet<&str> = ["postgresql", "postgres", "mysql", "sqlite"]
        .into_iter()
        .collect();

    // Проверяем, какая именно БД выбрана (ищем первое совпадение)
    let detected_db = context.frameworks.iter().find(|fw| sql_dbs.contains(fw.as_str()));
    let has_mongo = context.frameworks.iter().any(|fw| fw == "mongo" || fw == "mongodb");

    // Базовый вектор шагов
    let mut base_vec = vec![
        mkdir("create_src", "src"),
        Step::WriteFile {
            id: "pyproject_toml".into(),
            label: "Create pyproject.toml".into(),
            description: "Initialize Python project configuration".into(),
            path: "pyproject.toml".into(),
            content: format!("[project]\nname = \"{}\"\nversion = \"0.1.0\"\ndescription = \"\"\nrequires-python = \">=3.10\"\n", project_name),
            overwrite: false,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        },
        Step::WriteFile {
            id: "requirements_txt".into(),
            label: "Create requirements.txt".into(),
            description: "Initialize requirements file".into(),
            path: "requirements.txt".into(),
            content: requirements,
            overwrite: false,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        },
    ];

    // Если найдена реляционная БД, генерируем для нее структуру
    if let Some(db_type) = detected_db {
        // Формируем дефолтную строку подключения в зависимости от типа БД
        let default_url = match db_type.as_str() {
            "postgresql" | "postgres" => "postgresql+asyncpg://user:password@localhost:5432/dbname",
            "mysql" | "mariadb" => "mysql+aiomysql://user:password@localhost:3306/dbname",
            "sqlite" => "sqlite+aiosqlite:///./sql_app.db",
            "mssql" => "mssql+aioodbc://user:password@localhost:1433/dbname?driver=ODBC+Driver+17+for+SQL+Server",
            _ => "sqlite+aiosqlite:///./sql_app.db", // фолбек на sqlite
        };

        // Шаблон содержимого database.py
        let db_content = format!(
            r#"import os
from sqlalchemy.ext.asyncio import create_async_engine, AsyncSession, async_sessionmaker
from sqlalchemy.orm import DeclarativeBase

DATABASE_URL = os.getenv("DATABASE_URL", "{}")

engine = create_async_engine(DATABASE_URL, echo=True)
async_session = async_sessionmaker(engine, expire_on_commit=False, class_=AsyncSession)

class Base(DeclarativeBase):
    pass

async def get_db():
    async with async_session() as session:
        yield session
"#,
            default_url
        );

        let db_steps = vec![
            mkdir("create_db", "src/db"), // Хорошая практика: держать модули внутри папки src
            Step::WriteFile {
                id: "database_py".into(),
                label: "Create database.py".into(),
                description: "Create folder with basic database configuration".into(),
                path: "src/db/database.py".into(),
                content: db_content,
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ];
        base_vec.extend(db_steps);
    }
    // Отдельный шаблон, если используется MongoDB
    else if has_mongo {
        let mongo_content = r#"import os
from motor.motor_asyncio import AsyncIOMotorClient

MONGO_URL = os.getenv("DATABASE_URL", "mongodb://localhost:27017")
DATABASE_NAME = os.getenv("DATABASE_NAME", "app_db")

client = AsyncIOMotorClient(MONGO_URL)
db = client[DATABASE_NAME]

def get_nosql_db():
    return db
"#.to_string();

        let mongo_steps = vec![
            mkdir("create_db", "src/db"),
            Step::WriteFile {
                id: "database_py".into(),
                label: "Create database.py".into(),
                description: "Create folder with basic MongoDB configuration".into(),
                path: "src/db/database.py".into(),
                content: mongo_content,
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ];
        base_vec.extend(mongo_steps);
    }

    base_vec
},


        "go" => vec![
            cmd_once("go_mod_init", "Init Go module",
                &format!("Initialize Go module '{}'", project_name),
                &format!("go mod init {}", project_name),
                "go.mod"),
            mkdir("create_cmd", "cmd"),
            mkdir("create_internal", "internal"),
            // Создаём main.go
            Step::WriteFile {
                id: "main_go".into(),
                label: "Create main.go".into(),
                description: "Create Go entry point".into(),
                path: "cmd/main.go".into(),
                content: format!("package main\n\nimport \"fmt\"\n\nfunc main() {{\n\tfmt.Println(\"Hello from {}!\")\n}}\n", project_name),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "java" => vec![
            Step::Command {
                id: "maven_init".into(),
                label: "Init Maven project".into(),
                description: "Generate Maven project structure".into(),
                command: "mvn".into(),
                args: vec![
                    "archetype:generate".into(),
                    "-DgroupId=com.example".into(),
                    format!("-DartifactId={}", project_name),
                    "-DarchetypeArtifactId=maven-archetype-quickstart".into(),
                    "-DarchetypeVersion=1.4".into(),
                    "-DinteractiveMode=false".into(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(120),
                // Повторный запуск: pom.xml уже создан прошлым запуском.
                condition: Some(StepCondition::FileNotExists { path: "pom.xml".into() }),
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ],

        "csharp" => vec![
            cmd("dotnet_new", "Init .NET project",
                "Create new .NET console project",
                &format!("dotnet new console -n {} --force", project_name)),
        ],

        "cpp" => {
            vec![
                mkdir("create_src", "src"),
                mkdir("create_include", "include"),
                Step::WriteFile {
                    id: "main_source".into(),
                    label: "Create main.cpp".into(),
                    description: "Create main source file".into(),
                    path: "src/main.cpp".into(),
                    content: format!("#include <iostream>\n\nint main() {{\n    std::cout << \"Hello from {}!\" << std::endl;\n    return 0;\n}}\n", project_name),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
                // Базовый Makefile
                Step::WriteFile {
                    id: "makefile".into(),
                    label: "Create Makefile".into(),
                    description: "Create basic Makefile for C++".into(),
                    path: "Makefile".into(),
                    content: format!(
                        "CXX=g++\nCXXFLAGS=-Iinclude -Wall -Wextra\nSRC=src/main.cpp\nTARGET={}\n\nall: $(TARGET)\n\n$(TARGET): $(SRC)\n\t$(CXX) $(CXXFLAGS) $(SRC) -o $(TARGET)\n\nclean:\n\trm -f $(TARGET)\n\nrun: all\n\t./$(TARGET)\n",
                        project_name
                    ),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ]
        },

        "zig" => {
            // `zig init` раскладывает shell (build.zig, build.zig.zon,
            // src/) В ТЕКУЩЕМ каталоге — способность generates_root_shell
            // с пост-условиями: провал/неполный вывод виден как ошибка шага,
            // а не тихо скипнутая команда (раньше был plain Command).
            // Сегментация: каталогом становится сегмент (backend/ в
            // mono-репозитории), куда и валидируются build.zig + src/.
            vec![scaffold_step(
                "zig_init",
                "Init Zig project",
                "Initialize Zig project (build.zig, build.zig.zon, src/)",
                "zig",
                vec!["init"],
                ScaffoldCapability::GeneratesRootShell,
                ".",
                ScaffoldExtras::default()
                    .expects(&["build.zig", "build.zig.zon"])
                    .policy(FilePolicy::SkipIfExists),
            )]
        },

        "dart" => {
            // Dart-пакеты не принимают дефис в имени — приводим к подчёркиванию
            let safe_name = project_name.replace('-', "_");
            vec![
                cmd_once("dart_create", "Create Dart project",
                    &format!("Create new Dart project '{}'", project_name),
                    &format!("dart create {}", safe_name),
                    // dart create создаёт ПОДПАПКУ с именем пакета
                    &format!("{}/pubspec.yaml", safe_name)),
            ]
        }

        "kotlin" => vec![
            // Gradle init — интерактивный, поэтому просто создаём структуру
            mkdir("create_src_main", "src/main/kotlin"),
            Step::WriteFile {
                id: "main_kt".into(),
                label: "Create Main.kt".into(),
                description: "Create Kotlin entry point".into(),
                path: "src/main/kotlin/Main.kt".into(),
                content: format!("fun main() {{\n    println(\"Hello from {}!\")\n}}\n", project_name),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "php" => vec![
            mkdir("create_src", "src"),
            Step::WriteFile {
                id: "composer_json".into(),
                label: "Create composer.json".into(),
                description: "Initialize Composer configuration".into(),
                path: "composer.json".into(),
                content: format!("{{\n  \"name\": \"app/{}\",\n  \"description\": \"\",\n  \"type\": \"project\",\n  \"autoload\": {{\n    \"psr-4\": {{\n      \"App\\\\\": \"src/\"\n    }}\n  }}\n}}\n", project_name),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "swift" => vec![
            cmd("swift_init", "Init Swift package",
                &format!("Initialize Swift package '{}'", project_name),
                &format!("swift package init --name {} --type executable", project_name)),
        ],

        "elixir" => vec![
            cmd_once("mix_new", "Create Elixir project",
                &format!("Create new Elixir project '{}'", project_name),
                &format!("mix new {}", project_name),
                "mix.exs"),
        ],

        "gleam" => vec![
            cmd_once("gleam_new", "Create Gleam project",
                &format!("Create new Gleam project '{}'", project_name),
                &format!("gleam new {}", project_name),
                "gleam.toml"),
        ],
        "html" => vec![
            mkdir("create_html_src", "src"),

            Step::WriteFile {
                id: "html_index".into(),
                label: "Create index.html".into(),
                description: "Create the main static HTML page".into(),
                path: "index.html".into(),
                content: format!(
                    r#"<!doctype html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{}</title>
    <link rel="stylesheet" href="src/styles.css">
</head>
<body>
    <main class="page">
        <h1>Hello from {}!</h1>
        <p>Edit <code>index.html</code> to start building your page.</p>
    </main>

    <script src="src/script.js"></script>
</body>
</html>
"#,
                    project_name, project_name
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Abort,
            },

            Step::WriteFile {
                id: "html_styles".into(),
                label: "Create styles.css".into(),
                description: "Create the initial stylesheet for the static page".into(),
                path: "src/styles.css".into(),
                content: r#":root {
    font-family: system-ui, sans-serif;
    color: #1f2937;
    background: #f3f4f6;
}

body {
    margin: 0;
}

.page {
    max-width: 720px;
    margin: 0 auto;
    padding: 4rem 1.5rem;
}
"#
                .into(),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Abort,
            },

            Step::WriteFile {
                id: "html_script".into(),
                label: "Create script.js".into(),
                description: "Create the initial JavaScript file for the static page".into(),
                path: "src/script.js".into(),
                content: r#"console.log("Static HTML project is ready.");
"#
                .into(),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Abort,
            },
        ],

        "c" => vec![
            mkdir("create_c_src", "src"),
            Step::WriteFile {
                id: "c_main".into(),
                label: "Create src/main.c".into(),
                description: "Create C entry point".into(),
                path: "src/main.c".into(),
                content: format!(
                    r#"#include <stdio.h>

int main(void) {{
    printf("Hello from {}!\n");
    return 0;
}}
"#,
                    project_name
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
            Step::WriteFile {
                id: "c_cmakelists".into(),
                label: "Create CMakeLists.txt".into(),
                description: "Initialize CMake configuration for C".into(),
                path: "CMakeLists.txt".into(),
                content: format!(
                    r#"cmake_minimum_required(VERSION 3.15)
project({} C)

set(CMAKE_C_STANDARD 11)

add_executable(app src/main.c)
"#,
                    project_name.replace('-', "_")
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
            Step::WriteFile {
                id: "c_makefile".into(),
                label: "Create Makefile".into(),
                description: "Initialize Makefile for direct compilation".into(),
                path: "Makefile".into(),
                content: format!(
                    "CC = gcc\nCFLAGS = -Wall -Wextra -O2\nSRC = src/main.c\nTARGET = {}\n\nall: $(TARGET)\n\n$(TARGET): $(SRC)\n\t$(CC) $(CFLAGS) -o $(TARGET) $(SRC)\n\nclean:\n\trm -f $(TARGET)\n",
                    if cfg!(target_os = "windows") { "app.exe" } else { "app" }
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "ruby" => vec![
            mkdir("create_ruby_src", "src"),
            Step::WriteFile {
                id: "ruby_main".into(),
                label: "Create src/main.rb".into(),
                description: "Create Ruby entry point".into(),
                path: "src/main.rb".into(),
                content: format!(
                    r#"# frozen_string_literal: true

puts "Hello from {}!"
"#,
                    project_name
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
            Step::WriteFile {
                id: "ruby_gemfile".into(),
                label: "Create Gemfile".into(),
                description: "Initialize Ruby Gemfile".into(),
                path: "Gemfile".into(),
                content: r#"# frozen_string_literal: true

source "https://rubygems.org"

# gem "rake"
"#
                .into(),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],


        _ => vec![
            // Для неизвестных языков создаём базовую структуру
            mkdir("create_src", "src"),
            Step::Command {
                id: "echo_unsupported".into(),
                label: "Unsupported language notice".into(),
                description: format!("Language '{}' has no specific init steps", lang),
                command: "echo".into(),
                args: vec![format!("Project initialized with basic structure for {}", lang)],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(5),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ],
    }
}

/// Вспомогательные функции для определения категорий языков
pub fn _is_frontend_lang(l: &str) -> bool {
    matches!(
        l,
        "typescript" | "javascript" | "dart" | "kotlin" | "swift" | "csharp"
    )
}
pub fn _is_backend_lang(l: &str) -> bool {
    matches!(
        l,
        "rust"
            | "python"
            | "go"
            | "java"
            | "csharp"
            | "php"
            | "elixir"
            | "zig"
            | "gleam"
            | "cpp"
            | "c"
    )
}


