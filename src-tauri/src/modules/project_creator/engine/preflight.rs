// ============================================================================
// Generic dependency and toolchain preflight layer.
//
// Вместо пер-фреймворковых патчей инструментария — единый слой префлайта:
//   - Python: ЕДИНСТВЕННЫЙ канонический venv, вычисляемый по ProjectLayout
//     (python_segment_dir), создаётся один раз; маркер (pyvenv.cfg)
//     проверяется отдельным шагом; pip бутстрапится интерпретатором venv;
//     манифест (requirements.txt) генерируется целиком ДО установки и
//     ставится РОВНО один раз — до любых framework-шагов (django-admin,
//     alembic обязаны видеть окружение). Никакого отдельного «django-venv».
//   - Node: node/npm-префлайт выполняется ДО любого npm-скаффолда; каждый
//     JS-каркас получает пост-валидацию package.json (manifest-check) —
//     каркас не считается успешным, если зависимость фреймворка не
//     задекларирована.
//   - PHP: общий префлайт (исполняемый файл, версия, php.ini, extension_dir,
//     fileinfo, Composer) используется И laravel, И symfony; без Composer
//     каркас не инициализируется — ошибка перечисляет пути поиска и точную
//     команду.
//
// Все диагностические шаги — ErrorMode::Abort: отсутствие обязательного
// инструментария останавливает пайплайн, а не молча пропускается.
// ============================================================================

use std::path::PathBuf;

use super::{framework_def, language_scaffold_suppressed, python_command, python_venv_bin};
use crate::modules::project_creator::models::*;

/// Каталоги, в которых ищется composer.phar (единый список с composer_launch
/// в engine/mod.rs): Toolchain store и стандартные каталоги установки
/// Composer. Порядок важен — первое совпадение побеждает.
pub const COMPOSER_PHAR_DIRS: [&str; 3] = [
    "%LOCALAPPDATA%\\StackPilot\\tools\\php",
    "%APPDATA%\\Composer",
    "%LOCALAPPDATA%\\Programs\\php",
];

// ============================================================================
// Python: канонический манифест и единый жизненный цикл venv
// ============================================================================

/// Полный детерминированный манифест зависимостей Python-проекта:
/// все выбранные Python-фреймворки и инструменты, без дубликатов,
/// в стабильном порядке (фреймворки в порядке выбора, затем инструменты).
/// Это ЕДИНСТВЕННЫЙ источник requirements.txt — framework-шаги ничего не
/// дописывают после генерации.
pub fn python_manifest(context: &WizardContext) -> String {
    python_manifest_lines(context).join("\n")
}

/// Имена пакетов манифеста (без extras/спецификаторов версий) — для
/// пост-валидации requirements.txt (manifest-check).
pub fn python_manifest_entries(context: &WizardContext) -> Vec<String> {
    python_manifest_lines(context)
        .into_iter()
        .map(|line| {
            line.split(['[', '=', '<', '>', '~', '!', ';'])
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn python_manifest_lines(context: &WizardContext) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut add = |line: &str| {
        if !lines.iter().any(|l| l == line) {
            lines.push(line.to_string());
        }
    };
    for fw in &context.frameworks {
        match fw.as_str() {
            "django" => add("django"),
            "fastapi" => {
                // Корректный ASGI-сервер обязателен: fastapi[standard] тянет
                // uvicorn, но явная строка гарантирует сервер даже при
                // смене extras пакета.
                add("fastapi[standard]");
                add("uvicorn");
            }
            "flask" => add("flask"),
            // python-dotenv: сгенерированный код (src/bot.py, src/database.py)
            // читает TELEGRAM_BOT_TOKEN / DATABASE_URL из .env — без
            // загрузчика переменные окружения не применяются.
            "aiogram" => {
                add("aiogram");
                add("python-dotenv");
            }
            _ => {}
        }
    }
    for tool in &context.tools {
        match tool.as_str() {
            "sqlalchemy" => {
                add("sqlalchemy");
                add("python-dotenv");
            }
            "alembic" => add("alembic"),
            "ruff" => add("ruff"),
            "pytest" => add("pytest"),
            // dags/example_dag.py импортирует airflow — без пакета каталог
            // dags/ не запускается.
            "airflow" => add("apache-airflow"),
            // dbt_project.yml + profiles.yml требуют dbt-core и адаптер БД.
            "dbt" => {
                add("dbt-core");
                add("dbt-postgres");
            }
            _ => {}
        }
    }
    lines
}

/// Абсолютный путь к каноническому venv проекта: `<project>\venv` или
/// `<project>\<python_dir>\venv` (backend/ в split-раскладке).
pub fn venv_abs(project_path: &str, python_dir: &str) -> PathBuf {
    let base = PathBuf::from(project_path);
    if python_dir.is_empty() || python_dir == "." {
        base.join("venv")
    } else {
        base.join(python_dir).join("venv")
    }
}

/// Маркер уже созданного venv — путь ОТ КОРНЯ проекта (для условий шагов).
pub fn venv_marker_rel(python_dir: &str) -> String {
    if python_dir.is_empty() || python_dir == "." {
        "venv/pyvenv.cfg".to_string()
    } else {
        format!("{}/venv/pyvenv.cfg", python_dir)
    }
}

/// Путь requirements.txt относительно корня проекта (там, куда сегментация
/// language-скаффолда переносит python-файлы).
pub fn requirements_path(python_dir: &str) -> String {
    if python_dir.is_empty() || python_dir == "." {
        "requirements.txt".to_string()
    } else {
        format!("{}/requirements.txt", python_dir)
    }
}

/// Абсолютный путь к requirements.txt — pip install выполняется из корня
/// проекта, а файл живёт внутри python-сегмента.
pub fn requirements_abs(project_path: &str, python_dir: &str) -> String {
    let base = PathBuf::from(project_path);
    let path = if python_dir.is_empty() || python_dir == "." {
        base.join("requirements.txt")
    } else {
        base.join(python_dir).join("requirements.txt")
    };
    path.to_string_lossy().into_owned()
}

/// Префлайт интерпретатора: печатает путь и версию ДО любых venv/pip-шагов,
/// чтобы ошибка «python не найден» была видна сразу, а не в середине
/// пайплайна.
pub fn python_preflight_step(project_path: &str) -> Step {
    Step::Command {
        id: "python_preflight".into(),
        label: "Check Python interpreter".into(),
        description:
            "Verify the Python interpreter and print its executable path and version".into(),
        command: python_command().to_string(),
        args: vec![
            "-c".into(),
            "import sys; print('interpreter: ' + sys.executable); print('python version: ' + sys.version.split()[0])"
                .into(),
        ],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(30),
        condition: None,
        on_error: ErrorMode::Abort,
        interactive: vec![],
    }
}

/// Скрипт верификации окружения: печатает интерпретатор, версию Python,
/// путь venv, ПРОВЕРЯЕТ маркер pyvenv.cfg и что мы действительно внутри
/// venv (sys.prefix != base_prefix). Несовместимые старые Python
/// предупреждаются явно (requires-python у современных Django/FastAPI).
const VENV_VERIFY_SCRIPT: &str = r#"import sys, os, importlib.util
print('interpreter: ' + sys.executable)
print('python version: ' + sys.version.split()[0])
print('venv path: ' + sys.prefix)
cfg = os.path.join(sys.prefix, 'pyvenv.cfg')
if not os.path.isfile(cfg):
    print('ERROR: virtual environment marker ' + cfg + ' is missing — the environment is broken or incomplete', file=sys.stderr)
    sys.exit(1)
print('venv marker: ' + cfg)
if sys.prefix == getattr(sys, 'base_prefix', None):
    print('ERROR: running from the base interpreter, not from the project virtual environment', file=sys.stderr)
    sys.exit(1)
if importlib.util.find_spec('pip') is None:
    print('pip module: missing (will be bootstrapped by the next step)')
else:
    print('pip module: present')
if sys.version_info < (3, 9):
    print('WARNING: Python ' + sys.version.split()[0] + ' is older than 3.9; current Django and FastAPI releases require Python 3.10+ and pip may refuse to install the manifest (requires-python)', file=sys.stderr)
"#;

/// Канонический жизненный цикл окружения Python-проекта — ЕДИНСТВЕННЫЙ для
/// всех Python-фреймворков (django, fastapi, flask, aiogram) и инструментов
/// (alembic, ruff, pytest):
///   1. py_venv_create — `python -m venv <abs>` ровно один раз (маркер);
///   2. py_venv_verify — проверка маркера/интерпретатора/версии;
///   3. py_pip_upgrade — bootstrap pip ИНТЕРПРЕТАТОРОМ venv;
///   4. py_pip_check — `python -m pip --version` (виден путь и версия pip);
///   5. py_pip_install — установка манифеста РОВНО один раз (`-r`).
/// Все вызовы pip и Python-CLI (django-admin, alembic, ruff) идут только
/// как `<venv>/python -m pip ...` / бинарники venv.
pub fn python_environment_steps(project_path: &str, python_dir: &str) -> Vec<Step> {
    let wd = project_path.to_string();
    let venv_str = venv_abs(project_path, python_dir)
        .to_string_lossy()
        .into_owned();
    let marker = venv_marker_rel(python_dir);
    let req_str = requirements_abs(project_path, python_dir);
    let venv_py = python_venv_bin(project_path, python_dir, "python");

    vec![
        Step::Command {
            id: "py_venv_create".into(),
            label: "Create Python virtual environment".into(),
            description: format!(
                "Run {} -m venv {} (canonical project environment, created once)",
                python_command(),
                venv_str
            ),
            command: python_command().to_string(),
            args: vec!["-m".into(), "venv".into(), venv_str],
            working_dir: Some(wd.clone()),
            env: None,
            timeout_secs: Some(120),
            condition: Some(StepCondition::FileNotExists { path: marker }),
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "py_venv_verify".into(),
            label: "Verify Python virtual environment".into(),
            description: "Verify the venv marker (pyvenv.cfg), interpreter and Python version".into(),
            command: venv_py.clone(),
            args: vec!["-c".into(), VENV_VERIFY_SCRIPT.into()],
            working_dir: Some(wd.clone()),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "py_pip_upgrade".into(),
            label: "Bootstrap pip in virtual environment".into(),
            description: "Run python -m pip install --upgrade pip inside the project venv".into(),
            command: venv_py.clone(),
            args: vec![
                "-m".into(),
                "pip".into(),
                "install".into(),
                "--upgrade".into(),
                "pip".into(),
            ],
            working_dir: Some(wd.clone()),
            env: None,
            timeout_secs: Some(300),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "py_pip_check".into(),
            label: "Verify pip in virtual environment".into(),
            description: "Run python -m pip --version inside the project venv (shows pip version and interpreter)".into(),
            command: venv_py.clone(),
            args: vec!["-m".into(), "pip".into(), "--version".into()],
            working_dir: Some(wd.clone()),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "py_pip_install".into(),
            label: "Install Python dependencies".into(),
            description: format!(
                "Run python -m pip install -r {} inside the project venv (exactly one manifest install)",
                req_str
            ),
            command: venv_py,
            args: vec![
                "-m".into(),
                "pip".into(),
                "install".into(),
                "-r".into(),
                req_str,
            ],
            working_dir: Some(wd),
            env: None,
            timeout_secs: Some(600),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
    ]
}

// ============================================================================
// Node / npm: префлайт для каждого npm-скаффолда
// ============================================================================

/// node/npm-префлайт: выполняется ДО любого npm-скаффолда. Отсутствие
/// node или npm останавливает пайплайн (Abort) — каркасы не «молча
/// пропускаются» из-за невозможности запустить npx.
pub fn node_preflight_steps(project_path: &str) -> Vec<Step> {
    let wd = project_path.to_string();
    vec![
        Step::Command {
            id: "node_preflight".into(),
            label: "Check Node.js".into(),
            description:
                "Verify the Node.js runtime (prints version and executable path) before any npm-based scaffold".into(),
            command: "node".into(),
            args: vec![
                "-e".into(),
                "console.log('node version: ' + process.version); console.log('node executable: ' + process.execPath)"
                    .into(),
            ],
            working_dir: Some(wd.clone()),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "npm_preflight".into(),
            label: "Check npm".into(),
            description: "Verify the npm CLI (npm --version) before any npm-based scaffold".into(),
            command: "npm".into(),
            args: vec!["--version".into()],
            working_dir: Some(wd),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
    ]
}

// ============================================================================
// Generic host tools: где/которые-префлайт для всего, что движок реально
// вызывает при генерации (dart, cargo, go, dotnet, zig, mix, gleam, maven,
// gradle...). node/npm/python/php+composer покрыты специализированными
// префлайтами — generic-проверка их не дублирует.
// ============================================================================

/// Инструмент хоста, который вызывает language-скаффолд языка (по тем же
/// правилам, что steps_for_language в engine/mod.rs). None — язык не
/// вызывает CLI при генерации (kotlin, c/cpp — только файлы) или покрыт
/// специализированным префлайтом (node/npm/python/php).
fn language_host_tool(lang: &str) -> Option<&'static str> {
    match lang {
        "rust" => Some("cargo"),
        "go" => Some("go"),
        "java" => Some("mvn"),
        "csharp" => Some("dotnet"),
        "zig" => Some("zig"),
        "dart" => Some("dart"),
        "swift" => Some("swift"),
        "elixir" => Some("mix"),
        "gleam" => Some("gleam"),
        _ => None,
    }
}

/// Инструмент покрыт специализированным префлайтом (node/npm —
/// node_preflight_steps, python — python_preflight_step, php+composer —
/// php_preflight_step): generic-проверка не дублирует его.
fn host_tool_covered_by_dedicated_preflight(tool: &str) -> bool {
    matches!(tool, "node" | "npm" | "python" | "php" | "composer")
}

/// Generic-префлайт инструментов хоста: по одному Abort-шагу на группу
/// требований. Группы:
///   - «все» (mode=all): инструменты языковых скаффолдов + единственные
///     required_tools фреймворков (ktor→gradle) — каждый обязан быть в PATH;
///   - «хотя бы один» (mode=any): альтернативные required_tools фреймворка
///     (spring-boot: maven ИЛИ gradle).
/// Отсутствие инструмента останавливает пайплайн с понятной причиной —
/// каркасы не «молча пропускаются» из-за невозможности запустить CLI.
pub fn host_tool_preflight_steps(context: &WizardContext) -> Vec<Step> {
    let mut all_tools: Vec<String> = Vec::new();
    let mut any_groups: Vec<Vec<String>> = Vec::new();

    for lang in &context.languages {
        if language_scaffold_suppressed(lang, context) {
            continue;
        }
        if let Some(tool) = language_host_tool(lang) {
            push_unique(&mut all_tools, tool.to_string());
        }
    }
    for fw in &context.frameworks {
        let Some(def) = framework_def(fw) else {
            continue;
        };
        let mut required: Vec<String> = def
            .required_tools
            .iter()
            .filter(|t| !host_tool_covered_by_dedicated_preflight(t))
            .cloned()
            .collect();
        if required.is_empty() {
            continue;
        }
        required.sort();
        required.dedup();
        if required.len() == 1 {
            push_unique(&mut all_tools, required[0].clone());
        } else if !any_groups.contains(&required) {
            any_groups.push(required);
        }
    }

    // Erlang/OTP — обязательный рантайм Elixir: `mix`/`elixir` стартуют
    // Erlang VM (erl.exe). Без erl в PATH mix падает кодом 9009 в середине
    // скаффолда (phoenix), а не понятной ошибкой префлайта. Оба инструмента
    // обязательны ОДНОВРЕМЕННО (не альтернативы — режим any сюда не
    // подходит). erl/mix добавляются и когда elixir-скаффолд подавлен
    // фреймворком (phoenix подавляет generic `mix new` и не декларирует
    // required_tools — иначе проверки не было бы вовсе).
    let uses_elixir = context.languages.iter().any(|l| l == "elixir")
        || context.frameworks.iter().any(|f| f == "phoenix");
    if uses_elixir {
        push_unique(&mut all_tools, "erl".to_string());
        push_unique(&mut all_tools, "mix".to_string());
    }

    all_tools.sort();
    all_tools.dedup();

    let mut steps: Vec<Step> = Vec::new();
    if !all_tools.is_empty() {
        steps.push(host_tool_check_step(
            "host_tools_preflight",
            "Check host toolchain",
            "Verify required host tools (where/which) before scaffolding",
            &all_tools,
            true,
        ));
    }
    for group in any_groups {
        steps.push(host_tool_check_step(
            &format!("host_tool_{}_preflight", group.join("_or_")),
            &format!("Check {} toolchain", group.join("/")),
            &format!(
                "Verify one of the required host tools is available: {}",
                group.join(", ")
            ),
            &group,
            false,
        ));
    }
    steps
}

/// Один Abort-шаг проверки инструментов через генератор "host-tool-check".
fn host_tool_check_step(
    id: &str,
    label: &str,
    desc: &str,
    tools: &[String],
    mode_all: bool,
) -> Step {
    Step::Generate {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        generator_id: "host-tool-check".into(),
        generator_config: serde_json::json!({
            "tools": tools,
            "mode": if mode_all { "all" } else { "any" },
        }),
        policy: None,
        condition: None,
        on_error: ErrorMode::Abort,
    }
}

/// Добавить значение в список, если его там ещё нет.
fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

// ============================================================================
// PHP: общий префлайт Laravel/Symfony + Composer
// ============================================================================

/// Скрипт префлайта PHP: исполняемый файл, версия, php.ini, extension_dir,
/// fileinfo, затем доступность Composer (PATH или composer.phar в каталогах
/// discovery — тот же список, что у composer_launch). При отсутствии
/// Composer печатаются все проверенные пути и ТОЧНАЯ команда, которая
/// потребуется для скаффолда; выход с кодом 2 останавливает пайплайн
/// ДО инициализации каркаса. php.ini не изменяется.
const PHP_PREFLIGHT_SCRIPT: &str = r#"$version = PHP_VERSION;
$binary = PHP_BINARY;
if (!$binary) { $binary = 'unknown'; }
$ini = php_ini_loaded_file();
if (!$ini) { $ini = 'none'; }
$ext_dir = ini_get('extension_dir');
if (!$ext_dir) { $ext_dir = 'unknown'; }
echo 'PHP executable: ' . $binary . "\n";
echo 'PHP version: ' . $version . "\n";
echo 'php.ini: ' . $ini . "\n";
echo 'extension_dir: ' . $ext_dir . "\n";
if (!extension_loaded('fileinfo')) {
    $dll_names = ['fileinfo.dll', 'php_fileinfo.dll'];
    $dll_found = '';
    $search_dir = $ext_dir;
    if ($search_dir !== 'unknown' && $search_dir !== '') {
        foreach ($dll_names as $dll) {
            $candidate = $search_dir . DIRECTORY_SEPARATOR . $dll;
            if (is_file($candidate)) { $dll_found = $candidate; break; }
        }
        if ($dll_found === '' && !empty($ini) && $ini !== 'none') {
            $ini_dir = dirname($ini);
            foreach ($dll_names as $dll) {
                $candidate = $ini_dir . DIRECTORY_SEPARATOR . 'ext' . DIRECTORY_SEPARATOR . $dll;
                if (is_file($candidate)) { $dll_found = $candidate; break; }
            }
        }
    }
    $ini_line_exists = false;
    if (!empty($ini) && $ini !== 'none' && is_file($ini)) {
        $ini_content = file_get_contents($ini);
        if (preg_match('/^\s*;?\s*extension\s*=\s*fileinfo\s*$/mi', $ini_content)) {
            $ini_line_exists = true;
        }
    }
    fwrite(STDERR, "PHP error: extension fileinfo is not enabled.\n");
    fwrite(STDERR, 'php.ini: ' . $ini . "\n");
    fwrite(STDERR, 'extension_dir: ' . $ext_dir . "\n");
    if ($dll_found !== '') {
        fwrite(STDERR, "Extension file found: " . $dll_found . "\n");
    }
    if ($ini_line_exists) {
        fwrite(STDERR, "The line 'extension=fileinfo' exists in php.ini (likely commented out with ';'). Uncomment it and restart PHP.\n");
        fwrite(STDERR, "Open php.ini and remove the ';' before 'extension=fileinfo'.\n");
    } else {
        fwrite(STDERR, "Add this line to php.ini (or uncomment it if present with ';'):\n");
        fwrite(STDERR, "  extension=fileinfo\n");
    }
    fwrite(STDERR, "Required by Composer to download packages via stream wrappers.\n");
    fwrite(STDERR, "Alternative: install Composer via 'php -d extension=fileinfo <path-to-composer.phar>' and retry.\n");
    exit(1);
}
echo "fileinfo: enabled\n";
$win = DIRECTORY_SEPARATOR === '\\';
$out = array();
$code = 0;
if ($win) { exec('where composer 2>NUL', $out, $code); } else { exec('which composer 2>/dev/null', $out, $code); }
if ($code === 0 && count($out) > 0 && trim($out[0]) !== '') {
    echo 'Composer: ' . trim($out[0]) . " (from PATH)\n";
    exit(0);
}
$local = getenv('LOCALAPPDATA');
$roaming = getenv('APPDATA');
$phars = array(
    $win && $local ? $local . '\\StackPilot\\tools\\php\\composer.phar' : '',
    $win && $roaming ? $roaming . '\\Composer\\composer.phar' : '',
    $win && $local ? $local . '\\Programs\\php\\composer.phar' : '',
);
foreach ($phars as $phar) {
    if ($phar !== '' && is_file($phar)) {
        echo 'Composer: ' . $phar . " (composer.phar)\n";
        exit(0);
    }
}
fwrite(STDERR, "Composer error: composer is not available — the framework cannot be initialized.\n");
fwrite(STDERR, "Searched locations:\n");
fwrite(STDERR, '  - PATH (' . ($win ? 'where composer' : 'which composer') . ")\n");
foreach ($phars as $phar) {
    if ($phar !== '') { fwrite(STDERR, '  - ' . $phar . "\n"); }
}
fwrite(STDERR, "Required command (after installing Composer):\n");
fwrite(STDERR, '  composer create-project __PACKAGE__ . --no-interaction --prefer-source' . "\n");
fwrite(STDERR, '  or: php <path-to-composer.phar> create-project __PACKAGE__ . --no-interaction --prefer-source' . "\n");
exit(2);
"#;

/// Общий PHP-префлайт для Composer-каркасов (laravel, symfony): проверяет
/// PHP (путь, версия, php.ini, extension_dir, fileinfo) и доступность
/// Composer, печатает точную команду скаффолда. Abort: без обязательных
/// предусловий каркас НЕ инициализируется.
///
/// Если fileinfo не загружен, скрипт проверяет, существует ли DLL расширения
/// в extension_dir — если да, пытается загрузить через `-d extension=fileinfo`
/// и предупреждает пользователя раскомментировать строку в php.ini. Если DLL
/// не найдена — ошибка с инструкцией по установке.
pub fn php_preflight_step(id: &str, label: &str, desc: &str, composer_package: &str) -> Step {
    let script = PHP_PREFLIGHT_SCRIPT.replace("__PACKAGE__", composer_package);
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        command: "php".into(),
        args: vec![
            "-d".into(),
            "extension=fileinfo".into(),
            "-r".into(),
            script,
        ],
        working_dir: None,
        env: None,
        timeout_secs: Some(30),
        condition: None,
        on_error: ErrorMode::Abort,
        interactive: vec![],
    }
}

// ============================================================================
// Валидация манифестов (manifest-check) и патчи зависимостей
// ============================================================================

/// Пост-валидация манифеста: генератор "manifest-check" проверяет, что файл
/// существует, парсится и содержит обязательные зависимости. Используется
/// для package.json, composer.json, requirements.txt, pyproject.toml,
/// Cargo.toml. Abort: каркас не считается успешным, пока манифест не
/// подтверждён.
pub fn manifest_check_step(
    id: &str,
    label: &str,
    path: &str,
    kind: &str,
    required: &[&str],
) -> Step {
    Step::Generate {
        id: id.to_string(),
        label: label.to_string(),
        description: format!(
            "Validate {} and verify required dependencies: {}",
            path,
            required.join(", ")
        ),
        generator_id: "manifest-check".into(),
        generator_config: serde_json::json!({
            "path": path,
            "kind": kind,
            "required_dependencies": required,
        }),
        policy: None,
        condition: None,
        on_error: ErrorMode::Abort,
    }
}

/// Проверка package.json на наличие обязательных npm-зависимостей
/// (dependencies + devDependencies).
pub fn package_json_check_step(id: &str, label: &str, path: &str, deps: &[&str]) -> Step {
    manifest_check_step(id, label, path, "package_json", deps)
}

/// Патч манифеста через глубокое JSON-слияние (FilePolicy::MergeJson):
/// добавляет недостающие зависимости в существующий package.json, не
/// трогая остальные ключи. Условие FileExists — патч применяется только
/// к реально созданному каркасу (нет package.json → шаг пропускается).
pub fn manifest_patch_step(id: &str, label: &str, path: &str, content: String) -> Step {
    Step::WriteFile {
        id: id.to_string(),
        label: label.to_string(),
        description: format!("Merge required dependencies into {}", path),
        path: path.to_string(),
        content,
        overwrite: false,
        policy: Some(FilePolicy::MergeJson),
        condition: Some(StepCondition::FileExists {
            path: path.to_string(),
        }),
        on_error: ErrorMode::Abort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(languages: &[&str], frameworks: &[&str], tools: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            docker: false,
            ..Default::default()
        }
    }

    #[test]
    fn python_manifest_is_deterministic_and_deduplicated() {
        // Дубликат через фреймворк и инструмент (alembic в tools дважды не
        // встретится, но защита от повторных строк обязана работать) —
        // проверяем, что порядок стабилен и дублей нет.
        let a = python_manifest(&ctx(
            &["python"],
            &["django", "fastapi"],
            &["alembic", "sqlalchemy", "ruff"],
        ));
        let b = python_manifest(&ctx(
            &["python"],
            &["django", "fastapi"],
            &["alembic", "sqlalchemy", "ruff"],
        ));
        assert_eq!(a, b, "манифест детерминирован");
        let lines: Vec<&str> = a.lines().collect();
        let mut uniq = lines.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(lines.len(), uniq.len(), "нет дубликатов: {a}");
        assert!(a.contains("django"), "{a}");
        assert!(a.contains("fastapi[standard]"), "{a}");
        assert!(
            a.contains("uvicorn"),
            "ASGI-сервер обязан быть в манифесте: {a}"
        );
        assert!(
            a.contains("alembic") && a.contains("sqlalchemy") && a.contains("ruff"),
            "{a}"
        );
    }

    #[test]
    fn python_environment_is_single_canonical_chain() {
        let steps = python_environment_steps("C:\\dev\\myapp", ".");
        let ids: Vec<String> = steps.iter().map(|s| s.id()).collect();
        assert_eq!(
            ids,
            vec![
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install"
            ]
        );
        // Все шаги — Abort: сбой обязательной установки останавливает пайплайн.
        for step in &steps {
            let on_error = match step {
                Step::Command { on_error, .. } => on_error,
                _ => panic!("все шаги цепочки — Command"),
            };
            assert_eq!(on_error, &ErrorMode::Abort, "{:?}", step.id());
        }
        // venv создаётся ровно один раз (маркер), pip — только через venv.
        let create = &steps[0];
        let (cmd, args) = match create {
            Step::Command { command, args, .. } => (command, args),
            _ => unreachable!(),
        };
        assert!(cmd == "python" || cmd == "python3", "{cmd}");
        assert_eq!(&args[..2], &["-m".to_string(), "venv".to_string()]);
        assert!(args[2].ends_with("venv"), "{args:?}");
        assert!(matches!(
            create.condition(),
            Some(StepCondition::FileNotExists { path }) if path == "venv/pyvenv.cfg"
        ));
        // pip install — интерпретатором venv, ровно из манифеста (-r).
        let install = &steps[4];
        let (cmd, args) = match install {
            Step::Command { command, args, .. } => (command, args),
            _ => unreachable!(),
        };
        assert!(cmd.contains("venv"), "pip вызывается только из venv: {cmd}");
        assert_eq!(&args[..2], &["-m".to_string(), "pip".to_string()]);
        assert_eq!(args[2], "install");
        assert_eq!(args[3], "-r");
        assert!(args[4].contains("requirements.txt"), "{args:?}");
    }

    #[test]
    fn venv_paths_follow_segment() {
        assert_eq!(
            venv_marker_rel("."),
            "venv/pyvenv.cfg",
            "корневой Python-проект"
        );
        assert_eq!(
            venv_marker_rel("backend"),
            "backend/venv/pyvenv.cfg",
            "backend/ Python-проект"
        );
        assert_eq!(requirements_path("."), "requirements.txt");
        assert_eq!(requirements_path("backend"), "backend/requirements.txt");
        // Абсолютный venv: разделитель платформенный — join использует
        // MAIN_SEPARATOR, поэтому на Windows это myapp\backend\venv, на
        // Unix myapp/backend/venv (проверяем через ожидание с sep).
        let sep = std::path::MAIN_SEPARATOR;
        let abs = venv_abs("C:/dev/myapp", "backend")
            .to_string_lossy()
            .into_owned();
        let expected = format!("myapp{sep}backend{sep}venv");
        assert!(abs.ends_with(&expected), "{abs}");
    }

    #[test]
    fn node_preflight_aborts_for_missing_runtime() {
        let steps = node_preflight_steps("C:\\dev\\myapp");
        let ids: Vec<String> = steps.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["node_preflight", "npm_preflight"]);
        for step in &steps {
            match step {
                Step::Command {
                    command, on_error, ..
                } => {
                    assert!(command == "node" || command == "npm", "{command}");
                    assert_eq!(on_error, &ErrorMode::Abort);
                }
                _ => panic!("префлайты — Command"),
            }
        }
    }

    #[test]
    fn php_preflight_checks_toolchain_and_composer() {
        let step = php_preflight_step(
            "laravel_php_check",
            "Check PHP for Laravel",
            "preflight",
            "laravel/laravel",
        );
        match step {
            Step::Command {
                command,
                args,
                on_error,
                ..
            } => {
                assert_eq!(command, "php");
                assert_eq!(args[0], "-d");
                assert_eq!(args[1], "extension=fileinfo");
                assert_eq!(args[2], "-r");
                let script = &args[3];
                for needle in [
                    "PHP executable",
                    "PHP version",
                    "php.ini",
                    "extension_dir",
                    "fileinfo",
                    "where composer",
                    "StackPilot",
                    "create-project laravel/laravel",
                ] {
                    assert!(
                        script.contains(needle),
                        "скрипт префлайта обязан покрывать '{needle}': {script}"
                    );
                }
                assert_eq!(on_error, ErrorMode::Abort);
            }
            _ => panic!("php-префлайт — Command"),
        }
    }

    #[test]
    fn manifest_check_steps_are_abort() {
        let step = package_json_check_step(
            "express_pkg_check",
            "Validate package.json",
            "package.json",
            &["express"],
        );
        match step {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "manifest-check");
                assert_eq!(
                    generator_config.get("path").and_then(|v| v.as_str()),
                    Some("package.json")
                );
                assert_eq!(
                    generator_config
                        .get("required_dependencies")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str()),
                    Some("express")
                );
                assert_eq!(on_error, ErrorMode::Abort);
            }
            _ => panic!("manifest-check — Generate"),
        }
    }

    #[test]
    fn manifest_patch_merges_json_only_when_manifest_exists() {
        let step = manifest_patch_step(
            "prisma_deps",
            "Add Prisma dependencies",
            "backend/package.json",
            serde_json::json!({ "devDependencies": { "prisma": "^6.1.0" } }).to_string(),
        );
        match step {
            Step::WriteFile {
                policy,
                condition,
                on_error,
                ..
            } => {
                assert_eq!(policy, Some(FilePolicy::MergeJson));
                assert!(matches!(
                    condition,
                    Some(StepCondition::FileExists { path }) if path == "backend/package.json"
                ));
                assert_eq!(on_error, ErrorMode::Abort);
            }
            _ => panic!("патч — WriteFile"),
        }
    }

    #[test]
    fn host_tool_preflight_covers_language_scaffold_tools() {
        // rust+go: language-скаффолды вызывают cargo init и go mod init —
        // generic-префлайт обязан проверить оба инструмента (mode=all).
        let steps = host_tool_preflight_steps(&ctx(&["rust", "go"], &[], &[]));
        assert_eq!(steps.len(), 1, "все инструменты — в одном all-шаге");
        let Step::Generate {
            generator_id,
            generator_config,
            on_error,
            ..
        } = &steps[0]
        else {
            panic!("host-tool префлайт — Generate");
        };
        assert_eq!(generator_id, "host-tool-check");
        assert_eq!(on_error, &ErrorMode::Abort);
        let tools: Vec<String> = generator_config
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(tools, vec!["cargo".to_string(), "go".to_string()]);
        assert_eq!(
            generator_config.get("mode").and_then(|v| v.as_str()),
            Some("all")
        );
    }

    #[test]
    fn host_tool_preflight_merges_single_required_tools_and_keeps_alternatives() {
        // flutter требует dart (tree), spring-boot — maven ИЛИ gradle:
        // dart уходит в общий all-шаг, maven/gradle остаются отдельным
        // any-шагом (альтернативы).
        let steps = host_tool_preflight_steps(&ctx(&["dart"], &["flutter", "spring-boot"], &[]));
        let groups: Vec<(&str, Vec<String>, &str)> = steps
            .iter()
            .map(|s| match s {
                Step::Generate {
                    generator_id,
                    generator_config,
                    ..
                } => {
                    let tools: Vec<String> = generator_config
                        .get("tools")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        generator_id.as_str(),
                        tools,
                        generator_config
                            .get("mode")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    )
                }
                _ => unreachable!(),
            })
            .collect();
        assert!(
            groups.contains(&("host-tool-check", vec!["dart".to_string()], "all")),
            "dart — обязательный инструмент: {groups:?}"
        );
        assert!(
            groups.contains(&(
                "host-tool-check",
                vec!["gradle".to_string(), "maven".to_string()],
                "any"
            )),
            "maven/gradle — альтернативы (any): {groups:?}"
        );
    }

    #[test]
    fn host_tool_preflight_requires_erlang_and_mix_for_elixir() {
        // phoenix: elixir-скаффолд подавлен (mix phx.new генерирует каркас
        // сам), required_tools phoenix пуст — без явной проверки erl/mix
        // префлайта не было бы вовсе, и mix падал бы кодом 9009 в середине
        // скаффолда. erl И mix обязательны ОДНОВРЕМЕННО (mode=all).
        let steps = host_tool_preflight_steps(&ctx(&["elixir"], &["phoenix"], &[]));
        let Step::Generate {
            generator_id,
            generator_config,
            on_error,
            ..
        } = &steps[0]
        else {
            panic!("host-tool префлайт — Generate");
        };
        assert_eq!(generator_id, "host-tool-check");
        assert_eq!(on_error, &ErrorMode::Abort);
        let tools: Vec<String> = generator_config
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            tools.contains(&"erl".to_string()),
            "Erlang/OTP (erl) обязателен для Elixir: {tools:?}"
        );
        assert!(
            tools.contains(&"mix".to_string()),
            "mix обязателен для Elixir: {tools:?}"
        );
        assert_eq!(
            generator_config.get("mode").and_then(|v| v.as_str()),
            Some("all"),
            "erl и mix — НЕ альтернативы (any), а два обязательных инструмента: {generator_config}"
        );

        // elixir без phoenix: язык не подавлен — mix добавляется и языковым
        // путём, erl — явно; оба в одном all-шаге.
        let steps = host_tool_preflight_steps(&ctx(&["elixir"], &[], &[]));
        let Step::Generate {
            generator_config, ..
        } = &steps[0]
        else {
            panic!("host-tool префлайт — Generate");
        };
        let tools: Vec<String> = generator_config
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(tools.contains(&"erl".to_string()), "{tools:?}");
        assert!(tools.contains(&"mix".to_string()), "{tools:?}");
    }

    #[test]
    fn host_tool_preflight_skips_dedicated_and_suppressed_tools() {
        // typescript: node/npm покрыты node_preflight_steps — не дублируются;
        // dart при flutter: language-скаффолд подавлен (flutter генерирует
        // каркас сам), но dart остаётся через required_tools фреймворка.
        let steps = host_tool_preflight_steps(&ctx(&["typescript", "dart"], &["flutter"], &[]));
        let configs: Vec<serde_json::Value> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Generate {
                    generator_config, ..
                } => Some(generator_config.clone()),
                _ => None,
            })
            .collect();
        let all_tools = configs
            .iter()
            .flat_map(|c| {
                c.get("tools")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)))
                    .into_iter()
                    .flatten()
            })
            .collect::<Vec<_>>();
        assert!(
            !all_tools.iter().any(|t| t == "node" || t == "npm"),
            "{all_tools:?}"
        );
        assert!(all_tools.contains(&"dart".to_string()), "{all_tools:?}");
        assert_eq!(configs.len(), 1, "только dart: {configs:?}");
    }
}
