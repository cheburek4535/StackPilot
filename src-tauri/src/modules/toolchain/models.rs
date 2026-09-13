// ============================================================
// Toolchain Manager — модели данных
// ============================================================
// Модуль отвечает за управление локальным окружением разработчика:
// обнаружение ПО, валидация версий, установка, обновление, PATH, health.
//
// Этот файл содержит ТОЛЬКО данные и их формы (struct/enum).
// Никакой логики — вся логика живёт в сервисах (core/, platforms/).
// Благодаря serde эти типы сериализуются в JSON и уезжают на фронтенд
// (тот же подход, что в project_creator::models).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ------------------------------------------------------------
// Статичные определения инструментов (tools.json)
// ------------------------------------------------------------

/// Полное определение инструмента. Загружается из tools.json при старте приложения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Уникальный id. Там, где возможно, совпадает с id из wizard_tree.json
    /// (postgresql, docker, npm, ...), чтобы маппинг был тривиальным.
    pub id: String,
    /// Категория: language / package_manager / compiler / database /
    /// container / vcs / editor / utility
    pub category: String,
    /// Человекочитаемое имя (Node.js, Docker Desktop, ...)
    pub display: String,
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Правила обнаружения на машине пользователя
    pub detection: DetectionRules,
    /// Минимальная и рекомендуемая версии (advisory, не блокирующие)
    #[serde(default)]
    pub versions: VersionRules,
    /// Источники установки по ОС. Порядок внутри списка = приоритет:
    /// сначала менеджер пакетов, потом официальный установщик.
    #[serde(default)]
    pub sources: InstallSources,
    /// Примерный размер загрузки установщика, МБ (для предупреждений)
    #[serde(default)]
    pub size_mb: u32,
    /// Требуются ли права администратора
    #[serde(default)]
    pub needs_admin: bool,
    /// Каталоги, которые нужно добавить в PATH после установки
    #[serde(default)]
    pub path_entries: Vec<String>,
    /// id инструмента, с которым этот поставляется «в комплекте»
    /// (npm→node, cargo→rust, pip→python). Такие тулы не устанавливаются отдельно.
    #[serde(default)]
    pub bundled_with: Option<String>,
    /// Дополнительные health-проверки (помимо версии)
    #[serde(default)]
    pub health_checks: Vec<HealthCheck>,
    /// Заметки пользователю (WSL2 для Docker, лицензии и т.п.)
    #[serde(default)]
    pub notes: Option<String>,
    /// Если задано — инструмент НЕ устанавливается автоматически
    /// (движки, SDK и т.п.), а в отчёте показывается честное предупреждение
    /// о ручной установке. Значение — текст предупреждения пользователю.
    #[serde(default)]
    pub manual_install: Option<String>,
    /// Расширенные метаданные каталога (aliases, зависимости, конфликты,
    /// ссылки, docker-альтернатива, объявленные возможности). Сериализуются
    /// «вплоскую» (без вложенного объекта в JSON); старые записи tools.json
    /// без этих полей читаются с честными значениями по умолчанию —
    /// отсутствие метаданных трактуется как «не заявлено», никогда как
    /// «угадано».
    #[serde(default, flatten)]
    pub extended: ToolExtendedMetadata,
}

/// Расширенные метаданные каталога (см. ToolDefinition.extended).
/// Все поля опциональны/пусты по умолчанию: старый каталог остаётся
/// валидным, а незаявленная возможность не выдаётся за заявленную.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolExtendedMetadata {
    /// Альтернативные имена/идентификаторы (для поиска и маппинга).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// id инструментов, необходимых этому для работы (замыкание зависимостей).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    /// id инструментов, одновременная установка которых нежелательна.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// Документация инструмента.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Исходники/домашняя страница (для аудита источников установки).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Явно заявленная доступность по ОС ("windows"/"linux"/"macos").
    /// Пусто = доступность выводится из наличия источников (и это
    /// вывод, а не заявление, — потребители обязаны это учитывать).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platform_availability: Vec<String>,
    /// Явно заявленные возможности обслуживания. None = не заявлено.
    #[serde(default)]
    pub declared_capabilities: DeclaredCapabilities,
    /// Docker-альтернатива хост-установке (если применимо).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerCapability>,
}

/// Заявленные (а не выведенные) возможности обслуживания инструмента.
/// Option<bool>: отсутствующее поле — «каталог ничего не говорит»,
/// что честно отличается от явного «нет».
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredCapabilities {
    /// Поддерживается ли удаление (uninstall) отдельным заданием.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removable: Option<bool>,
    /// Поддерживается ли восстановление (repair) отдельным заданием.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repairable: Option<bool>,
}

impl DeclaredCapabilities {
    /// Удаление возможно ТОЛЬКО если явно заявлено true.
    pub fn removable(&self) -> bool {
        self.removable == Some(true)
    }

    /// Восстановление возможно ТОЛЬКО если явно заявлено true.
    pub fn repairable(&self) -> bool {
        self.repairable == Some(true)
    }
}

/// Docker-альтернатива: инструмент может работать в контейнере вместо
/// хост-установки (двойные инструменты мастера: postgresql, redis, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerCapability {
    /// Имя образа по умолчанию (например postgres:17), если уместно.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Примечание для пользователя (WSL2, лицензии, порты и т.п.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl ToolDefinition {
    /// Есть ли хоть один источник установки.
    /// false = информационный тул (curl, tar, sqlite) — он не устанавливается,
    /// только проверяется, и никогда не блокирует создание проекта.
    pub fn installable(&self) -> bool {
        !self.sources.windows.is_empty()
            || !self.sources.linux.is_empty()
            || !self.sources.macos.is_empty()
    }

    /// Возвращает эффективные правила детекции для указанной ОС.
    /// Если platform_overrides содержит переопределение для os_name,
    /// его поля заменяют соответствующие базовые. Остальные поля
    /// наследуются из базовых detection rules.
    pub fn effective_detection(&self, os_name: &str) -> DetectionRules {
        let base = self.detection.clone();
        let override_opt = match os_name {
            "windows" => self.detection.platform_overrides.windows.as_ref(),
            "linux" => self.detection.platform_overrides.linux.as_ref(),
            "macos" => self.detection.platform_overrides.macos.as_ref(),
            _ => None,
        };
        let Some(ovr) = override_opt else {
            return base;
        };
        DetectionRules {
            version_probes: ovr.version_probes.clone().unwrap_or(base.version_probes),
            known_paths: ovr.known_paths.clone().unwrap_or(base.known_paths),
            registry_keys: ovr.registry_keys.clone().unwrap_or(base.registry_keys),
            ..Default::default()
        }
    }
}

/// Правила обнаружения инструмента на машине.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectionRules {
    /// Пробы версии: список команд, каждая — [бинарник, аргументы...].
    /// Пробуются по очереди, пока одна не вернёт ответ
    /// (например python на Linux может быть только python3).
    #[serde(default)]
    pub version_probes: Vec<Vec<String>>,
    /// Типовые пути установки (проверяются на существование).
    #[serde(default)]
    pub known_paths: Vec<String>,
    /// Ключи реестра Windows (reg query), проверяются на существование.
    /// Используются ТОЛЬКО на Windows; на Unix игнорируются.
    #[serde(default)]
    pub registry_keys: Vec<String>,
    /// Платформенные переопределения детекции: если заданы,
    /// используются вместо базовых правил для указанной ОС.
    /// Позволяет не дублировать registry_keys в tools.json
    /// для инструментов, которые работают и на Unix без реестра.
    #[serde(default, skip_serializing_if = "PlatformDetectionOverrides::is_empty")]
    pub platform_overrides: PlatformDetectionOverrides,
}

/// Платформенные переопределения: каждое поле — optional override
/// поверх базовых DetectionRules. Пустая структура = ничего не
/// переопределяется (legacy catalog остаётся валидным).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformDetectionOverrides {
    /// Переопределения для Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<PlatformDetection>,
    /// Переопределения для Linux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux: Option<PlatformDetection>,
    /// Переопределения для macOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<PlatformDetection>,
}

impl PlatformDetectionOverrides {
    pub fn is_empty(&self) -> bool {
        self.windows.is_none() && self.linux.is_none() && self.macos.is_none()
    }
}

/// Platform-specific detection override: any field set replaces
/// the corresponding field from the base DetectionRules.
/// Only `Some(...)` fields override; `None` fields inherit from base.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformDetection {
    /// Override version_probes (Some replaces base entirely).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_probes: Option<Vec<Vec<String>>>,
    /// Override known_paths (Some replaces base entirely).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_paths: Option<Vec<String>>,
    /// Override registry_keys (Some replaces base entirely; use
    /// Some(vec![]) to explicitly clear Windows-only keys on Unix).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_keys: Option<Vec<String>>,
}

/// Advisory-версии. min — «ниже этого работа проекта не гарантирована»,
/// recommended — «рекомендуем обновиться».
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionRules {
    #[serde(default)]
    pub min: Option<String>,
    #[serde(default)]
    pub recommended: Option<String>,
    /// Динамическое разрешение рекомендуемой версии (против устаревания
    /// каталога): при сканировании recommended берётся у резолвера
    /// (кэш с TTL), статичное значение — страховка, если резолвер
    /// недоступен. None = чисто статичная политика (как раньше).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver: Option<VersionResolver>,
}

/// Наборы источников установки по ОС.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallSources {
    #[serde(default)]
    pub windows: Vec<InstallSource>,
    #[serde(default)]
    pub linux: Vec<InstallSource>,
    #[serde(default)]
    pub macos: Vec<InstallSource>,
}

/// Один источник установки инструмента.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSource {
    pub kind: InstallSourceKind,
    /// id пакета в менеджере пакетов (OpenJS.NodeJS.LTS) или имя источника
    pub id: String,
    /// Прямая ссылка для Official/Script источников
    #[serde(default)]
    pub url: Option<String>,
    /// Имя файла, под которым сохраняется скачанный установщик.
    /// Нужно для программ, которые определяют своё поведение по имени
    /// файла: rustup-init.exe должен остаться rustup-init.exe, иначе он
    /// решит, что его вызвали как «прокси» (unknown proxy name).
    #[serde(default)]
    pub file_name: Option<String>,
    /// Статичные аргументы тихой установки (winget: --silent; MSI: /quiet)
    #[serde(default)]
    pub args: Vec<String>,
    /// Дополнительные аргументы менеджера пакетов (winget --override ...)
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// true — аргументы дополняются кодом на этапе установки
    /// (пароль PostgreSQL генерируется в рантайме, его нельзя хранить в JSON)
    #[serde(default)]
    pub dynamic_args: bool,
    /// Куда распаковывать zip-архив (gradle, maven) или клонировать
    /// git-репозиторий. Без этого поля zip-источник считается
    /// некорректным и не запускается.
    #[serde(default)]
    pub install_dir: Option<String>,
    /// Переопределение needs_admin для конкретного источника (zip-распаковка
    /// не требует UAC, даже если у инструмента в целом needs_admin=true).
    #[serde(default)]
    pub needs_admin: Option<bool>,
    /// Способ исполнения источника (см. ExecutionKind). По умолчанию
    /// (None) движок определяет его автоматически: по URL (.git →
    /// git clone) и расширению скачанного файла (.phar → php,
    /// .ps1/.sh → интерпретатор, .zip/.tgz → распаковка, иначе exe).
    /// Явное значение нужно для файлов без расширения (composer-installer)
    /// и для жёсткой гарантии поведения.
    #[serde(default)]
    pub execution: Option<ExecutionKind>,
    /// Команда финализации установки: [программа, аргументы...].
    /// Выполняется ПОСЛЕ обновления PATH и ДО verify. Нужна SDK,
    /// чей первый запуск долгий: flutter после git clone качает
    /// Dart SDK минутами, и без этого шага verify (и последующие
    /// сканы) упираются в таймаут пробы 10с — «установка не
    /// подтвердилась» при реально установленном SDK.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<Vec<String>>,
    /// SHA-256 скачанного файла (hex, нижний регистр). Задаётся в
    /// tools.json для Official/Script источников. None = источник
    /// БЕЗ контроля целостности: скачивание помечается как
    /// непроверенное (unverified) и честно рапортует об этом в UI,
    /// но не притворяется проверенным.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Шаблон URL с плейсхолдером `{version}`: источник не привязан
    /// к конкретной версии — самая свежая резолвится на лету
    /// (version_resolver), версия подставляется в шаблон при
    /// установке. Статичное поле url остаётся страховкой на случай
    /// недоступности резолвера (устаревшее лучше сломанного).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
    /// Чем резолвится актуальная версия источника (см. VersionResolver).
    /// Обязателен при url_template; без шаблона используется, если
    /// резолвер сам возвращает готовый URL (hashicorp, apache и т.п.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_resolver: Option<VersionResolver>,
}

/// Способ исполнения скачанного источника: чем движок «запускает» файл.
/// Поле InstallSource.execution; Auto — автоматическое определение
/// (правила в installer.rs::resolve_execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    /// Git-репозиторий: `git clone <url> <каталог>`. Файл НЕ скачивается
    /// и НЕ запускается — иначе «%1 не является приложением Win32»
    /// (os error 193), как было с flutter.git.
    GitClone,
    /// Бинарь/инсталлятор (.exe, .msi, .msix): запускается напрямую
    /// (msi/msix — через свои механизмы).
    Exe,
    /// Скрипт интерпретатора: .ps1 → `powershell -File`, .sh → `bash`.
    Script,
    /// PHP-скрипт (.phar — composer и т.п.): `php <файл>` — CreateProcess
    /// phar не понимает (os error 193).
    Phar,
    /// Архив (.zip/.tgz): распаковывается в install_dir, а не запускается.
    Archive,
    /// Автоопределение по URL/расширению (значение по умолчанию).
    Auto,
}

/// Чем ставим: менеджером пакетов ОС, официальным установщиком или скриптом.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallSourceKind {
    PkgManager,
    Official,
    Script,
    /// Прямая установка из официального online-репозитория (Qt).
    /// url — базовый каталог репозитория (например
    /// https://download.qt.io/online/qtsdkrepository/windows_x86);
    /// install_dir — корень, куда складывается Qt (обычно
    /// %LOCALAPPDATA%/Programs/Qt). Пакеты, версии и каталоги
    /// извлечения разбираются из Updates.xml репозитория.
    QtOnline,
}

/// Динамический резолвер актуальной версии инструмента (против
/// устаревания каталога: версии в tools.json больше не зашиваются
/// намертво, а запрашиваются у официальных источников на лету).
///
/// Используется в двух местах:
///   - `InstallSource.version_resolver` (+ `url_template`) — какая
///     версия скачивается при установке;
///   - `VersionRules.resolver` — какая версия считается рекомендуемой
///     в сканах/UI.
///
/// Все резолверы работают по HTTPS-запросам (curl) и поддерживают
/// кэш с TTL; при недоступности источника используется статичное
/// значение из tools.json (устаревшее лучше сломанного).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionResolver {
    /// ziglang.org/download/index.json — стабильный релиз с билдом
    /// для нужной платформы; отдаёт версию и SHA-256.
    Ziglang,
    /// releases.hashicorp.com/<product>/index.json (terraform и т.п.):
    /// последний стабильный релиз с билдом windows/amd64.
    Hashicorp {
        /// Имя продукта в каталоге релизов (например "terraform").
        product: String,
    },
    /// api.github.com/repos/<repo>/releases/latest: последний релиз
    /// (тег без ведущей "v") и URL ассета по шаблону.
    Github {
        /// "JetBrains/kotlin"
        repo: String,
        /// Шаблон имени ассета с плейсхолдером {version}
        /// (например "kotlin-compiler-{version}.zip").
        asset: String,
    },
    /// windows.php.net/downloads/releases/ — свежий билд ветки
    /// (например "8.4") nts vs17 x64, имя из каталога релизов
    /// (windows.php.net хранит только актуальный билд ветки).
    PhpWindows {
        /// Ветка PHP, например "8.4".
        branch: String,
    },
    /// services.gradle.org/versions/current: актуальный стабильный
    /// Gradle; URL и SHA-256 берутся из API.
    Gradle,
    /// repo.maven.apache.org maven-metadata.xml: актуальный Maven.
    MavenApache,
    /// downloads.apache.org/kafka/: последняя стабильная Kafka
    /// (имя архива разбирается из каталога версии — scala-суффикс
    /// не зашит).
    ApacheKafka,
    /// dl.grafana.com/oss/release/ не отдаёт листинг (404) — Grafana
    /// резолвится через GitHub releases (grafana/grafana).
    Grafana,
    /// www.swift.org/api/v1/install/releases.json: свежий стабильный
    /// релиз Swift (Windows-сборка собирается шаблоном каталога).
    Swift,
    /// downloads.mongodb.org/current.json: свежий production-релиз
    /// MongoDB Community с windows/x86_64 base-сборкой (URL и SHA-256
    /// берутся из ответа).
    MongoDb,
    /// winget show --id <id>: актуальная версия пакета менеджера
    /// (используется для recommended-политики; установка winget
    /// и так всегда берёт свежайшую версию по умолчанию).
    Winget {
        /// id пакета (например "PostgreSQL.PostgreSQL.17").
        id: String,
    },
    /// dotnetcli.blob.core.windows.net releases-index.json: свежайший
    /// SDK канала (например "10.0") — для recommended .NET.
    DotnetChannel {
        /// Канал: "8.0", "10.0", ...
        channel: String,
    },
}

impl VersionResolver {
    /// Стабильный идентификатор для кэша (сериализованная форма).
    pub fn cache_key(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "resolver".to_string())
    }
}

/// Дополнительная health-проверка: команда должна выполниться успешно.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub label: String,
    pub command: Vec<String>,
}

// ------------------------------------------------------------
// Рантайм-модели: что происходит на машине пользователя
// ------------------------------------------------------------

/// Статус инструмента по результатам обнаружения.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolStatus {
    /// Инструмент не найден
    Missing,
    /// Установлен, версия устраивает
    Installed { version: String },
    /// Установлен, но вышла рекомендуемая версия
    UpdateAvailable {
        installed: String,
        recommended: String,
    },
    /// Найден, но не работает (бинарь не в PATH, сломанная установка)
    PathBroken { reason: String },
    /// Инструмент не установлен и ставится ТОЛЬКО вручную (движки, SDK).
    /// Честное предупреждение в отчёте: не блокирует создание проекта,
    /// не участвует в установке. Причина — текст для пользователя.
    ManualInstall { reason: String },
    /// «Двойной» инструмент мастера (postgresql, mongodb, kafka, ...):
    /// по умолчанию разворачивается docker-compose.yaml проекта, локально
    /// не устанавливается. Показывается в ОПЦИОНАЛЬНОЙ секции отчёта с
    /// предложением поставить локально; в план установки не попадает.
    RunInDocker,
}

impl ToolStatus {
    /// «Всё в порядке, можно работать»?
    pub fn is_ok(&self) -> bool {
        matches!(self, ToolStatus::Installed { .. })
    }
}

/// Информация об ОС и менеджерах пакетов (для фронтенда и логов).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub os: String,
    /// Версия ОС — наполняется на этапе 5 (команда ОС)
    pub os_version: String,
    pub package_managers: Vec<String>,
    /// Сколько инструментов знает Toolchain Manager
    pub tool_count: usize,
    /// Возможности платформы: честный ответ «умеет ли бэкенд этой ОС
    /// исполнять установки». false = кнопки установки показывать нельзя
    /// (задачи вернут Skipped/unsupported).
    #[serde(default)]
    pub capabilities: PlatformCapabilities,
}

/// Что бэкенд реально умеет на текущей ОС (платформенная правда).
/// Default — «ничего не умеем» (консервативно): UI без кнопок установки
/// безопаснее, чем кнопки, которые не сработают.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    /// Автоматическая установка реализована для этой ОС.
    /// Windows: да. Linux/macOS: пока нет (источники есть в каталоге,
    /// исполнитель — нет) — UI не должен предлагать кнопку установки.
    pub install_execution_supported: bool,
    /// Запуск с повышением прав (UAC) поддерживается.
    pub elevation_supported: bool,
}

/// Требования проекта к окружению — входной контракт tc_check_environment.
/// Фронтенд собирает его из WizardContext проекта (языки, фреймворки, тулы,
/// флаги git/vscode/docker) и присылает в toolchain.
///
/// Намеренно НЕ зависит от project_creator::models::WizardContext:
/// модули остаются независимыми, интеграция идёт только через фронтенд.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectRequirements {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub frameworks: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// Docker-инструменты мастера (postgresql, redis, mongodb, kafka,
    /// grafana, mysql), которые пользователь решил ставить ЛОКАЛЬНО вместо
    /// docker-compose. Попадают в обычные requirements (как Missing и т.п.),
    /// а из docker-compose.yaml проекта исключаются (см. WizardContext.local_infra_tools).
    #[serde(default)]
    pub local_infra_tools: Vec<String>,
    #[serde(default)]
    pub git_init: bool,
    #[serde(default)]
    pub vscode_config: bool,
    #[serde(default)]
    pub docker: bool,
}

/// Требование проекта к окружению — по одному экземпляру на инструмент.
/// Из этого собирается экран проверки зависимостей.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequirement {
    pub tool_id: String,
    pub display: String,
    pub category: String,
    /// Имя файла иконки (static/images/), из tools.json. Фронтенд
    /// рендерит /images/<icon>, отсутствующий файл → дефолтная SVG.
    #[serde(default)]
    pub icon: Option<String>,
    pub status: ToolStatus,
    /// Сколько МБ скачаем, если нужна установка
    pub size_mb: u32,
    pub needs_admin: bool,
    /// Человекочитаемое описание источника (например «winget: Git.Git»)
    pub source_description: String,
    /// Модули/компоненты установки (для Qt: qt-qml, qt-webengine, ...).
    /// Заполняется из выбора пользователя в мастере, переносится
    /// в InstallTask и передаётся установщику.
    #[serde(default)]
    pub install_options: Vec<String>,
}

/// Полный отчёт проверки окружения под конкретный проект.
/// Это ответ команды tc_check_environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentCheck {
    pub os: String,
    pub requirements: Vec<ToolRequirement>,
    /// «Опциональные» требования: docker-инструменты мастера (см.
    /// requirements::docker_optional_requirements), которые по умолчанию
    /// разворачиваются контейнерами проекта. Показываются отдельной
    /// секцией экрана окружения с возможностью переключиться на локальную
    /// установку (тогда они уезжают в requirements).
    #[serde(default)]
    pub optional_requirements: Vec<ToolRequirement>,
    /// Суммарный размер загрузки, МБ
    pub total_size_mb: u64,
    /// Свободное место на целевом диске, МБ (0 = ещё не проверялось)
    pub free_space_mb: u64,
    pub enough_space: bool,
    pub needs_admin_any: bool,
    /// true = всё установлено и можно создавать проект
    pub all_ready: bool,
    /// true = проверка успела ДОПРОСИТЬ все запрошенные инструменты до
    /// дедлайна. false = отчёт ЧАСТИЧНЫЙ: отсутствующие в requirements
    /// инструменты не «готовы», а «не проверены» — их нельзя считать
    /// установленными или готовыми (см. scan_timed_out).
    #[serde(default = "default_true")]
    pub complete: bool,
    /// Инструменты, не успевшие провериться до дедлайна (частичный отчёт).
    #[serde(default)]
    pub scan_timed_out: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Задача установки одного инструмента. Задачи собираются в InstallPlan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallTask {
    pub task_id: String,
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/). Копируется из ToolRequirement.
    #[serde(default)]
    pub icon: Option<String>,
    pub size_mb: u32,
    pub needs_admin: bool,
    pub source_description: String,
    /// Модули/компоненты установки (Qt: qt-qml/qt-widgets/qt-webengine/...),
    /// переносятся из ToolRequirement без изменений.
    #[serde(default)]
    pub install_options: Vec<String>,
    pub state: TaskState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskState {
    Pending,
    Running { phase: TaskPhase },
    Success { version: String },
    Failed { error: String },
    Skipped { reason: String },
}

/// Фаза установки — на фронтенде показывается как подпрогресс задачи.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskPhase {
    Downloading,
    Installing,
    Verifying,
    UpdatingPath,
}

/// План установки: упорядоченный список задач.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub tasks: Vec<InstallTask>,
    pub total_size_mb: u64,
    pub os: String,
    /// Идентификатор установки (генерирует бэкенд при запуске).
    /// Присутствует в событиях и финальном снапшоте, чтобы слушатель
    /// отличал события ТЕКУЩЕЙ установки от «хвостов» прежней.
    #[serde(default)]
    pub session_id: String,
}

/// Терминальные/живые состояния сессии установки. `running` (bool)
/// остаётся для совместимости с фронтендом, но авторитетным считается
/// status: он же пишется в журнал заданий и переживает перезапуск.
///
/// Default = Running: старые журналы/файлы без поля status читаются
/// как «задание шло», после чего recover_on_startup честно переводит
/// его в Interrupted (никогда не притворяется успехом).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InstallSessionStatus {
    /// Установка идёт
    #[default]
    Running,
    /// Все задачи завершились (успех/пропуск), ошибок нет
    Completed,
    /// Хотя бы одна задача упала
    Failed,
    /// Отменено пользователем
    Cancelled,
    /// Приложение перезапустилось во время установки — задача
    /// НЕ продолжается, состояние восстановлено из журнала как
    /// «прервано» (не «running» и не «успех»)
    Interrupted,
}

/// Живая сессия установки — состояние для tc_get_install_status.
/// Хранится в ToolchainState, пока идёт/завершилась установка.
///
/// Секреты в сессию НЕ сериализуются (serde(skip)): они живут только
/// в изолированном хранилище (core/secrets.rs) и выдаются один раз
/// через tc_take_new_secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSession {
    pub started_at: String,
    /// false = установка завершена (или ещё не начиналась).
    /// Производное от status; оставлено для совместимости фронтенда.
    pub running: bool,
    pub plan: InstallPlan,
    /// Авторитетное состояние сессии (см. InstallSessionStatus).
    #[serde(default)]
    pub status: InstallSessionStatus,
    /// Секреты не покидают бэкенд через этот тип (skip-сериализация);
    /// поле оставлено внутренним для передачи в одноразовую витрину.
    #[serde(skip)]
    pub secrets: HashMap<String, String>,
}

impl InstallSession {
    /// Новая running-сессия без секретов.
    pub fn starting(started_at: String, plan: InstallPlan) -> Self {
        Self {
            started_at,
            running: true,
            plan,
            status: InstallSessionStatus::Running,
            secrets: HashMap::new(),
        }
    }
}

/// Событие установки — стримится на фронтенд через tauri events
/// (тот же механизм, что project_creator:step_event).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainEvent {
    pub event_type: ToolchainEventType,
    pub task_index: usize,
    pub total_tasks: usize,
    pub task_id: String,
    pub tool_id: String,
    pub timestamp: String,
    /// Идентификатор установки-владельца события. События прежней
    /// установки (прилетевшие с задержкой) фронтенд обязан отбрасывать,
    /// если session_id не совпадает с текущим.
    #[serde(default)]
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolchainEventType {
    TaskStarted,
    TaskPhaseChanged {
        phase: TaskPhase,
    },
    /// Строка вывода установщика
    TaskProgress {
        line: String,
    },
    TaskCompleted {
        state: TaskState,
    },
    AllCompleted {
        success_count: usize,
        failed: Vec<String>,
    },
    Error {
        message: String,
    },
}

/// Промежуточный прогресс проверки окружения. Стримится на фронтенд
/// событием `toolchain:check_progress` после каждого проверенного
/// инструмента — пользователь видит, что проверка идёт, а не висит.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckProgressEvent {
    /// Сколько инструментов уже проверено (включая этот)
    pub done: usize,
    /// Сколько всего проверяется
    pub total: usize,
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/), из tools.json.
    #[serde(default)]
    pub icon: Option<String>,
    pub status: ToolStatus,
    /// Идентификатор запуска проверки (генерирует бэкенд): события
    /// прежнего запуска не должны смешиваться с текущим.
    #[serde(default)]
    pub scan_id: String,
}

// ------------------------------------------------------------
// Health
// ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

/// Явное состояние здоровья инструмента. Пустой список проверок —
/// это «не проверяли» (NotChecked), а НЕ «нездоров»: отсутствие
/// данных не должно выглядеть как отрицательный вердикт.
///
/// Default = NotChecked: отсутствие данных о здоровье — «не знаем»,
/// а не какой-либо вердикт.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HealthState {
    /// Установлен, но health_checks в каталоге нет — не проверялся
    #[default]
    NotChecked,
    /// Установлен и все проверки прошли
    Healthy,
    /// Установлен, но хотя бы одна проверка упала
    Failed,
    /// Не установлен / сломан путь — проверки здоровья неприменимы
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHealth {
    pub tool_id: String,
    pub display: String,
    /// Имя файла иконки (static/images/), из tools.json.
    #[serde(default)]
    pub icon: Option<String>,
    pub checks: Vec<HealthCheckResult>,
    /// Явное состояние (авторитетное; см. HealthState).
    #[serde(default)]
    pub state: HealthState,
    /// Совместимость со старым фронтендом: true только при Healthy.
    /// Для NotChecked значение false, но состояние различает
    /// «не проверяли» и «проверяли — плохо».
    pub ok: bool,
}

/// Полный health-отчёт по окружению (для страницы окружения).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub tools: Vec<ToolHealth>,
    /// Процент здоровья 0..100
    pub score: u8,
    pub scanned_at: String,
}

// ------------------------------------------------------------
// Metadata — локальное хранилище состояния (state.json)
// ------------------------------------------------------------

/// Состояние Toolchain Manager, сохраняется в app_data_dir/toolchain/state.json.
///
/// СЕКРЕТЫ сюда больше не пишутся: они живут в изолированном
/// хранилище (core/secrets.rs, отдельный файл, шифрование DPAPI на
/// Windows). Поле secrets оставлено только для ОБРАТНОЙ совместимости
/// загрузки старых state.json — при загрузке оно вычищается и
/// мигрируется в изолированное хранилище (см. MetadataStore::load).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolchainMetadata {
    #[serde(default)]
    pub last_scan: Option<String>,
    #[serde(default)]
    pub tools: HashMap<String, InstalledToolInfo>,
    /// УСТАРЕЛО: не заполняется новыми записями; при загрузке старого
    /// state.json содержимое мигрирует в SecretStore и из файла уходит.
    /// Пустая секция в файл не пишется.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub secrets: HashMap<String, String>,
    #[serde(default)]
    pub prefs: HashMap<String, String>,
    /// ЯВНО усыновлённые инструменты (tcx_adopt_tool): найдены на машине
    /// как ручная установка и взяты под наблюдение по явному действию
    /// пользователя. НЕ означают «установлено StackPilot» — это только
    /// track-метка (значение — момент усыновления).
    #[serde(default)]
    pub adopted: HashMap<String, String>,
}

/// Санитизированная выдача tc_get_metadata: те же поля, что у
/// ToolchainMetadata, но БЕЗ секретов. Это единственная форма,
/// которую команда отдаёт наружу — секреты не покидают бэкенд
/// иначе как через одноразовую витрину tc_take_new_secrets.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolchainMetadataView {
    #[serde(default)]
    pub last_scan: Option<String>,
    #[serde(default)]
    pub tools: HashMap<String, InstalledToolInfo>,
    #[serde(default)]
    pub prefs: HashMap<String, String>,
    #[serde(default)]
    pub adopted: HashMap<String, String>,
}

impl From<&ToolchainMetadata> for ToolchainMetadataView {
    fn from(data: &ToolchainMetadata) -> Self {
        Self {
            last_scan: data.last_scan.clone(),
            tools: data.tools.clone(),
            prefs: data.prefs.clone(),
            adopted: data.adopted.clone(),
        }
    }
}

/// Информация об установленном инструменте, известная Toolchain Manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledToolInfo {
    pub path: String,
    pub version: String,
    pub installed_at: String,
    /// Каталоги, добавленные в PATH этим инструментом
    #[serde(default)]
    pub path_entries: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_def() -> ToolDefinition {
        ToolDefinition {
            id: "test-tool".into(),
            category: "utility".into(),
            display: "Test".into(),
            description: String::new(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![vec!["tool".into(), "--version".into()]],
                known_paths: vec!["/usr/local/bin/tool".into()],
                registry_keys: vec!["HKLM\\Software\\Tool".into()],
                platform_overrides: PlatformDetectionOverrides::default(),
            },
            versions: Default::default(),
            sources: InstallSources::default(),
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    #[test]
    fn effective_detection_without_override_returns_base() {
        let def = base_def();
        let det = def.effective_detection("linux");
        assert_eq!(det.version_probes, def.detection.version_probes);
        assert_eq!(det.known_paths, def.detection.known_paths);
        assert_eq!(det.registry_keys, def.detection.registry_keys);
    }

    #[test]
    fn effective_detection_with_linux_override_replaces_version_probes() {
        let mut def = base_def();
        def.detection.platform_overrides.linux = Some(PlatformDetection {
            version_probes: Some(vec![vec!["tool3".to_string(), "--version".to_string()]]),
            known_paths: None,
            registry_keys: None,
        });
        let det = def.effective_detection("linux");
        assert_eq!(
            det.version_probes,
            vec![vec!["tool3".to_string(), "--version".to_string()]]
        );
        assert_eq!(det.known_paths, def.detection.known_paths);
        assert_eq!(det.registry_keys, def.detection.registry_keys);
    }

    #[test]
    fn effective_detection_with_macos_override_clears_registry_keys() {
        let mut def = base_def();
        def.detection.platform_overrides.macos = Some(PlatformDetection {
            version_probes: None,
            known_paths: None,
            registry_keys: Some(vec![]),
        });
        let det = def.effective_detection("macos");
        assert!(det.registry_keys.is_empty());
        assert_eq!(det.version_probes, def.detection.version_probes);
    }

    #[test]
    fn effective_detection_windows_override_inherits_base() {
        let mut def = base_def();
        def.detection.platform_overrides.windows = Some(PlatformDetection {
            version_probes: None,
            known_paths: Some(vec!["C:\\Tool\\bin".into()]),
            registry_keys: None,
        });
        let det = def.effective_detection("windows");
        assert_eq!(det.known_paths, vec!["C:\\Tool\\bin".to_string()]);
        assert_eq!(det.version_probes, def.detection.version_probes);
        assert_eq!(det.registry_keys, def.detection.registry_keys);
    }

    #[test]
    fn effective_detection_unknown_os_returns_base() {
        let mut def = base_def();
        def.detection.platform_overrides.linux = Some(PlatformDetection {
            version_probes: Some(vec![vec!["other".into()]]),
            known_paths: None,
            registry_keys: None,
        });
        let det = def.effective_detection("freebsd");
        assert_eq!(det.version_probes, def.detection.version_probes);
    }

    #[test]
    fn platform_detection_overrides_is_empty_by_default() {
        let ovr = PlatformDetectionOverrides::default();
        assert!(ovr.is_empty());
    }

    #[test]
    fn platform_detection_overrides_not_empty_with_linux() {
        let mut ovr = PlatformDetectionOverrides::default();
        ovr.linux = Some(PlatformDetection::default());
        assert!(!ovr.is_empty());
    }

    #[test]
    fn effective_detection_linux_override_replaces_known_paths() {
        let mut def = base_def();
        def.detection.platform_overrides.linux = Some(PlatformDetection {
            version_probes: None,
            known_paths: Some(vec!["/opt/tool/bin".into()]),
            registry_keys: Some(vec![]),
        });
        let det = def.effective_detection("linux");
        assert_eq!(det.known_paths, vec!["/opt/tool/bin".to_string()]);
        assert!(det.registry_keys.is_empty());
    }
}
