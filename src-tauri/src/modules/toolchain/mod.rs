// ============================================================
// Toolchain Manager — точка входа модуля
// ============================================================
// Модуль управляет локальным окружением разработчика:
//  1. узнаёт, что установлено и какие версии (Discovery/Version);
//  2. собирает план установки недостающего (Planner/Installer);
//  3. следит за PATH, диском и здоровьем окружения (Path/Health);
//  4. хранит локальную мета-информацию (Metadata).
//
// Модуль самодостаточен: не зависит от project_creator, devlauncher
// и workspace. Интеграция с ними — только через фронтенд и события,
// бэкенд соседних модулей не трогается.

pub mod commands;
pub mod core;
pub mod defs;
pub mod domain;
pub mod engine;
pub mod models;
pub mod platforms;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use core::jobs::JobJournal;
use core::metadata::MetadataStore;
use core::secrets::SecretStore;
use domain::engine::ScanEngine;
use engine::JobEngine;
use models::*;

/// Глобальное состояние модуля Toolchain Manager.
/// Регистрируется в Tauri State (как ProjectCreatorState).
pub struct ToolchainState {
    /// Standalone-каталог (tools.json): единственный источник для
    /// tcx_*-команд, сканов, планировщика и UI standalone Toolchain.
    definitions: Vec<ToolDefinition>,
    /// Легаси-каталог совместимости Project Creator (unity/unreal/godot):
    /// НЕ виден standalone-поверхности; добавляется только в легаси
    /// tc_*-команды (мастер создания проектов).
    legacy_definitions: Vec<ToolDefinition>,
    /// Живая сессия установки — обновляется фоновой задачей
    /// (Arc+Mutex, т.к. команда-run и команда-status живут отдельно).
    install_session: Arc<Mutex<Option<InstallSession>>>,
    /// Флаг отмены установки: tc_abort_install ставит его, installer
    /// опрашивает между задачами и в процессе стриминга (kill).
    abort_install: Arc<AtomicBool>,
    /// Постоянное состояние (state.json): установленные инструменты,
    /// секреты, настройки. Меняется по завершении установки.
    metadata: Arc<Mutex<MetadataStore>>,
    /// Журнал заданий (jobs.json): терминальные состояния установок
    /// переживают перезапуск; Running при старте = «прервано».
    journal: Arc<JobJournal>,
    /// Изолированное хранилище секретов (secrets.bin, DPAPI на Windows).
    /// Секреты не живут ни в state.json, ни в ответах команд.
    secrets: Arc<Mutex<SecretStore>>,
    /// Секреты ПОСЛЕДНЕЙ установки (пароль PostgreSQL и т.п.) —
    /// «одноразовая витрина» для фронтенда: tc_take_new_secrets
    /// забирает и очищает, чтобы старые пароли не висели в UI.
    pending_secrets: Arc<Mutex<HashMap<String, String>>>,
    /// Движок read-only сканов (domain/): задания, события, кэш снапшотов.
    /// Сканы не мутируют машину — только пробы и health-объявления каталога.
    scan_engine: Arc<ScanEngine>,
    /// Канонический движок заданий (engine/): install/update/repair-path/
    /// health-check задания с персистентностью до исполнения,
    /// восстановлением после перезапуска и типизированными событиями.
    /// Фактическая реализация мутаций машины; легаси tc_run_install
    /// идёт через адаптер над ним.
    job_engine: Arc<JobEngine>,
}

impl ToolchainState {
    /// `dir` — каталог хранения состояния (app_data/toolchain).
    pub fn new(dir: PathBuf) -> Self {
        // Осиротевшие временные файлы заданий (крэш посреди установки)
        // убираются при старте — «вечного» мусора в temp не копится.
        core::console::sweep_stale_temp_files();

        let definitions = defs::load_definitions();
        let legacy_definitions = defs::load_legacy_definitions();

        // Дубликаты id ломают lookup по id — это ошибка разработчика,
        // падаем громко и сразу, а не тихо при первом обращении.
        let warnings = defs::validate(&definitions);
        let has_duplicates = warnings.iter().any(|w| w.starts_with("Дубликат"));
        assert!(
            !has_duplicates,
            "tools.json содержит дубликаты id: {:?}",
            warnings
        );

        let mut metadata_store = MetadataStore::load(&dir);

        // Секреты: изолированное хранилище + однократная миграция
        // plaintext-секретов из старых state.json (обратная совместимость).
        // take_legacy_secrets вырезает их из стора — state.json больше
        // никогда их не содержит.
        let mut secret_store = SecretStore::load(&dir);
        let legacy = { metadata_store.take_legacy_secrets() };
        if let Err(e) = secret_store.migrate_from_map(&legacy) {
            log::error!("[toolchain] миграция унаследованных секретов не удалась: {e}");
        }
        // Деградация защиты — не молчаливая: на Windows секреты обязаны
        // шифроваться DPAPI; если нет, пользователь узнаёт об этом из лога.
        if cfg!(target_os = "windows") && !secret_store.is_encrypted() && !legacy.is_empty() {
            log::warn!("[toolchain] ВНИМАНИЕ: DPAPI недоступен — секреты хранятся без шифрования");
        }

        // Восстановление журнала заданий: незавершённая Running-сессия
        // после перезапуска становится Interrupted («вечных» установок нет).
        let journal = JobJournal::load(&dir);
        if let Some(recovered) = journal.recover_on_startup() {
            if matches!(recovered.status, InstallSessionStatus::Interrupted) {
                log::info!(
                    "[toolchain] восстановлена прерванная перезапуском установка от {}",
                    recovered.started_at
                );
            }
        }

        // Движок сканов: восстановление прерванного скана из журнала
        // (Running → Interrupted) и загрузка кэша снапшотов.
        let scan_engine = Arc::new(ScanEngine::new(&dir));
        if let Some(recovered_scan) = scan_engine.recover_on_startup() {
            log::info!(
                "[toolchain] восстановлен прерванный скан {} ({})",
                recovered_scan.job_id, recovered_scan.scan_id
            );
        }

        // Канонический движок заданий: Running-записи в журнале
        // (jobs/*.json) становятся Interrupted — «вечных» заданий нет.
        let job_engine = Arc::new(JobEngine::load(&dir));
        for recovered_job in job_engine.recover_on_startup() {
            log::info!(
                "[toolchainx] восстановлено прерванное задание {} ({})",
                recovered_job.job_id,
                recovered_job.operation.as_str()
            );
        }

        Self {
            definitions,
            legacy_definitions,
            install_session: Arc::new(Mutex::new(None)),
            abort_install: Arc::new(AtomicBool::new(false)),
            metadata: Arc::new(Mutex::new(metadata_store)),
            journal: Arc::new(journal),
            secrets: Arc::new(Mutex::new(secret_store)),
            pending_secrets: Arc::new(Mutex::new(HashMap::new())),
            scan_engine,
            job_engine,
        }
    }

    /// Все определения инструментов (для фронтенда и сервисов).
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    /// Объединённый каталог для легаси-команд: standalone + легаси-совместимость.
    pub fn merged_definitions(&self) -> Vec<ToolDefinition> {
        let mut all = self.definitions.clone();
        for d in &self.legacy_definitions {
            if !all.iter().any(|x| x.id == d.id) {
                all.push(d.clone());
            }
        }
        all
    }

    /// Найти определение по id (только standalone-каталог).
    pub fn get_definition(&self, id: &str) -> Option<&ToolDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    /// Общая ссылка на сессию установки (для команд run/status).
    pub fn install_session(&self) -> Arc<Mutex<Option<InstallSession>>> {
        Arc::clone(&self.install_session)
    }

    /// Общий флаг отмены установки (для команд run/abort).
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.abort_install)
    }

    /// Общий доступ к хранилищу state.json (для команд и фоновой задачи).
    pub fn metadata(&self) -> Arc<Mutex<MetadataStore>> {
        Arc::clone(&self.metadata)
    }

    /// Журнал заданий (jobs.json): запись Running/терминальных статусов
    /// установок и восстановление после перезапуска.
    pub fn journal(&self) -> Arc<JobJournal> {
        Arc::clone(&self.journal)
    }

    /// Изолированное хранилище секретов (чтение/запись значений —
    /// только внутри бэкенда; наружу выдаётся только одноразовая витрина).
    pub fn secrets(&self) -> Arc<Mutex<SecretStore>> {
        Arc::clone(&self.secrets)
    }

    /// «Одноразовые» секреты последней установки (см. поле pending_secrets).
    pub fn pending_secrets(&self) -> Arc<Mutex<HashMap<String, String>>> {
        Arc::clone(&self.pending_secrets)
    }

    /// Движок read-only сканов (задания/события/кэш снапшотов).
    pub fn scan_engine(&self) -> Arc<ScanEngine> {
        Arc::clone(&self.scan_engine)
    }

    /// Канонический движок заданий (install/update/repair-path/health).
    pub fn job_engine(&self) -> Arc<JobEngine> {
        Arc::clone(&self.job_engine)
    }

    /// Информация об ОС и количестве известных инструментов.
    /// Версия ОС спрашивается у системы (быстрая команда с таймаутом).
    /// «Двойные» docker-инструменты считаются только после локальной
    /// установки (см. tc_get_health_report).
    ///
    /// ЛЕГАСИ-поверхность: счётчик считается по ОБЪЕДИНЁННОМУ каталогу
    /// (standalone + легаси-совместимость), как до выделения unity/
    /// unreal/godot в отдельный файл.
    pub async fn environment_info(&self) -> EnvironmentInfo {
        let installed = self
            .metadata()
            .lock()
            .expect("metadata poisoned")
            .data()
            .tools
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<String>>();
        let catalog = self.merged_definitions();
        let visible_count = catalog
            .iter()
            .filter(|d| {
                !core::requirements::is_dual_tool(&d.id, &catalog) || installed.contains(&d.id)
            })
            .count();
        EnvironmentInfo {
            os: std::env::consts::OS.to_string(),
            os_version: platforms::current_platform().os_version().await,
            package_managers: platforms::current_platform().package_managers(),
            tool_count: visible_count,
            capabilities: PlatformCapabilities {
                install_execution_supported: core::installer::install_execution_supported(),
                elevation_supported: std::env::consts::OS == "windows",
            },
        }
    }
}
