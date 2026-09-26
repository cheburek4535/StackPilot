use super::*;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::ports;
use std::path::PathBuf;

// ============================================================================
// Qt: генерация по UI-режиму (QML / Widgets / WebEngine / Kirigami).
// Режим выбирается в мастере (answers["qt_ui"] = ["qt-qml"] и т.п.) —
// он решает, какие модули Qt подключить и какой main.cpp сгенерировать.
// ============================================================================

pub fn qt_ui_mode(context: &WizardContext) -> &'static str {
    if let Some(modes) = context.answers.get("qt_ui") {
        if let Some(m) = modes.first() {
            return match m.as_str() {
                "qt-qml" => "qml",
                "qt-webengine" => "webengine",
                "qt-kirigami" => "kirigami",
                _ => "widgets",
            };
        }
    }
    // Ретро-совместимость: стек без ответов мастера (пресеты, старые сессии)
    if context.frameworks.iter().any(|f| f == "qt-qml") {
        return "qml";
    }
    if context.frameworks.iter().any(|f| f == "qt-webengine") {
        return "webengine";
    }
    if context.frameworks.iter().any(|f| f == "qt-kirigami") {
        return "kirigami";
    }
    "widgets"
}

pub fn qt_web_framework_label(context: &WizardContext) -> &'static str {
    if let Some(fws) = context.answers.get("qt_web_framework") {
        if let Some(f) = fws.first() {
            return match f.as_str() {
                "react" => "React",
                "vue" => "Vue",
                "svelte" => "Svelte",
                _ => "web UI",
            };
        }
    }
    if context.frameworks.iter().any(|f| f == "react") {
        "React"
    } else if context.frameworks.iter().any(|f| f == "vue") {
        "Vue"
    } else if context.frameworks.iter().any(|f| f == "svelte") {
        "Svelte"
    } else {
        "web UI"
    }
}

pub fn qt_step_write(id: &str, label: &str, path: &str, content: String) -> Step {
    Step::WriteFile {
        id: id.to_string(),
        label: label.to_string(),
        description: format!("Create {}", path),
        path: path.to_string(),
        content,
        overwrite: false,
        policy: None,
        condition: None,
        on_error: ErrorMode::Abort,
    }
}

pub fn qt_steps_widgets(project_name: &str) -> Vec<Step> {
    vec![
        qt_step_write(
            "qt_main",
            "Create Qt main",
            "src/main.cpp",
            format!(
                r#"#include <QApplication>
#include <QWidget>
#include <QPushButton>

int main(int argc, char *argv[]) {{
    QApplication app(argc, argv);
    QWidget window;
    window.setWindowTitle("{}");
    window.resize(400, 300);
    QPushButton btn("Hello from {}!", &window);
    btn.setGeometry(50, 50, 300, 200);
    window.show();
    return app.exec();
}}
"#,
                project_name, project_name
            ),
        ),
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
                r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

add_executable({p} src/main.cpp)
target_link_libraries({p} Qt6::Widgets)
"#,
                p = project_name
            ),
        ),
    ]
}

pub fn qt_steps_qml(project_name: &str) -> Vec<Step> {
    let main_cpp = r#"#include <QGuiApplication>
#include <QQmlApplicationEngine>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QQmlApplicationEngine engine;
    const QUrl url(QStringLiteral("qrc:/main.qml"));
    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreated,
        &app, [url](QObject *obj, const QUrl &objUrl) {
            if (!obj && url == objUrl)
                QCoreApplication::exit(-1);
        },
        Qt::QueuedConnection);
    engine.load(url);
    return app.exec();
}
"#;
    let main_qml = format!(
        r#"import QtQuick

Window {{
    width: 480
    height: 320
    visible: true
    title: "{}"

    Text {{
        anchors.centerIn: parent
        text: "Hello from {}!"
        font.pixelSize: 24
    }}
}}
"#,
        project_name, project_name
    );
    vec![
        qt_step_write("qt_main", "Create Qt main", "src/main.cpp", main_cpp.into()),
        qt_step_write("qt_qml_main", "Create QML view", "src/main.qml", main_qml),
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
                r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Quick)

add_executable({p} src/main.cpp)

qt_add_resources({p} "qml"
    PREFIX "/"
    FILES src/main.qml
)

target_link_libraries({p} Qt6::Quick)
"#,
                p = project_name
            ),
        ),
    ]
}

pub fn qt_steps_kirigami(project_name: &str) -> Vec<Step> {
    let main_cpp = r#"#include <QGuiApplication>
#include <QQmlApplicationEngine>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QQmlApplicationEngine engine;
    const QUrl url(QStringLiteral("qrc:/main.qml"));
    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreated,
        &app, [url](QObject *obj, const QUrl &objUrl) {
            if (!obj && url == objUrl)
                QCoreApplication::exit(-1);
        },
        Qt::QueuedConnection);
    engine.load(url);
    return app.exec();
}
"#;
    let main_qml = format!(
        r#"import QtQuick
import org.kde.kirigami 2.20 as Kirigami

Kirigami.ApplicationWindow {{
    width: 600
    height: 450
    title: "{}"

    pageStack.initialPage: Kirigami.Page {{
        Kirigami.Heading {{
            text: "Hello from {}!"
        }}
    }}
}}
"#,
        project_name, project_name
    );
    vec![
        qt_step_write("qt_main", "Create Qt main", "src/main.cpp", main_cpp.into()),
        qt_step_write(
            "qt_kirigami_main",
            "Create Kirigami view",
            "src/main.qml",
            main_qml,
        ),
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
                r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Quick)
find_package(KF6 REQUIRED COMPONENTS Kirigami)

add_executable({p} src/main.cpp)

qt_add_resources({p} "qml"
    PREFIX "/"
    FILES src/main.qml
)

target_link_libraries({p} Qt6::Quick KF6::Kirigami)
"#,
                p = project_name
            ),
        ),
    ]
}

pub fn qt_steps_webengine(project_name: &str, context: &WizardContext, seg: Option<&str>) -> Vec<Step> {
    let web = qt_web_framework_label(context);
    // Расширенные шаблоны живут в TemplateEngine ({{ project_name }} и т.п.)
    // — см. engine/template.rs: qt_webengine_main_cpp / qt_webengine_cmake.
    let engine = template::TemplateEngine::new();
    let main_cpp = engine.qt_webengine_main_cpp(project_name);
    let cmake_lists = engine.qt_webengine_cmake(project_name);
    // Пост-условия с учётом сегментации qt (backend/ в mono-репозитории):
    // into_segment не переписывает condition-пути, поэтому префикс сегмента
    // добавляется ЗДЕСЬ; рабочие директории cmake-шагов (".") сегментация
    // переведёт в каталог qt (backend/) сама.
    let cmake_cond = match seg {
        Some(dir) => format!("{}/CMakeLists.txt", dir),
        None => "CMakeLists.txt".to_string(),
    };
    vec![
        qt_step_write("qt_main", "Create Qt main (WebEngine)", "src/main.cpp", main_cpp),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", cmake_lists),
        Step::Command {
            id: "qt_web_build".into(),
            label: "Build the web UI for Qt WebEngine".into(),
            description: format!(
                "Run npm run build in frontend/ (Web UI is {}). The built app must appear at frontend/dist/index.html — the Qt WebEngine widget loads this file at runtime; without it the window stays blank.",
                web
            ),
            command: "npm".into(),
            args: vec!["run".into(), "build".into()],
            working_dir: Some("frontend".into()),
            env: None,
            timeout_secs: Some(600),
            condition: Some(StepCondition::FileExists {
                path: "frontend/package.json".into(),
            }),
            on_error: ErrorMode::Skip,
            interactive: vec![],
        },
        Step::Command {
            id: "qt_cmake_configure".into(),
            label: "Configure Qt build with CMake".into(),
            description: format!(
                "Configure the Qt WebEngine application (cmake -S . -B build). Requires Qt6 with the WebEngine module (Qt6::WebEngineWidgets), CMake 3.16+ and a C++ compiler (MSVC, MinGW or g++/clang). The web UI must be built first (frontend/dist/index.html)."
            ),
            command: "cmake".into(),
            args: vec!["-S".into(), ".".into(), "-B".into(), "build".into()],
            working_dir: Some(".".into()),
            env: None,
            timeout_secs: Some(600),
            condition: Some(StepCondition::FileExists { path: cmake_cond }),
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "qt_cmake_build".into(),
            label: "Build Qt WebEngine application".into(),
            description: format!(
                "Compile the Qt WebEngine application (cmake --build build). Depends on the configured build/ and the built web UI (frontend/dist/index.html); failures usually mean missing Qt6 WebEngineWidgets dev files or a broken compiler toolchain."
            ),
            command: "cmake".into(),
            args: vec!["--build".into(), "build".into()],
            working_dir: Some(".".into()),
            env: None,
            timeout_secs: Some(1200),
            condition: Some(StepCondition::FileExists {
                path: "frontend/dist/index.html".into(),
            }),
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
    ]
}

/// Бандл-идентификатор для tauri init (--identifier): домен +
/// санитизированное имя проекта (только [a-zA-Z0-9-._], сегмент не
/// начинается с цифры — иначе CLI отвергает ввод).
pub fn tauri_identifier(project_name: &str) -> String {
    let mut base: String = project_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if base.is_empty() {
        base.push_str("app");
    }
    if base.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        base.insert(0, 'a');
    }
    format!("com.{}", base)
}

/// Шаг «scaffold» через Composer: command/args резолвятся через
/// composer_launch() — глобальный `composer` или `php <абс. composer.phar>`,
/// поэтому плейсхолдер ищется по фактическому положению в args.
/// Composer — creates_named_directory с временной папкой по умолчанию
/// (temp+move): "." не принимается в непустом каталоге.
/// Пост-условие — composer.json (валидный манифест каркаса, а не
/// package.json). Abort: без composer каркас не инициализируется —
/// префлайт уже проверил доступность и напечатал пути/команду.
pub fn composer_scaffold_step(id: &str, label: &str, desc: &str, package: &str) -> Step {
    let (command, prefix) = composer_launch();
    let mut args: Vec<String> = prefix;
    args.extend([
        "create-project".to_string(),
        package.to_string(),
        SCAFFOLD_TARGET.to_string(),
        "--no-interaction".to_string(),
        // Composer's dist downloader requires PHP's zip extension (or
        // unzip/7z). Prefer source so Laravel/Symfony still scaffold on
        // minimal Windows PHP installations.
        "--prefer-source".to_string(),
    ]);
    let mut step = scaffold_step(
        id,
        label,
        desc,
        &command,
        args.iter().map(String::as_str).collect(),
        ScaffoldCapability::CreatesNamedDirectory,
        ".",
        ScaffoldExtras::default()
            .expects(&["composer.json"])
            .policy(FilePolicy::SkipIfExists),
    );
    if let Step::Generate { on_error, .. } = &mut step {
        *on_error = ErrorMode::Abort;
    }
    step
}

/// Дополнительные параметры scaffold-шага (все опциональны; значения по
/// умолчанию — в ScaffoldGenerator: temp_dir_allowed по способности,
/// working_dir по способности, timeout 600).
#[derive(Default)]
pub struct ScaffoldExtras<'a> {
    working_dir: Option<&'a str>,
    temp_dir_allowed: Option<bool>,
    expected_outputs: Vec<&'a str>,
    interactive: Vec<serde_json::Value>,
    timeout_secs: Option<u64>,
    policy: Option<FilePolicy>,
}

impl<'a> ScaffoldExtras<'a> {
    /// Пост-условия: пути, которые обязаны появиться после завершения CLI
    /// (относительно каталога назначения). Провал любого — ошибка шага.
    pub fn expects(mut self, outputs: &[&'a str]) -> Self {
        self.expected_outputs.extend_from_slice(outputs);
        self
    }
    /// Интерактивные ответы: {"trigger": "...", "response_type": ...}.
    pub fn interact(mut self, entries: Vec<serde_json::Value>) -> Self {
        self.interactive = entries;
        self
    }
#[allow(dead_code)]
    pub fn in_dir(mut self, wd: &'a str) -> Self {
        self.working_dir = Some(wd);
        self
    }
    /// Явно разрешить/запретить временную папку (по умолчанию — по способности).
    pub fn temp_dir(mut self, allowed: bool) -> Self {
        self.temp_dir_allowed = Some(allowed);
        self
    }
#[allow(dead_code)]
    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
    /// Политика идемпотентности: SkipIfExists пропускает CLI, когда все
    /// expected_outputs уже на месте (повторный запуск рецепта).
    pub fn policy(mut self, policy: FilePolicy) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// Шаг «scaffold»: CLI-генератор, поведение которого задаёт ЯВНАЯ
/// способность (ScaffoldCapability). ScaffoldGenerator разбирается с
/// каталогом и пост-условиями сам (см. generators/mod.rs):
///   - creates_named_directory / creates_project_and_may_prompt: CLI
///     выполняется во временной папке temp_<target> (если разрешено),
///     содержимое (включая скрытые файлы) программно переносится в target;
///   - creates_in_current_directory: CLI работает ВНУТРИ target с ".".
/// Матрёшек testapp/testapp и пустых каркасов без node_modules нет.
#[allow(clippy::too_many_arguments)]
pub fn scaffold_step(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    capability: ScaffoldCapability,
    target_dir: &str,
    extras: ScaffoldExtras<'_>,
) -> Step {
    let mut config = serde_json::json!({
        "command": command,
        "args": args,
        "capability": capability.as_str(),
        "target_dir": target_dir,
    });
    if let Some(wd) = extras.working_dir {
        config["working_dir"] = serde_json::Value::String(wd.to_string());
    }
    if let Some(allowed) = extras.temp_dir_allowed {
        config["temp_dir_allowed"] = serde_json::Value::Bool(allowed);
    }
    if !extras.expected_outputs.is_empty() {
        config["expected_outputs"] = serde_json::Value::Array(
            extras
                .expected_outputs
                .iter()
                .map(|o| serde_json::Value::String(o.to_string()))
                .collect(),
        );
    }
    if !extras.interactive.is_empty() {
        config["interactive"] = serde_json::Value::Array(extras.interactive);
    }
    if let Some(secs) = extras.timeout_secs {
        config["timeout_secs"] = serde_json::Value::Number(secs.into());
    }
    Step::Generate {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        generator_id: "scaffold".into(),
        generator_config: config,
        policy: extras.policy,
        condition: None,
        on_error: ErrorMode::Skip,
    }
}

/// CRC-32 (IEEE 802.3): poly 0x04C11DB7 (отражённый 0xEDB88320),
/// init/xorout 0xFFFFFFFF — тот же алгоритм, что у std.hash.Crc32 в Zig.
/// Используется для поля `.fingerprint` в build.zig.zon (Zig 0.14+):
/// верхние 32 бита обязаны равняться crc32(имя_пакета).
pub fn crc32(data: &[u8]) -> u32 {
    const POLY: u32 = 0xEDB88320;
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Ключевые слова Zig — их нельзя использовать как имя пакета (`.name = .fn`
/// не распарсится). Зарезервированные слова, отсутствующие в этом списке,
/// в имя попасть не могут (список полный для 0.14).
const ZIG_KEYWORDS: &[&str] = &[
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "noalias",
    "noinline",
    "nosuspend",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "linksection",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
];

/// Валидное имя пакета Zig из имени проекта: нижний регистр, не-буквенно-
/// цифровые символы → '_', ≤32 байт (ограничение build.zig.zon), не ключевое
/// слово (иначе enum-literal `.name = .<keyword>` не распарсится).
pub fn zig_package_name(project_name: &str) -> String {
    let mut name: String = project_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    // обрезаем по байтам (имена пакетов — ASCII после санитизации)
    name.truncate(32);
    if name.trim_matches('_').is_empty() {
        name = "app".to_string();
    }
    if name.as_bytes()[0].is_ascii_digit() {
        name.insert(0, '_');
    }
    if ZIG_KEYWORDS.contains(&name.as_str()) {
        name.push('_');
    }
    name
}

/// Экранирование для строковых литералов Java/Kotlin в генерируемом коде
/// (кавычки, бэкслеш, `$` — шаблоны Kotlin).
pub fn string_literal_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Экранирование для XML-атрибутов (android:label в манифесте).
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Полный каркас Android-приложения (Gradle + MainActivity). Может
/// создаваться фреймворком "android" (java/kotlin, с Compose при связке
/// с jetpack-compose) или самим "jetpack-compose" (kotlin + Compose).
/// Все пути относительны — сегментация (frontend/) применяется движком.
pub fn android_steps(
    project_name: &str,
    project_path: &str,
    compose: bool,
    java_lang: bool,
    seg: Option<&str>,
) -> Vec<Step> {
    // Compose доступен только в Kotlin-модуле; при java-языке — обычный
    // Activity (связка android+compose+java не возникает в мастере).
    let compose = compose && !java_lang;
    let safe_name: String = project_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let label = xml_escape(project_name);
    let text = string_literal_escape(project_name);

    let wf = |id: &str, label: &str, path: &str, content: String| -> Step {
        Step::WriteFile {
            id: id.to_string(),
            label: label.to_string(),
            description: format!("Create {}", path),
            path: path.to_string(),
            content,
            overwrite: false,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    // Корневой build.gradle.kts: AGP + Kotlin (и Compose-плагин при
    // compose-проекте). Всё с apply false — приложения подключают плагины
    // в app/build.gradle.kts.
    let root_build = if compose {
        r#"plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}
"#
        .to_string()
    } else {
        r#"plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
"#
        .to_string()
    };

    let app_build = if compose {
        r#"plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
}
"#
        .to_string()
    } else if java_lang {
        r#"plugins {
    id("com.android.application")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
"#
        .to_string()
    } else {
        r#"plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}
"#
        .to_string()
    };

    // MainActivity: Compose-проект — ComponentActivity + setContent; иначе —
    // обычный Activity с TextView (без внешних зависимостей).
    let (main_path, main_content) = if compose {
        (
            "app/src/main/kotlin/com/example/app/MainActivity.kt".to_string(),
            format!(
                r#"package com.example.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

class MainActivity : ComponentActivity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)
        setContent {{
            MaterialTheme {{
                Surface(modifier = Modifier.fillMaxSize()) {{
                    Box(contentAlignment = Alignment.Center) {{
                        Text("Hello from {text}!")
                    }}
                }}
            }}
        }}
    }}
}}
"#
            ),
        )
    } else if java_lang {
        (
            "app/src/main/java/com/example/app/MainActivity.java".to_string(),
            format!(
                r#"package com.example.app;

import android.app.Activity;
import android.os.Bundle;
import android.widget.TextView;

public class MainActivity extends Activity {{
    @Override
    protected void onCreate(Bundle savedInstanceState) {{
        super.onCreate(savedInstanceState);
        TextView textView = new TextView(this);
        textView.setText("Hello from {text}!");
        setContentView(textView);
    }}
}}
"#
            ),
        )
    } else {
        (
            "app/src/main/kotlin/com/example/app/MainActivity.kt".to_string(),
            format!(
                r#"package com.example.app

import android.app.Activity
import android.os.Bundle
import android.widget.TextView

class MainActivity : Activity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)
        val textView = TextView(this)
        textView.text = "Hello from {text}!"
        setContentView(textView)
    }}
}}
"#
            ),
        )
    };

    vec![
        wf("android_settings", "Create settings.gradle.kts", "settings.gradle.kts",
            format!(r#"pluginManagement {{
    repositories {{
        google()
        mavenCentral()
        gradlePluginPortal()
    }}
}}

dependencyResolutionManagement {{
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {{
        google()
        mavenCentral()
    }}
}}

rootProject.name = "{safe_name}"
include(":app")
"#)),
        wf("android_root_build", "Create build.gradle.kts", "build.gradle.kts", root_build),
        wf("android_gradle_props", "Create gradle.properties", "gradle.properties",
            "org.gradle.jvmargs=-Xmx2048m -Dfile.encoding=UTF-8\nandroid.useAndroidX=true\nandroid.nonTransitiveRClass=true\nkotlin.code.style=official\n".to_string()),
        wf("android_app_build", "Create app/build.gradle.kts", "app/build.gradle.kts", app_build),
        wf("android_manifest", "Create AndroidManifest.xml", "app/src/main/AndroidManifest.xml",
            format!(r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application
        android:label="{label}"
        android:theme="@android:style/Theme.Material.Light.NoActionBar">
        <activity
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#)),
        wf("android_main", "Create MainActivity", &main_path, main_content),
        // Wrapper (gradlew) обязателен для ./gradlew assembleDebug — без него
        // у пользователя нет ни одной команды сборки, а Gradle может вообще
        // не стоять локально. Abort: отсутствие Gradle останавливает пайплайн
        // с понятной причиной, а не тихо скипает каркас.
        Step::Command {
            id: "android_gradle_wrapper".into(),
            label: "Generate Gradle wrapper".into(),
            description: "Create gradlew + gradle/wrapper for the Android project".into(),
            command: "gradle".into(),
            args: vec!["wrapper".into()],
            // Рабочая директория — корень проекта: into_segment добавит
            // сегмент (frontend/ в split-стеке) РОВНО ОДИН раз. Раньше
            // сегмент подставлялся здесь и повторно в into_segment —
            // получался несуществующий путь frontend/frontend.
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(300),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        // Пост-валидация: build.gradle.kts обязан декларировать Android-плагин
        // (и Compose-артефакты в compose-проекте) — «каркас» без плагина не
        // собирается.
        {
            let build_path = match seg {
                Some(dir) => format!("{}/build.gradle.kts", dir),
                None => "build.gradle.kts".to_string(),
            };
            let mut required = vec!["com.android.application"];
            if compose {
                required.push("androidx.compose.ui:ui");
            }
            preflight::manifest_check_step(
                "android_build_check",
                "Validate Android build.gradle.kts",
                &build_path,
                "gradle_kts",
                &required,
            )
        },
    ]
}

/// Раскрывает %VAR% в пути через переменные текущего процесса
/// (неизвестная переменная остаётся как есть).
pub fn expand_env_path(raw: &str) -> PathBuf {
    let mut out = raw.to_string();
    let mut guard = 0;
    while let Some(start) = out.find('%') {
        if guard > 10 {
            break;
        }
        guard += 1;
        let Some(end_rel) = out[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_rel;
        let name = &out[start + 1..end];
        if let Ok(value) = std::env::var(name) {
            out.replace_range(start..=end, &value);
        } else {
            break;
        }
    }
    PathBuf::from(out)
}

/// Есть ли команда в PATH (where/which)?
pub fn command_on_path(name: &str) -> bool {
    let (prog, arg) = if cfg!(target_os = "windows") {
        ("where", name)
    } else {
        ("which", name)
    };
    std::process::Command::new(prog)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Способ запуска Composer: (команда, префикс аргументов).
///
/// 1. Глобальный `composer` в PATH — используем его напрямую.
/// 2. Иначе — абсолютный путь к скачанному composer.phar в Toolchain store
///    (`%LOCALAPPDATA%\StackPilot\tools\php\composer.phar`) или в каталогах
///    установки (`%APPDATA%\Composer`, `%LOCALAPPDATA%\Programs\php`) и
///    запуск через `php <абсолютный путь>`.
///
/// Относительный `composer.phar` НЕ используется никогда: `php composer.phar`
/// ищет файл в рабочем каталоге и падает с «Could not open input file:
/// composer.phar» (движок выполняет CLI во временной папке, где phar нет).
pub fn composer_launch() -> (String, Vec<String>) {
    if command_on_path("composer") {
        return ("composer".to_string(), Vec::new());
    }
    // Единый список каталогов discovery (тот же, что печатает PHP-префлайт
    // в preflight.rs). PHP запускается с `-d extension=fileinfo` чтобы
    // Composer мог скачивать пакеты через stream wrappers даже если
    // расширение закомментировано в php.ini (но DLL доступна в extension_dir).
    for dir in preflight::COMPOSER_PHAR_DIRS {
        let phar = expand_env_path(dir).join("composer.phar");
        if phar.is_file() {
            return (
                "php".to_string(),
                vec![
                    "-d".to_string(),
                    "extension=fileinfo".to_string(),
                    phar.to_string_lossy().into_owned(),
                ],
            );
        }
    }
    // Ничего не нашли — честный fallback: php с абсолютным путём в Toolchain
    // store (ошибка установки будет явной, а не «Could not open input file»).
    let phar = expand_env_path("%LOCALAPPDATA%\\StackPilot\\tools\\php").join("composer.phar");
    (
        "php".to_string(),
        vec![
            "-d".to_string(),
            "extension=fileinfo".to_string(),
            phar.to_string_lossy().into_owned(),
        ],
    )
}

pub fn steps_for_framework_impl(
    fw: &str,
    project_path: &str,
    project_name: &str,
    context: &WizardContext,
    seg: Option<&str>,
) -> Vec<Step> {
    let registry = crate::modules::project_creator::engine::providers::ProviderRegistry::new();
    if let Some(provider) = registry.get(fw.to_lowercase().as_str()) {
        return provider.generate_steps(project_path, project_name, context, seg);
    }
    crate::modules::project_creator::engine::providers::legacy::legacy_match(fw, project_path, project_name, context, seg)
}

/// Каталог сегмента, где живёт python-код проекта: "backend" в
/// моно-репозитории (backend + frontend), "." — корень проекта.
/// Именно рядом с ним лежит requirements.txt и создаётся venv.
pub fn python_segment_dir(context: &WizardContext) -> String {
    if context.languages.iter().any(|l| l == "python") {
        if let Some(seg) = ProjectLayout::compute(context).language_dir("python") {
            return seg;
        }
    }
    ".".to_string()
}

/// Каталог, где лежит package.json JS-части проекта (для dep-патчей и
/// пост-валидации prisma/drizzle): сначала каталог JS-фреймворка, затем
/// каталог JS-языка (None — JS-части в проекте нет).
pub fn js_manifest_dir(context: &WizardContext) -> Option<String> {
    let layout = ProjectLayout::compute(context);
    for fw in &context.frameworks {
        if is_js_framework(fw) {
            return Some(layout.framework_dir(fw).unwrap_or_else(|| ".".to_string()));
        }
    }
    for lang in &context.languages {
        if matches!(lang.as_str(), "typescript" | "javascript") {
            return Some(layout.language_dir(lang).unwrap_or_else(|| ".".to_string()));
        }
    }
    None
}

/// Стандартные каталоги установки Python на Windows (шаблон с `*`
/// раскрывается в конкретный каталог, например `Python313`).
const PYTHON_WINDOWS_DIR_PATTERNS: [&str; 2] = [
    "C:/Program Files/Python3*",
    "%LOCALAPPDATA%/Programs/Python/Python3*",
];

/// Раскрывает %VAR% в известных шаблонах каталогов Python.
pub fn expand_python_dir_pattern(raw: &str) -> String {
    let mut out = raw.to_string();
    for (pattern, var) in [
        ("%LOCALAPPDATA%", "LOCALAPPDATA"),
        ("%ProgramFiles%", "ProgramFiles"),
    ] {
        if out.contains(pattern) {
            if let Ok(value) = std::env::var(var) {
                out = out.replace(pattern, &value);
            }
        }
    }
    out
}

/// «Естественно-числовой» ключ имени версионного каталога Python:
/// `Python313` → [3, 313], `Python3.11` → [3, 11] — сравнение числовое,
/// `Python313` старше `Python311`, а не лексикографически меньше.
pub fn python_dir_natural_key(name: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut num = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else if !num.is_empty() {
            out.push(num.parse().unwrap_or(0));
            num.clear();
        }
    }
    if !num.is_empty() {
        out.push(num.parse().unwrap_or(0));
    }
    out
}

/// Первая реальная `python.exe`/`python3.exe` из переданного PATH —
/// Microsoft Store-заглушки (WindowsApps) пропускаются: они «отвечают»
/// кодом 9009, а не версией интерпретатора.
pub fn python_from_path(path_str: &str) -> Option<String> {
    for entry in path_str.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let dir = std::path::Path::new(entry);
        for name in ["python.exe", "python3.exe"] {
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            if crate::platform::paths::is_windows_store_alias(&candidate) {
                continue;
            }
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Реальный интерпретатор Python на Windows (абсолютный путь), НЕ
/// Store-заглушка. Поиск:
///   1. известные каталоги установки (python.org, StackPilot Toolchain)
///      — самый новый `Python3x` из `C:/Program Files/` и
///      `%LOCALAPPDATA%/Programs/Python/`;
///   2. PATH — первая `python.exe`/`python3.exe`, КРОМЕ каталогов
///      WindowsApps (Microsoft Store aliases — заглушки, а не Python).
/// `None` — интерпретатор не найден: шаг пайплайна упадёт с понятной
/// ошибкой «python не найден» (а не с кодом 9009 Store-заглушки).
pub fn resolve_windows_python() -> Option<String> {
    let mut best: Option<(Vec<u64>, String)> = None;
    for pattern in PYTHON_WINDOWS_DIR_PATTERNS {
        let expanded = expand_python_dir_pattern(pattern);
        let Some((star_idx, _)) = expanded.char_indices().find(|(_, c)| *c == '*') else {
            continue;
        };
        let prefix: String = expanded[..star_idx].to_string();
        let parent = std::path::Path::new(&expanded[..star_idx]);
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
                continue;
            }
            let exe = path.join("python.exe");
            if !exe.is_file() {
                continue;
            }
            let key = python_dir_natural_key(&name);
            if best.as_ref().map_or(true, |(bk, _)| bk < &key) {
                best = Some((key, exe.to_string_lossy().into_owned()));
            }
        }
    }
    if let Some((_, exe)) = best {
        return Some(exe);
    }

    python_from_path(&std::env::var("PATH").unwrap_or_default())
}

/// Интерпретатор Python: на Windows — РЕАЛЬНЫЙ интерпретатор (абсолютный
/// путь, если найден; иначе `python`), на unix — `python3` (дистрибутивный).
/// ВСЕ python-шаги пайплайна используют ровно этот выбор — никаких жёстко
/// зашитых "python" там, где возможен unix.
pub fn python_command() -> String {
    if cfg!(target_os = "windows") {
        resolve_windows_python().unwrap_or_else(|| "python".to_string())
    } else {
        "python3".to_string()
    }
}

/// АБСОЛЮТНЫЙ путь к бинарю внутри venv проекта:
/// `<project_path>\venv\Scripts\<name>.exe` на Windows,
/// `<project_path>/venv/bin/<name>` на unix. В моно-репозитории venv живёт
/// в каталоге python-сегмента (`<project_path>\backend\venv\Scripts\alembic.exe`
/// / `<project_path>/backend/venv/bin/alembic`), а не в корне.
///
/// Правило движка: Python-утилиты (alembic, pip) вызываются ТОЛЬКО через
/// бинарники виртуального окружения — глобальный `alembic` не используется.
/// Путь обязан быть АБСОЛЮТНЫМ: относительный `<project>/backend/venv/...`
/// зависел бы от рабочего каталога процесса (executor запускает команды из
/// project_path, но venv создаётся ВНУТРИ каталога python-сегмента).
pub fn python_venv_bin(project_path: &str, python_dir: &str, name: &str) -> String {
    let base = PathBuf::from(project_path);
    let venv = if python_dir.is_empty() || python_dir == "." {
        base.join("venv")
    } else {
        base.join(python_dir).join("venv")
    };
    let bin = if cfg!(target_os = "windows") {
        venv.join("Scripts").join(format!("{}.exe", name))
    } else {
        venv.join("bin").join(name)
    };
    bin.to_string_lossy().into_owned()
}

/// Путь `migrations/env.py` относительно корня проекта — условие шага
/// alembic_env_patch (патч применяется только к реально созданному каркасу).
pub fn alembic_env_py_rel(python_dir: &str) -> String {
    if python_dir.is_empty() || python_dir == "." {
        "migrations/env.py".to_string()
    } else {
        format!("{}/migrations/env.py", python_dir)
    }
}

/// Скрипт-патч `migrations/env.py` (запускается интерпретатором venv из
/// каталога python-сегмента). `alembic init` создаёт шаблон, читающий
/// статический `sqlalchemy.url` из alembic.ini, — README и .env.example
/// обещают `DATABASE_URL` из окружения (.env / docker compose). Патч:
///   - идемпотентен (маркер STACKPILOT_DATABASE_URL);
///   - вставляет `config.set_main_option(...)` из `os.environ` сразу после
///     `config = context.config` (покрывает и offline-, и online-режим);
///   - экранирует `%` — configparser интерполирует его в значениях;
///   - если структура env.py иная (другая версия alembic) — печатает
///     предупреждение и завершается нулём: шаг (on_error=Skip) не должен
///     валить уже сгенерированный проект.
///
/// Скрипт ASCII-only: аргумент `-c` проходит через cmd.exe на Windows.
pub const ALEMBIC_DATABASE_URL_PATCH: &str = r##"import pathlib, sys

MARKER = 'STACKPILOT_DATABASE_URL'
path = pathlib.Path('migrations') / 'env.py'
if not path.is_file():
    print('alembic: migrations/env.py not found - nothing to patch')
    sys.exit(0)
text = path.read_text(encoding='utf-8')
if MARKER in text:
    print('alembic: migrations/env.py already reads DATABASE_URL')
    sys.exit(0)
anchor = 'config = context.config'
if anchor not in text:
    print('WARNING: alembic migrations/env.py has no "config = context.config" anchor - DATABASE_URL was not wired', file=sys.stderr)
    sys.exit(0)
patch = anchor + '''


# STACKPILOT_DATABASE_URL: connection string from the environment (.env / docker compose)
import os  # noqa: E402

_database_url = os.environ.get('DATABASE_URL')
if _database_url:
    # configparser interpolates %, so the URL must be escaped
    config.set_main_option('sqlalchemy.url', _database_url.replace('%', '%%'))
'''
path.write_text(text.replace(anchor, patch, 1), encoding='utf-8')
print('alembic: migrations/env.py patched - DATABASE_URL is used by migrations')
"##;

pub fn steps_for_tools(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let tools = &context.tools;
    let project_name = context.project_name.as_deref().unwrap_or("app");
    let mut steps = Vec::new();
    let mut infra_envs: Vec<String> = Vec::new();

    // Канонический порт приложения (порт, на котором слушает каркас) —
    // нужен локальным инфра-инструментам, чей штатный порт может совпасть
    // с ним (локальный Grafana на 3000 рядом с NestJS/Express на 3000).
    let app_port: u16 = ports::framework_default_port(
        context.frameworks.first().map(String::as_str).unwrap_or(""),
    )
    .unwrap_or(3000);

    let write_file = |id: &str, label: &str, path: &str, content: &str| -> Step {
        Step::WriteFile {
            id: id.to_string(),
            label: label.to_string(),
            description: format!("Create {}", path),
            path: path.to_string(),
            content: content.to_string(),
            overwrite: false,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    // Каталог python-кода (backend/ в моно-репозитории, иначе корень) —
    // нужен шагам alembic ниже. Канонический venv + установка манифеста
    // выполняются в compose_recipe ДО всех framework-шагов (preflight.rs) —
    // здесь окружение не создаётся и не конкурирует с ним.
    let python_dir = python_segment_dir(context);

    for tool_id in tools {
        match tool_id.as_str() {
            // Database tools
            "sqlalchemy" => {
                // database.py читает DATABASE_URL из окружения (docker-compose
                // передаёт его app-сервису, локально — .env через python-dotenv),
                // а не хардкодит учётные данные. Путь учитывает каталог python-
                // сегмента (backend/ в mono-репозитории).
                let db_path = if python_dir == "." {
                    "src/database.py".to_string()
                } else {
                    format!("{}/src/database.py", python_dir)
                };
                steps.push(write_file(
                    "sqlalchemy_config",
                    "SQLAlchemy config",
                    &db_path,
                    r#"import os
from dotenv import load_dotenv
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker, DeclarativeBase

load_dotenv()

DATABASE_URL = os.getenv("DATABASE_URL")
if not DATABASE_URL:
    raise RuntimeError(
        "DATABASE_URL is not set. Copy .env.example to .env and fill in the connection string."
    )

engine = create_engine(DATABASE_URL)
SessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)

class Base(DeclarativeBase):
    pass

def get_db():
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()
"#,
                ));
                // .env.example: DATABASE_URL уже приходит из postgresql-инструмента
                // (get_env_example). Без него (например sqlalchemy + mysql) —
                // добавляем строку сами, иначе у пользователя нет ни одной
                // подсказки для строки подключения.
                if !tools.contains(&"postgresql".to_string()) {
                    infra_envs.push(
                        "# Database connection string (adjust for your DB engine)\nDATABASE_URL=postgresql://postgres:12345@localhost:5432/postgres\n"
                            .to_string(),
                    );
                }
            }
            "alembic" => {
                // Каталог python-сегмента (backend/ в моно-репозитории,
                // корень в монолите): alembic init создаёт migrations/ рядом
                // с venv и requirements.txt, а не в корне проекта.
                let alembic_wd = if python_dir == "." {
                    project_path.to_string()
                } else {
                    format!(
                        "{}/{}",
                        project_path.trim_end_matches(['/', '\\']),
                        python_dir
                    )
                };
                steps.push(Step::Command {
                    id: "alembic_init".into(),
                    label: "Init Alembic".into(),
                    description: "Initialize Alembic migrations (inside project venv)".into(),
                    // Вызывается строго через бинарь виртуального окружения
                    // (абсолютный путь: <project>\backend\venv\Scripts\alembic.exe
                    // / <project>/backend/venv/bin/alembic внутри python-
                    // сегмента): py_pip_install (с гарантированным alembic)
                    // отрабатывает ДО этого шага — см. выше.
                    command: python_venv_bin(project_path, &python_dir, "alembic"),
                    args: vec!["init".into(), "migrations".into()],
                    working_dir: Some(alembic_wd.clone()),
                    env: None,
                    timeout_secs: Some(60),
                    condition: None,
                    // Abort вместо Skip: alembic обязан быть в venv после
                    // py_pip_install, поэтому провал — реальная ошибка,
                    // а не «тихо провалившийся» шаг генерации.
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                });
                // alembic init создаёт env.py, читающий статический
                // sqlalchemy.url из alembic.ini, — README и .env.example
                // обещают DATABASE_URL из окружения (.env / docker compose).
                steps.push(Step::Command {
                    id: "alembic_env_patch".into(),
                    label: "Wire Alembic to DATABASE_URL".into(),
                    description: "Patch migrations/env.py to read DATABASE_URL from the environment"
                        .into(),
                    command: python_venv_bin(project_path, &python_dir, "python"),
                    args: vec!["-c".into(), ALEMBIC_DATABASE_URL_PATCH.into()],
                    working_dir: Some(alembic_wd),
                    env: None,
                    timeout_secs: Some(30),
                    // Патч применяется только к реально созданному каркасу:
                    // если alembic init был пропущен/неудачен, migrations/env.py
                    // отсутствует и шаг скипается (без вторичных ошибок).
                    condition: Some(StepCondition::FileExists {
                        path: alembic_env_py_rel(&python_dir),
                    }),
                    // Skip: патч — улучшение конфигурации, его провал не
                    // должен валить уже сгенерированный проект; скрипт сам
                    // печатает предупреждение, если структура env.py иная.
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
            }
            "prisma" => {
                // `npx prisma init` (без версии) скачивает ПОСЛЕДНЮЮ версию
                // CLI и в Prisma 8+ упал бы: там удалены флаги
                // --datasource-provider и --no-skills (теперь --skills=none).
                // Версия пинится на 6.x — она совпадает с декларируемой в
                // package.json зависимостью (^6.1.0), не интерактивна под
                // CI=1 и генерирует классическую раскладку
                // (prisma/schema.prisma + .env). Флага --no-skills у 6.x нет,
                // поэтому агентные артефакты при необходимости чистит
                // Rust-генератор prisma_cleanup ниже.
                let provider = if context.tools.iter().any(|t| t == "postgresql") {
                    "postgresql"
                } else if context.tools.iter().any(|t| t == "mysql") {
                    "mysql"
                } else if context.tools.iter().any(|t| t == "mongodb") {
                    "mongodb"
                } else {
                    "sqlite"
                };
                // Prisma-зависимости ДО init: init читает package.json и
                // добавляет prisma-скрипты; патч гарантирует, что prisma и
                // @prisma/client попадут в npm install финальной фазы (а не
                // только транзитно через npx). Патч применяется к package.json
                // JS-сегмента, когда он существует (FileExists-условие).
                let prisma_js_dir = js_manifest_dir(context);
                if let Some(dir) = &prisma_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_patch_step(
                        "prisma_deps",
                        "Add Prisma dependencies",
                        &pkg,
                        serde_json::json!({
                            "dependencies": { "@prisma/client": "^6.1.0" },
                            "devDependencies": { "prisma": "^6.1.0" }
                        })
                        .to_string(),
                    ));
                }
                steps.push(Step::Command {
                    id: "prisma_init".into(),
                    label: "Init Prisma".into(),
                    description: "Initialize Prisma ORM".into(),
                    command: "npx".into(),
                    args: vec![
                        "--yes".into(),
                        "prisma@6".into(),
                        "init".into(),
                        "--datasource-provider".into(),
                        provider.into(),
                    ],
                    // init читает/патчит package.json JS-сегмента (backend/
                    // в split-раскладке): рабочая директория — каталог
                    // сегмента, иначе prisma/ + schema.prisma ложились бы в
                    // корень мимо манифеста.
                    working_dir: Some(
                        prisma_js_dir
                            .as_ref()
                            .map(|dir| {
                                if dir == "." {
                                    project_path.to_string()
                                } else {
                                    format!(
                                        "{}/{}",
                                        project_path.trim_end_matches(['/', '\\']),
                                        dir
                                    )
                                }
                            })
                            .unwrap_or_else(|| project_path.to_string()),
                    ),
                    env: None,
                    timeout_secs: Some(120),
                    condition: None,
                    // Abort: выбранный инструмент обязан инициализироваться;
                    // невозможность запустить npx (нет npm) не маскируется
                    // молчаливым пропуском.
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                });
                // Подстраховка для версий Prisma без флага --no-skills
                // (или если флаг проигнорирован): движок на Rust принудительно
                // удаляет агентные артефакты из корня проекта. Папки
                // удаляются только при наличии маркера skills-lock.json —
                // пользовательские .claude/.windsurf не трогаются.
                steps.push(Step::Generate {
                    id: "prisma_cleanup".into(),
                    label: "Clean up Prisma AI skills".into(),
                    description: "Remove Prisma agent skill directories (.agents, .claude, .windsurf, skills-lock.json)".into(),
                    generator_id: "fs-cleanup".into(),
                    generator_config: serde_json::json!({
                        "paths": [".agents", ".claude", ".windsurf", "skills-lock.json"]
                    }),
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
                // Пост-валидация: prisma обязана быть задекларирована в
                // package.json JS-сегмента после init (Abort).
                if let Some(dir) = &prisma_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_check_step(
                        "prisma_deps_check",
                        "Validate Prisma dependencies",
                        &pkg,
                        "package_json",
                        &["@prisma/client", "prisma"],
                    ));
                }
            }
            "drizzle" => {
                // Конфиг лежит в каталоге JS-сегмента (backend/ в split-
                // раскладке) рядом с package.json — schema/out-пути внутри
                // него относительны каталога конфига.
                let drizzle_path = match js_manifest_dir(context) {
                    Some(dir) if dir != "." => format!("{}/drizzle.config.ts", dir),
                    _ => "drizzle.config.ts".to_string(),
                };
                steps.push(write_file(
                    "drizzle_config",
                    "Drizzle config",
                    &drizzle_path,
                    r#"import type { Config } from "drizzle-kit";

export default {
  schema: "./src/db/schema.ts",
  out: "./drizzle",
  driver: "pg",
  dbCredentials: {
    connectionString: process.env.DATABASE_URL!,
  },
} satisfies Config;
"#,
                ));
                // drizzle-kit init требует установленные drizzle-kit и
                // drizzle-orm: патч декларирует их в package.json
                // JS-сегмента (MergeJson, только когда файл существует) и
                // пост-валидация подтверждает наличие после npm install.
                let drizzle_js_dir = js_manifest_dir(context);
                if let Some(dir) = &drizzle_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_patch_step(
                        "drizzle_deps",
                        "Add Drizzle dependencies",
                        &pkg,
                        serde_json::json!({
                            "dependencies": { "drizzle-orm": "^0.36.0" },
                            "devDependencies": { "drizzle-kit": "^0.28.0" }
                        })
                        .to_string(),
                    ));
                    steps.push(preflight::manifest_check_step(
                        "drizzle_deps_check",
                        "Validate Drizzle dependencies",
                        &pkg,
                        "package_json",
                        &["drizzle-orm", "drizzle-kit"],
                    ));
                }
            }
            // Testing tools
            "pytest" => {
                // Конфиг и каталог тестов живут в python-сегменте (backend/
                // в mono-репозитории) рядом с requirements.txt и venv — иначе
                // `python -m pytest` из backend/ не видит ни конфиг, ни тесты.
                let ini_path = if python_dir == "." {
                    "pytest.ini".to_string()
                } else {
                    format!("{}/pytest.ini", python_dir)
                };
                let tests_path = if python_dir == "." {
                    "tests".to_string()
                } else {
                    format!("{}/tests", python_dir)
                };
                steps.push(write_file(
                    "pytest_config",
                    "Pytest config",
                    &ini_path,
                    r#"[pytest]
testpaths = tests
python_files = test_*.py
python_classes = Test*
python_functions = test_*
addopts = -v --tb=short
"#,
                ));
                steps.push(Step::CreateDirectory {
                    id: "create_tests_dir".into(),
                    label: "Create tests/".into(),
                    description: "Create tests directory".into(),
                    path: tests_path,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
                // Smoke-тест: без него `pytest` на свежем проекте завершается
                // кодом 5 («no tests collected») и CI сразу красный. Тест
                // тривиальный и перезаписывается пользователем по мере надобности
                // (overwrite=false — существующий файл не трогается).
                let smoke_path = if python_dir == "." {
                    "tests/test_smoke.py".to_string()
                } else {
                    format!("{}/tests/test_smoke.py", python_dir)
                };
                steps.push(write_file(
                    "pytest_smoke_test",
                    "Pytest smoke test",
                    &smoke_path,
                    r#"def test_smoke():
    """Первая проверка: pytest настроен и запускается."""
    assert True
"#,
                ));
            }
            "ruff" => {
                let ruff_path = if python_dir == "." {
                    "ruff.toml".to_string()
                } else {
                    format!("{}/ruff.toml", python_dir)
                };
                steps.push(write_file(
                    "ruff_config",
                    "Ruff config",
                    &ruff_path,
                    r#"[lint]
select = ["E", "F", "I", "N", "W"]
ignore = []

[format]
quote-style = "double"
indent-style = "space"
"#,
                ));
            }
            "airflow" => {
                infra_envs.push(content::get_env_example("airflow"));
                steps.push(Step::CreateDirectory {
                    id: "create_dags_dir".into(),
                    label: "Create dags/".into(),
                    description: "Create Airflow DAGs directory".into(),
                    path: "dags".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
                steps.push(write_file(
                    "airflow_example_dag",
                    "Example Airflow DAG",
                    "dags/example_dag.py",
                    r#"from datetime import datetime, timedelta

from airflow import DAG
from airflow.operators.python import PythonOperator

default_args = {"owner": "user", "retries": 1, "retry_delay": timedelta(minutes=5)}

with DAG(
    dag_id="example_dag",
    default_args=default_args,
    schedule="@daily",
    start_date=datetime(2024, 1, 1),
    catchup=False,
    tags=["example"],
) as dag:

    def print_hello() -> None:
        print("Hello from StackPilot Airflow!")

    hello = PythonOperator(task_id="print_hello", python_callable=print_hello)

    hello
"#,
                ));
            }
            // Infra tools — сервисы docker-compose; переменные окружения
            // собираем в один .env.example в конце (иначе каждый следующий
            // инструмент видел бы существующий файл и шаг скипался).
            // Инструменты из local_infra_tools поставлены локально:
            // для них в .env.example — локальные адреса (localhost),
            // а из docker-compose.yaml они исключаются (steps_for_docker).
            "postgresql" | "redis" | "mongodb" | "mysql" | "kafka" | "clickhouse" | "rabbitmq"
            | "minio" | "mailpit" => {
                if context.local_infra_tools.contains(tool_id) {
                    infra_envs.push(content::get_local_env_example(tool_id, app_port));
                } else {
                    infra_envs.push(content::get_env_example(tool_id));
                }
            }
            "grafana" => {
                if context.local_infra_tools.contains(tool_id) {
                    infra_envs.push(content::get_local_env_example(tool_id, app_port));
                } else {
                    infra_envs.push(content::get_env_example(tool_id));
                }
                // Реальный provisioning-конфиг Grafana: датасорсы для каждого
                // выбранного БД-инструмента + каталоги дашбордов. Учётные
                // данные совпадают с docker-compose.yaml (контейнеры).
                let mut ds_lines = String::new();
                for t in tools {
                    match t.as_str() {
                        "postgresql" => ds_lines.push_str(
                            r#"      - name: postgres
        type: postgres
        access: proxy
        url: postgres:5432
        database: postgres
        user: postgres
        secureJsonData:
          password: "12345"
        jsonData:
          sslmode: disable
"#,
                        ),
                        "mysql" => ds_lines.push_str(
                            r#"      - name: mysql
        type: mysql
        access: proxy
        url: mysql:3306
        database: mydb
        user: root
        secureJsonData:
          password: "root_pwd"
"#,
                        ),
                        "mongodb" => ds_lines.push_str(
                            r#"      - name: mongodb
        type: grafana-mongodb-datasource
        access: proxy
        url: mongo:27017
        jsonData:
          defaultAuthType: "NONE"
"#,
                        ),
                        "clickhouse" => ds_lines.push_str(
                            r#"      - name: clickhouse
        type: vertamedia-clickhouse-datasource
        access: proxy
        url: http://clickHouse:8123
"#,
                        ),
                        _ => {}
                    }
                }
                steps.push(write_file(
                    "grafana_datasources",
                    "Grafana datasource provisioning",
                    "config/grafana/provisioning/datasources/datasources.yaml",
                    &format!(
                        r#"apiVersion: 1

datasources:
{}
"#,
                        ds_lines
                    ),
                ));
                steps.push(write_file(
                    "grafana_dashboards",
                    "Grafana dashboards provisioning",
                    "config/grafana/provisioning/dashboards/dashboards.yaml",
                    r#"apiVersion: 1

providers:
  - name: "default"
    orgId: 1
    folder: ""
    type: file
    disableDeletion: false
    allowUiUpdates: true
    options:
      path: /var/lib/grafana/dashboards
"#,
                ));
                steps.push(Step::CreateDirectory {
                    id: "grafana_dashboards_dir".into(),
                    label: "Create Grafana dashboards dir".into(),
                    description: "Create config/grafana/dashboards/ for custom dashboards".into(),
                    path: "config/grafana/dashboards".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }
            "opentelemetry" => {
                // Реальный конфиг OpenTelemetry Collector (OTLP-приёмник →
                // консоль): сервис docker-compose (otel-collector) монтирует
                // этот файл в контейнер. Раньше здесь был md-хинт
                // «See documentation for setup details».
                infra_envs.push("OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318\n".to_string());
                steps.push(write_file(
                    "otel_collector",
                    "OpenTelemetry Collector config",
                    "config/otel-collector.yaml",
                    r#"receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch:

exporters:
  logging:
    verbosity: detailed

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [logging]
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [logging]
    logs:
      receivers: [otlp]
      processors: [batch]
      exporters: [logging]
"#,
                ));
            }
            "dbt" => {
                // Реальный dbt-проект: dbt_project.yml + profiles.yml (учётные
                // данные — из переменных окружения) + модель-пример. Пакеты
                // dbt-core/dbt-postgres попадают в requirements.txt
                // (preflight.python_manifest_lines); пост-валидация — по
                // dbt_project.yml (manifest-check "yaml").
                let safe_name: String = project_name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            c.to_ascii_lowercase()
                        } else {
                            '_'
                        }
                    })
                    .collect();
                steps.push(write_file(
                    "dbt_project",
                    "dbt project config",
                    "dbt_project.yml",
                    &format!(
                        r#"name: '{safe_name}'
version: '1.0.0'
config-version: 2
profile: '{safe_name}'

model-paths: ["models"]
analysis-paths: ["analyses"]
test-paths: ["tests"]
seed-paths: ["seeds"]
macro-paths: ["macros"]
snapshot-paths: ["snapshots"]

target-path: "target"
clean-targets:
  - "target"
  - "dbt_packages"

models:
  {safe_name}:
    +materialized: view
"#,
                    ),
                ));
                steps.push(write_file(
                    "dbt_profiles",
                    "dbt profiles",
                    "profiles.yml",
                    &format!(
                        r#"# Пользовательские профили dbt (обычно хранятся в ~/.dbt/).
# Учётные данные читаются из переменных окружения — заполните их в .env.
{safe_name}:
  target: dev
  outputs:
    dev:
      type: postgres
      host: "{{{{ env_var('POSTGRES_HOST', 'localhost') }}}}"
      port: 5432
      user: "{{{{ env_var('POSTGRES_USER', 'postgres') }}}}"
      password: "{{{{ env_var('POSTGRES_PASSWORD', '') }}}}"
      dbname: "{{{{ env_var('POSTGRES_DB', 'postgres') }}}}"
      schema: public
      threads: 4
"#,
                    ),
                ));
                steps.push(write_file(
                    "dbt_model",
                    "dbt example model",
                    "models/example.sql",
                    r#"-- Пример модели: выборка из таблицы sources.
-- Дополните моделями под ваши источники и запустите: dbt run
SELECT
    current_date AS report_date,
    'hello from dbt' AS message
"#,
                ));
                steps.push(preflight::manifest_check_step(
                    "dbt_project_check",
                    "Validate dbt_project.yml",
                    "dbt_project.yml",
                    "yaml",
                    &["name", "profile"],
                ));
            }
            "terraform" => {
                // Реальный Terraform-проект для локальной инфраструктуры
                // (docker-провайдер совпадает с docker-compose): main.tf +
                // переменные + пример tfvars. `terraform init` обязан
                // выполниться (Abort) — выбранный инструмент не маскируется
                // «тихим» скипом.
                steps.push(write_file(
                    "tf_main",
                    "Terraform main config",
                    "terraform/main.tf",
                    r#"# Локальная инфраструктура проекта через docker-провайдер.
# Переменные задаются в terraform/terraform.tfvars (см. terraform.tfvars.example).
terraform {
  required_version = ">= 1.5"
  required_providers {
    docker = {
      source  = "kreuzwerker/docker"
      version = "~> 3.0"
    }
  }
}

provider "docker" {}

resource "docker_container" "example" {
  name  = "stackpilot_example"
  image = "nginx:alpine"
  ports {
    internal = 80
    external = var.app_port
  }
}

output "example_url" {
  value = "http://localhost:${var.app_port}"
}
"#,
                ));
                steps.push(write_file(
                    "tf_variables",
                    "Terraform variables",
                    "terraform/variables.tf",
                    r#"variable "app_port" {
  description = "Порт, на который публикуется пример-контейнер"
  type        = number
  default     = 8081
}
"#,
                ));
                steps.push(write_file(
                    "tf_tfvars_example",
                    "Terraform tfvars example",
                    "terraform/terraform.tfvars.example",
                    "# Скопируйте в terraform.tfvars и заполните под ваш стек\napp_port = 8081\n",
                ));
                steps.push(Step::Command {
                    id: "terraform_init".into(),
                    label: "Init Terraform".into(),
                    description: "Run terraform init (downloads the docker provider; requires Terraform installed)".into(),
                    command: "terraform".into(),
                    args: vec!["init".into()],
                    working_dir: Some(format!("{}/terraform", project_path)),
                    env: None,
                    timeout_secs: Some(300),
                    condition: None,
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                });
                steps.push(preflight::manifest_check_step(
                    "terraform_check",
                    "Validate Terraform main.tf",
                    "terraform/main.tf",
                    "terraform",
                    &["provider \"docker\""],
                ));
            }
            "firebase" => {
                // Реальный конфиг Firebase Hosting + Firestore/Security Rules:
                // firebase.json + правила БД и Storage. CLI (firebase-tools)
                // подключается по Firebase-проекту — инструкция в README.
                steps.push(write_file(
                    "firebase_config",
                    "Firebase config",
                    "firebase.json",
                    r#"{
  "hosting": {
    "public": "frontend",
    "ignore": [
      "firebase.json",
      "**/.*",
      "**/node_modules/**"
    ],
    "rewrites": [
      {
        "source": "**",
        "destination": "/index.html"
      }
    ]
  },
  "firestore": {
    "rules": "firestore.rules",
    "indexes": "firestore.indexes.json"
  },
  "storage": {
    "rules": "storage.rules"
  }
}
"#,
                ));
                steps.push(write_file(
                    "firestore_rules",
                    "Firestore rules",
                    "firestore.rules",
                    r#"rules_version = '2';
service cloud.firestore {
  match /databases/{database}/documents {
    match /{document=**} {
      // Закомментируйте и настройте под свой доступ: allow read, write: if true;
      allow read, write: if false;
    }
  }
}
"#,
                ));
                steps.push(write_file(
                    "storage_rules",
                    "Storage rules",
                    "storage.rules",
                    r#"rules_version = '2';
service firebase.storage {
  match /b/{bucket}/o {
    match /{allPaths=**} {
      allow read, write: if false;
    }
  }
}
"#,
                ));
                steps.push(preflight::manifest_check_step(
                    "firebase_check",
                    "Validate firebase.json",
                    "firebase.json",
                    "firebase",
                    &["hosting", "firestore"],
                ));
            }
            "npm" | "gradle" | "maven" => {
                // Инструменты сборки — уже учтены в language/framework
                // Можно пропустить или добавить файлы конфигурации
            }
            "docker" => {
                // Сам инструмент «docker» (containerization): Dockerfile,
                // .dockerignore и docker-compose.yaml для app-сервиса
                // генерирует steps_for_docker — включая случай, когда
                // выбран только docker-инструмент без БД (requires_docker
                // у него false, поэтому context.docker сам по себе не
                // поднимается). Отдельная конфигурация не нужна.
            }
            "sqlite" => {
                // SQLite — встроенная БД: сервис и переменные окружения
                // не нужны (файл базы создаёт приложение). Используется
                // prisma-провайдером по умолчанию, когда БД-сервис не выбран.
            }
            _ => {
                // Для неизвестных — просто создаём директорию config/
                // (защитный fallback; все выбираемые инструменты мастера
                // имеют явные ветки — см. audit-тест selectable_tools_*).
                steps.push(Step::CreateDirectory {
                    id: format!("config_dir_{}", tool_id),
                    label: format!("Create config dir for {}", tool_id),
                    description: format!("Create configuration directory for {}", tool_id),
                    path: "config".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }
        }
    }

    // Telegram-боты (aiogram/telegraf): токен читается сгенерированным кодом
    // из окружения — он обязан быть в .env.example, иначе у пользователя нет
    // подсказки. Push-ится отдельно от инфра-сервисов.
    let has_telegram = context
        .frameworks
        .iter()
        .any(|f| f == "aiogram" || f == "telegraf");
    if has_telegram {
        infra_envs.push(
            "# Telegram bot token from @BotFather\nTELEGRAM_BOT_TOKEN=your_bot_token\n".to_string(),
        );
    }

    // Один .env.example на все инфра-сервисы (пишется один раз — в цикле
    // выше шаги для каждого инструмента по отдельности скипались бы).
    if !infra_envs.is_empty() {
        let mut combined = String::new();
        for env in &infra_envs {
            combined.push_str(env);
            combined.push('\n');
        }
        steps.push(write_file(
            "env_example",
            "Create .env.example",
            ".env.example",
            &combined,
        ));
    }

    // Локально установленные инфра-инструменты: инструкция по запуску
    // (LOCAL_INFRA.md) — как поднять сервис и куда он смотрит.
    if !context.local_infra_tools.is_empty() {
        steps.push(write_file(
            "local_infra_guide",
            "Create LOCAL_INFRA.md",
            "LOCAL_INFRA.md",
            &content::generate_local_infra_guide(&context.local_infra_tools, app_port),
        ));
    }

    steps
}

/// Фаза 5 (часть): Docker-шаблоны пишутся ПОСЛЕ всех CLI-фреймворков и
/// перезаписывают их версии (overwrite=true). Каталоги берутся из
/// канонической раскладки: в split-проекте Dockerfile и .dockerignore живут
/// ВНУТРИ backend/ (там серверное приложение, и docker-compose собирает
/// контекст ./backend), в integrated/одно-сторонних — в корне.
pub fn steps_for_docker(
    layout: &ProjectLayout,
    context: &WizardContext,
    _project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    // Локально установленные инфра-инструменты исключаются из docker-compose:
    // их сервисы уже запущены на машине, контейнер просто займёт порт.
    let docker_tools: Vec<String> = context
        .tools
        .iter()
        .filter(|t| !context.local_infra_tools.contains(t))
        .cloned()
        .collect();
    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    // Канонический порт приложения: единая таблица в ports.rs, чтобы
    // docker-compose, Dockerfile и devlauncher-профили не расходились.
    // Инфра-сервисы (airflow, grafana) ремапятся вокруг него в
    // collect_docker_services.
    let app_port: u16 = ports::framework_default_port(primary_fw.unwrap_or("")).unwrap_or(3000);
    let services = content::collect_docker_services(&docker_tools, app_port);
    // Контейнерная фаза активна, если (а) поднят флаг context.docker (выбран
    // любой requires_docker инструмент), (б) выбран инструмент «docker»
    // (containerization), либо (в) выбранные инструменты разворачивают
    // контейнеры (opentelemetry → otel-collector, grafana, airflow и т.д.).
    let docker_phase_active =
        context.docker || context.tools.iter().any(|t| t == "docker") || !services.is_empty();
    if !docker_phase_active {
        return Vec::new();
    }
    // Split: сервер живёт в backend/ — Dockerfile собирается оттуда.
    let app_dir = layout
        .eager_dirs()
        .iter()
        .find(|d| *d == "backend")
        .map(|d| format!("{}/", d))
        .unwrap_or_default();

    let mut result = Vec::new();

    // Для части языков/фреймворков (rust+clap, go+cobra, java без spring-boot,
    // php без laravel/symfony и т.д.) Dockerfile не генерируется вообще — тогда
    // app-сервис docker-compose не должен ссылаться на несуществующий build
    // (docker compose up --build упал бы на отсутствующем Dockerfile). Держим
    // include_app = generate_dockerfile_content вернул Some.
    let app_dockerfile =
        content::generate_dockerfile_content(primary_lang, primary_fw, project_name);

    if let Some(dockerfile_content) = app_dockerfile.clone() {
        result.push(Step::WriteFile {
            id: "dockerfile".into(),
            label: "Create Dockerfile".into(),
            description: format!("Create Dockerfile for {} + {:?}", primary_lang, primary_fw),
            path: format!("{}Dockerfile", app_dir),
            content: dockerfile_content,
            overwrite: true,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        })
    };
    result.push(Step::WriteFile {
        id: ("docker_ignore".into()),
        label: ("Create .dockerignore".into()),
        description: ("Generate .dockerignore file".into()),
        path: (format!("{}.dockerignore", app_dir)),
        content: (content::dockerignore_content(primary_lang)),
        overwrite: (true),
        policy: (None),
        condition: (None),
        on_error: (ErrorMode::Skip),
    });

    // App-сервис попадает в compose, только если генерируется Dockerfile
    // (app_dockerfile.is_some()). Инфра-сервисы (БД/кеши) — по мере наличия.
    result.push(Step::WriteFile {
        id: "docker_compose".into(),
        label: "Create docker-compose".into(),
        description: "Generate docker-compose file".into(),
        path: "docker-compose.yaml".into(),
        content: content::generate_docker_compose(
            &services,
            project_name,
            &app_port.to_string(),
            &app_dir,
            app_dockerfile.is_some(),
        ),
        overwrite: true,
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    result
}

/// Фаза 4: git init. Выполняется ДО шаблонизации (фаза 5) — чтобы
/// README/конфиги, написанные позже, попали в стартовый коммит.
///
/// Шаг удаления вложенных `.git`: Windows сохраняет историческую
/// PowerShell-команду; Unix — `find -mindepth 2`, который находит вложенные
/// репозитории на любой глубине и НЕ трогает корневой `.git` проекта
/// (прежняя PowerShell-команда на Linux молча скипалась, и `git add .`
/// падал на вложенных репозиториях).
pub fn git_cleanup_nested_step(project_path: &str) -> Step {
    if cfg!(target_os = "windows") {
        let root_git = format!("{}\\.git", project_path);
        Step::Command {
            id: "git_cleanup_nested".into(),
            label: "Remove nested Git repositories".into(),
            description: "Remove nested .git directories left by generators".into(),
            command: format!(
                r#"Get-ChildItem -LiteralPath '{project_path}' -Recurse -Force -Directory -Filter '.git' | Where-Object {{ $_.FullName -ne '{root_git}' }} | Remove-Item -Recurse -Force"#
            ),
            args: vec![],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    } else {
        Step::Command {
            id: "git_cleanup_nested".into(),
            label: "Remove nested Git repositories".into(),
            description: "Remove nested .git directories left by generators".into(),
            command: "find".into(),
            args: vec![
                project_path.to_string(),
                "-mindepth".into(),
                "2".into(),
                "-name".into(),
                ".git".into(),
                "-type".into(),
                "d".into(),
                "-prune".into(),
                "-exec".into(),
                "rm".into(),
                "-rf".into(),
                "{}".into(),
                "+".into(),
            ],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(30),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    }
}

pub fn steps_for_git_init(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();

    if !context.git_init {
        return steps;
    }

    // Генераторы (create-electron-app, flutter create и т.п.) часто сами
    // инициализируют git во вложенных каталогах — `git add .` потом падает
    // с «'dir' does not have a commit checked out». Убираем вложенные .git,
    // корневой (созданный ранее пользователем или нами) не трогаем.
    steps.push(git_cleanup_nested_step(project_path));

    // git init. Повторный запуск рецепта: .git/HEAD уже есть — шаг
    // пропускается (FileNotExists), git не переинициализируется.
    steps.push(Step::Command {
        id: "git_init".into(),
        label: "Initialize Git repository".into(),
        description: "Run git init".into(),
        command: "git".into(),
        args: vec!["init".into()],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: Some(StepCondition::FileNotExists {
            path: ".git/HEAD".into(),
        }),
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });

    steps
}

/// Команда git commit, идемпотентная при повторном запуске рецепта:
/// коммитит только когда в индексе есть изменения (`git diff --cached`),
/// и всегда завершается успешно (exit 0) — «нечего коммитить» не ошибка.
/// Git-сообщение экранируется под оболочку платформы.
pub fn git_commit_command(project_name: &str) -> String {
    let message = format!("Initial commit: {} project", project_name);
    if cfg!(windows) {
        let msg = message.replace('\'', "''");
        format!("git diff --cached --quiet; if (-not $?) {{ git commit -m '{msg}' }}; exit 0")
    } else {
        let msg = message.replace('\'', "'\"'\"'");
        format!("git diff --cached --quiet || git commit -m \"{msg}\"; exit 0")
    }
}

/// Фаза 5 (часть): .gitignore пишется в самой поздней фазе шаблонизации,
/// ПОСЛЕ всех CLI-фреймворков (create-next-app создаёт свой .gitignore —
/// наш шаблон обязан перезаписать его, overwrite=true).
pub fn steps_for_gitignore(context: &WizardContext, _project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();

    if !context.git_init {
        return steps;
    }

    let gitignore = content::gitignore_content(&context.languages);
    steps.push(Step::WriteFile {
        id: "gitignore".into(),
        label: "Create .gitignore".into(),
        description: "Generate .gitignore for project languages".into(),
        path: ".gitignore".into(),
        content: gitignore,
        overwrite: true,
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    steps
}

/// Фаза 6: финализация — самый конец пайплайна.
///   1. npm install: РОВНО один раз на каждый JS-каталог проекта (корень,
///      сегменты, подпапки фронтенд-каркасов). Скаффолдеры запускались с
///      --skip-install/--no-install, поэтому node_modules не плодятся на
///      каждом шаге генерации.
///   2. git add + git commit: README/конфиги уже записаны (фаза 5) и
///      попадают в стартовый коммит.
pub fn steps_for_finalize(
    context: &WizardContext,
    js_dirs: &[String],
    project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (i, dir) in js_dirs.iter().enumerate() {
        // "." = корень проекта; остальные каталоги — относительные пути,
        // которые разрешаются от project_path (как vite_install раньше)
        let wd = if dir == "." {
            project_path.to_string()
        } else {
            format!("{}/{}", project_path.trim_end_matches(['/', '\\']), dir)
        };
        if seen.contains(&wd) {
            continue;
        }
        seen.push(wd.clone());
        steps.push(Step::Command {
            id: format!("npm_install_{}", i),
            label: format!("Install npm dependencies ({})", wd),
            description: "Run npm install once, after all scaffolding".into(),
            command: "npm".into(),
            args: vec!["install".into()],
            working_dir: Some(wd),
            env: None,
            timeout_secs: Some(600),
            // Повторный запуск рецепта: node_modules уже на месте —
            // npm install пропускается (FileNotExists, путь от корня).
            condition: Some(StepCondition::FileNotExists {
                path: format!("{}/node_modules", if dir == "." { "." } else { dir }),
            }),
            // Abort: установка выбранных зависимостей (express, fastify,
            // telegraf, nest...) обязательна — провал не маскируется скипом.
            on_error: ErrorMode::Abort,
            interactive: vec![],
        });
    }

    if !context.git_init {
        return steps;
    }

    // git add + commit (опционально)
    steps.push(Step::Command {
        id: "git_add".into(),
        label: "Stage all files".into(),
        description: "Run git add .".into(),
        command: "git".into(),
        args: vec!["add".into(), ".".into()],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });

    steps.push(Step::Command {
        id: "git_commit".into(),
        label: "Create initial commit".into(),
        description: "Commit staged files (no-op when nothing is staged)".into(),
        command: git_commit_command(project_name),
        args: vec![],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });

    steps
}

/// Фаза 5 (часть): CI-workflow. Рабочий каталог job'а — там, где реально
/// лежит код главного языка:
///   - tauri: rust в `src-tauri/`, веб-часть — в `frontend/` (оболочка
///     владеет корнем, cargo/npm из корня не находят манифесты);
///   - иначе — каталог главного фреймворка (nest → backend/, react →
///     frontend/), затем каталог главного языка (python → backend/ в split);
///   - в одно-сторонних раскладках — корень проекта (без defaults-блока).
pub fn steps_for_ci(
    layout: &ProjectLayout,
    context: &WizardContext,
    _project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    let mut steps = Vec::new();

    if !context.ci {
        return steps;
    }

    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());

    let tauri_shell = context.frameworks.iter().any(|f| f == "tauri");
    let work_dir: Option<String> = if tauri_shell {
        match primary_lang {
            "rust" => Some("src-tauri".to_string()),
            "typescript" | "javascript" => Some("frontend".to_string()),
            _ => None,
        }
    } else {
        // Фреймворк главного языка (nest для typescript, django для python):
        // первый фреймворк, который реально использует этот язык — иначе
        // порядок карточек в мастере (react первым) увёл бы CI python-стека
        // в frontend/. Если такого фреймворка нет — каталог самого языка.
        primary_fw
            .filter(|fw| {
                framework_def(fw)
                    .is_some_and(|def| def.languages.iter().any(|l| l == primary_lang))
            })
            .and_then(|fw| layout.framework_dir(fw))
            .or_else(|| layout.language_dir(primary_lang))
    };

    // Создаём директорию .github/workflows
    steps.push(Step::CreateDirectory {
        id: "github_dir".into(),
        label: "Create .github directory".into(),
        description: "Create .github/workflows directory for CI".into(),
        path: ".github/workflows".into(),
        condition: None,
        on_error: ErrorMode::Skip,
    });

    let ci_content = content::generate_ci_content(
        primary_lang,
        primary_fw,
        project_name,
        work_dir.as_deref(),
        &context.tools,
    );

    steps.push(Step::WriteFile {
        id: "ci_workflow".into(),
        label: "Create CI workflow".into(),
        description: format!("Generate GitHub Actions workflow for {}", primary_lang),
        path: ".github/workflows/ci.yaml".into(),
        content: ci_content,
        overwrite: true,
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    steps
}

pub fn steps_for_readme(
    layout: &ProjectLayout,
    context: &WizardContext,
    _project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    // README генерируется подсистемой readme.rs: текст собирается из i18n-
    // словарей (locales/readme/*.json) на языке из context.readme_locale.
    let readme = readme::generate_readme(layout, context, project_name);

    vec![Step::WriteFile {
        id: "readme".into(),
        label: "Create README.md".into(),
        description: "Generate README.md with project info".into(),
        path: "README.md".into(),
        content: readme,
        // Шаг идёт в финальной фазе шаблонизации ПОСЛЕ всех CLI-фреймворков
        // и обязан перезаписать README, созданный самим CLI (create-next-app,
        // nest new...) — иначе наш шаблон молча теряется.
        overwrite: true,
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    }]
}

pub fn steps_for_vscode(layout: &ProjectLayout, context: &WizardContext) -> Vec<Step> {
    if !context.vscode_config {
        return Vec::new();
    }

    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");

    // Каталоги для слияния: корень всегда + сегменты канонической раскладки
    // + каталоги scaffold-генераторов (frontend/ в монолите). В не-корневых
    // каталогах генератор "vscode-merge" примешивает конфиг только если
    // .vscode/settings.json уже создал сам CLI (create-next-app и т.п.) —
    // лишние папки не дублируются.
    let mut dirs: Vec<String> = vec![".".to_string()];
    for dir in layout.eager_dirs() {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    for dir in [
        layout.backend_dir.as_deref(),
        layout.frontend_dir.as_deref(),
    ] {
        if let Some(dir) = dir {
            if !dirs.contains(&dir.to_string()) {
                dirs.push(dir.to_string());
            }
        }
    }
    for fw in &context.frameworks {
        if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw.as_str()) {
            let dir = scaffold_target_dir(fw, None);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }

    // Интерпретатор Python для .vscode/settings.json: канонический venv
    // проекта (venv/ или backend/venv), а не исторический `.venv` — VS Code
    // обязан видеть то же окружение, что создаёт пайплайн.
    let python_interpreter: Option<String> = if context.languages.iter().any(|l| l == "python") {
        let seg = python_segment_dir(context);
        let venv_rel = if seg == "." {
            "venv".to_string()
        } else {
            format!("{}/venv", seg)
        };
        if cfg!(target_os = "windows") {
            Some(format!(
                "${{workspaceFolder}}\\{}\\Scripts\\python.exe",
                venv_rel.replace('/', "\\")
            ))
        } else {
            Some(format!("${{workspaceFolder}}/{}/bin/python", venv_rel))
        }
    } else {
        None
    };

    vec![Step::Generate {
        id: "vscode_merge".into(),
        label: "Merge VS Code settings".into(),
        description: "Merge StackPilot VS Code settings with the ones created by CLIs".into(),
        generator_id: "vscode-merge".into(),
        generator_config: serde_json::json!({
            "lang": primary_lang,
            "dirs": dirs,
            "python_interpreter": python_interpreter,
        }),
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    }]
}

// Helper methods on Step (нужны, так как enum не может иметь методов напрямую)
// ============================================================================

impl Step {
    pub fn id(&self) -> String {
        match self {
            Step::Command { id, .. } => id.clone(),
            Step::WriteFile { id, .. } => id.clone(),
            // Step::RenderTemplate { id, .. } => id.clone(),
            Step::CreateDirectory { id, .. } => id.clone(),
            Step::Generate { id, .. } => id.clone(),
            Step::Parallel { id, .. } => id.clone(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Step::Command { label, .. } => label.clone(),
            Step::WriteFile { label, .. } => label.clone(),
            // Step::RenderTemplate { label, .. } => label.clone(),
            Step::CreateDirectory { label, .. } => label.clone(),
            Step::Generate { label, .. } => label.clone(),
            Step::Parallel { label, .. } => label.clone(),
        }
    }

    pub fn description(&self) -> String {
        match self {
            Step::Command { description, .. } => description.clone(),
            Step::WriteFile { description, .. } => description.clone(),
            // Step::RenderTemplate { description, .. } => description.clone(),
            Step::CreateDirectory { description, .. } => description.clone(),
            Step::Generate { description, .. } => description.clone(),
            Step::Parallel { description, .. } => description.clone(),
        }
    }

    pub fn condition(&self) -> Option<&StepCondition> {
        match self {
            Step::Command { condition, .. } => condition.as_ref(),
            Step::WriteFile { condition, .. } => condition.as_ref(),
            // Step::RenderTemplate { condition, .. } => condition.as_ref(),
            Step::CreateDirectory { condition, .. } => condition.as_ref(),
            Step::Generate { condition, .. } => condition.as_ref(),
            Step::Parallel { condition: _, .. } => None,
        }
    }
}

