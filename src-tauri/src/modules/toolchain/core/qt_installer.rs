// ============================================================
// Установка Qt из официального online-репозитория (qt_installer.rs)
// ============================================================
// Источник InstallSourceKind::QtOnline: Qt ставится не «одним
// установщиком», а набором 7z-архивов из официального репозитория
// (download.qt.io/online/qtsdkrepository/...). Официальный
// aqtinstall нам не подходит (Python), поэтому повторяем то,
// что делает его ядро:
//
//   1. листинг репозитория → самый свежий каталог Qt 6.8.x
//      (например desktop/qt6_683 → версия 6.8.3);
//   2. Updates.xml репозитория → для каждого нужного пакета:
//      Version (префикс имени архивов) и Operations/Extract
//      (куда извлекать и какой архив);
//   3. для каждого архива: скачать (console::download с прогрессом)
//      и распаковать встроенным tar.exe (Windows 10+);
//   4. PATH + повторное обнаружение (verify) — как у остальных тулов.
//
// Пакеты:
//   - base  qt.qt6.<v>.win64_msvc2022_64 — ядро, Qt Widgets, Qt Quick/QML
//     (qtbase, qtdeclarative, qtsvg, qttools, qttranslations, d3dcompiler,
//     opengl32sw — всё внутри одного пакета);
//   - всегда добавляем qtshadertools — Qt6ShaderTools.dll нужна
//     qtdeclarative в рантайме (модуль не входит в base);
//   - при опции qt-webengine: extensions.qtwebengine + qt5compat +
//     qtwebchannel (WebEngine собран поверх них; WebChannel нужен
//     WebEngineWidgets, qt5compat — обязательная зависимость модуля);
//   - qt-qml / qt-widgets / qt-kirigami дополнительных пакетов
//     не требуют — всё уже в base.
//
// Windows-first: репозиторий собирается под msvc2022_64; другие
// ОС/тулчейны появятся позже.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::modules::toolchain::models::*;

use super::console::{self, ps_quote, EventSink};
use super::discovery;
use super::path_service;

/// Один пакет репозитория: имя, версия (префикс имён архивов)
/// и список «куда извлекать → какой архив» (из Operations/Extract).
#[derive(Debug, Clone, PartialEq)]
struct QtPackage {
    name: String,
    version: String,
    /// Относительный путь к каталогу пакета внутри репозитория
    /// (desktop/qt6_683/qt6_683 или extensions/qtwebengine/683/msvc2022_64).
    base_path: String,
    extracts: Vec<(String, String)>,
}

// ------------------------------------------------------------
// Разбор Updates.xml (без внешних XML-крэйтов)
// ------------------------------------------------------------

/// Значение текстового узла первого уровня: <Name>x</Name> → x.
fn xml_tag(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(block[start..end].to_string())
}

/// Список строковых аргументов внутри блока (Operations/Extract).
fn xml_args(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(start) = rest.find("<Argument>") {
        let after = &rest[start + "<Argument>".len()..];
        let Some(end) = after.find("</Argument>") else {
            break;
        };
        out.push(after[..end].to_string());
        rest = &after[end..];
    }
    out
}

/// Разбирает Updates.xml: выбирает блоки <PackageUpdate> нужных пакетов.
/// У каждого блока читаются Name, Version и все Extract-операции
/// (первый аргумент — каталог с @TargetDir@, второй — имя архива).
fn parse_updates(xml: &str, base_path: &str, want: &[&str]) -> Vec<QtPackage> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<PackageUpdate>") {
        let after = &rest[start + "<PackageUpdate>".len()..];
        let end = after.find("</PackageUpdate>").unwrap_or(after.len());
        let block = &after[..end];
        rest = &after[end..];

        let Some(name) = xml_tag(block, "Name") else {
            continue;
        };
        if !want.iter().any(|w| *w == name) {
            continue;
        }

        let version = xml_tag(block, "Version").unwrap_or_default();
        let mut extracts = Vec::new();
        let mut ops = block;
        while let Some(op_start) = ops.find("<Operation name=\"Extract\">") {
            let after = &ops[op_start + "<Operation name=\"Extract\">".len()..];
            let op_end = after.find("</Operation>").unwrap_or(after.len());
            let op_block = &after[..op_end];
            let args = xml_args(op_block);
            if args.len() >= 2 {
                extracts.push((args[0].clone(), args[1].clone()));
            }
            ops = &after[op_end..];
        }

        out.push(QtPackage {
            name,
            version,
            base_path: base_path.to_string(),
            extracts,
        });
    }
    out
}

// ------------------------------------------------------------
// Версии: каталог репозитория qt6_683 → «6.8.3»
// ------------------------------------------------------------

/// Числовой суффикс каталога (qt6_683 → "683") в человекочитаемую
/// версию. Суффикс содержит ВСЕ цифры версии: «683» = 6.8.3,
/// «680» = 6.8.0, «6103» = 6.10.3, «6110» = 6.11.0.
fn version_from_suffix(suffix: &str) -> Option<String> {
    if suffix.len() < 3 || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let major = &suffix[..1];
    let minor = &suffix[1..suffix.len() - 1];
    let patch = &suffix[suffix.len() - 1..];
    Some(format!("{major}.{minor}.{patch}"))
}

/// Из HTML-листинга каталога desktop/ вытаскивает ссылки вида
/// qt6_68x/ и возвращает самую свежую: (суффикс, версия).
/// Возвращает None, если подходящих каталогов нет.
fn pick_latest_version_dir(listing: &str) -> Option<(String, String)> {
    let mut best: Option<(u64, String, String)> = None;
    let mut rest = listing;
    while let Some(href_pos) = rest.find("href=\"") {
        let after = &rest[href_pos + "href=\"".len()..];
        let Some(end) = after.find('"') else {
            break;
        };
        let href = &after[..end];
        rest = &after[end..];
        // Нужен каталог вида .../qt6_683/ (последний сегмент пути)
        let Some(dir) = href.strip_suffix('/').and_then(|h| h.rsplit('/').next()) else {
            continue;
        };
        let Some(suffix) = dir.strip_prefix("qt6_") else {
            continue;
        };
        // Берём только ветку 6.8.x (recommended в tools.json).
        if !suffix.starts_with("68") {
            continue;
        }
        let Ok(key) = suffix.parse::<u64>() else {
            continue;
        };
        let Some(ver) = version_from_suffix(suffix) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| key > b.0) {
            best = Some((key, dir.to_string(), ver));
        }
    }
    best.map(|(_, dir, ver)| (dir, ver))
}

/// Каталог извлечения: аргумент Extract с @TargetDir@ заменяется
/// на install_dir; если пакет без операций — fallback на
/// {install_dir}/{version}/{тулчейн}.
fn resolve_target(target_arg: &str, install_dir: &Path, qt_version: &str) -> PathBuf {
    if let Some(rel) = target_arg.strip_prefix("@TargetDir@") {
        let mut path = install_dir.to_path_buf();
        for comp in rel.split(['/', '\\']).filter(|c| !c.is_empty()) {
            path.push(comp);
        }
        return path;
    }
    install_dir.join(qt_version).join("msvc2022_64")
}

// ------------------------------------------------------------
// Сеть и распаковка
// ------------------------------------------------------------

/// Скачивает небольшой текст (листинг каталога, Updates.xml) во
/// временный файл и возвращает его содержимое. PowerShell-слой —
/// как у console::download, но без tc:dl-прогресса (документы
/// маленькие, шуметь в логе не нужно).
///
/// Качает через curl.exe (не Invoke-WebRequest): download.qt.io
/// отвечает 302 на ближайшее зеркало, и IWR на части зеркал
/// нестабилен (битый DNS у qt.mirror.constant.com), а curl честно
/// идёт по редиректам — см. updates_exist ниже.
async fn fetch_text(
    url: &str,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<String, String> {
    let dest = console::tracked_temp_file("qt-fetch", ".txt");
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$code = & curl.exe -sS -L -m 120 -o {1} -w '%{{http_code}}' {0}
if ($LASTEXITCODE -ne 0 -or $code -ne '200') {{
    Write-Output "tc:error HTTP $code (curl exit=$LASTEXITCODE) для {0}"
    exit 1
}}
"#,
        ps_quote(url),
        ps_quote(&dest.to_string_lossy())
    );
    let res = console::run_tool_script(
        tool_id,
        &script,
        Some(task_id),
        index,
        total,
        session_id,
        sink,
        abort,
    )
    .await?;
    if !res.success {
        let msg = res
            .error_line
            .clone()
            .unwrap_or_else(|| format!("Не удалось получить {url}"));
        return Err(msg);
    }
    let content = std::fs::read_to_string(&dest)
        .map_err(|e| format!("Не удалось прочитать скачанный документ {dest:?}: {e}"))?;
    let _ = std::fs::remove_file(&dest);
    Ok(content)
}

/// Существует ли каталог версии в репозитории (HEAD на Updates.xml).
///
/// Зеркала download.qt.io отвечают 302 (редирект на ближайшее зеркало),
/// и Invoke-WebRequest -Method Head на части зеркал падает (битый DNS
/// qt.mirror.constant.com). curl.exe есть на всех Windows 10+ и честно
/// идёт по редиректам — используем его.
async fn updates_exist(
    repo: &str,
    version_dir: &str,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> bool {
    let url = format!("{repo}/desktop/{version_dir}/{version_dir}/Updates.xml");
    let dest = console::tracked_temp_file("qt-head", ".txt");
    let script = format!(
        r#"$code = & curl.exe -s -L -o NUL -w '%{{http_code}}' -I {0}
if ($code -eq '200') {{
    [System.IO.File]::WriteAllText({1}, 'ok', [System.Text.Encoding]::UTF8)
    exit 0
}}
exit 1
"#,
        ps_quote(&url),
        ps_quote(&dest.to_string_lossy())
    );
    let res = console::run_tool_script(
        tool_id,
        &script,
        Some(task_id),
        index,
        total,
        session_id,
        sink,
        abort,
    )
    .await;
    let ok = matches!(res, Ok(r) if r.success);
    let _ = std::fs::remove_file(&dest);
    ok
}

/// Определяет самую свежую версию Qt 6.8 в репозитории.
/// Возвращает (суффикс каталога, человекочитаемая версия).
async fn resolve_latest_version(
    repo: &str,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<(String, String), String> {
    // Живой листинг каталога desktop/ — надёжнее всего.
    let listing_url = format!("{repo}/desktop/");
    match fetch_text(
        &listing_url,
        index,
        total,
        task_id,
        tool_id,
        session_id,
        sink,
        Arc::clone(&abort),
    )
    .await
    {
        Ok(listing) => {
            if let Some(found) = pick_latest_version_dir(&listing) {
                return Ok(found);
            }
        }
        Err(e) => {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:info Листинг репозитория недоступен ({e}) — пробую известные каталоги"
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
        }
    }

    // Fallback: известные каталоги 6.8.x по убыванию.
    for suffix in ["683", "682", "681", "680"] {
        if updates_exist(
            repo,
            suffix,
            index,
            total,
            task_id,
            tool_id,
            session_id,
            sink,
            Arc::clone(&abort),
        )
        .await
        {
            let version_dir = format!("qt6_{suffix}");
            return Ok((version_dir, version_from_suffix(suffix).unwrap_or_default()));
        }
    }

    Err("Не удалось определить доступную версию Qt 6.8 в репозитории".to_string())
}

/// Распаковка архива Qt (7z-контейнеры читает встроенный tar Windows 10+)
/// через traversal-безопасный слой archive.rs: список записей проверяется
/// ДО извлечения (zip-slip/tar-slip/симлинки отклоняются fail-closed).
async fn extract_archive(
    archive: &Path,
    target: &Path,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<(), String> {
    super::archive::extract_tar_safe(
        archive, target, tool_id, task_id, index, total, session_id, sink, abort,
    )
    .await
}

// ------------------------------------------------------------
// Главная функция установки
// ------------------------------------------------------------

/// Полная установка Qt из online-репозитория.
/// Возвращает Ok((версия, секрет)) при подтверждённой установке.
pub async fn install_qt_online(
    def: &ToolDefinition,
    source: &InstallSource,
    install_options: &[String],
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<(String, Option<String>), String> {
    let repo = source
        .url
        .as_deref()
        .ok_or_else(|| format!("{}: Qt-источник без url", source.id))?;
    // Репозиторий Qt — внешняя граница доверия: только https.
    console::validate_download_url(repo)?;
    let install_dir = source
        .install_dir
        .as_deref()
        .ok_or_else(|| format!("{}: Qt-источник без install_dir", source.id))?;
    let install_dir = path_service::expand_env_vars(install_dir);
    let install_dir = PathBuf::from(&install_dir);

    sink.emit(console::event(
        ToolchainEventType::TaskPhaseChanged {
            phase: TaskPhase::Installing,
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: "tc:info Установка Qt из официального репозитория (download.qt.io)".to_string(),
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));

    // 1. Самая свежая версия 6.8.x
    let (version_dir, qt_version) = resolve_latest_version(
        repo,
        index,
        total,
        task_id,
        tool_id,
        session_id,
        sink,
        Arc::clone(&abort),
    )
    .await?;
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: format!("tc:info Найдена версия Qt {qt_version}"),
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));

    // 2. Список пакетов: base + qtshadertools + модули по опциям.
    //    version_dir вида qt6_683; в именах пакетов и extension-репо
    //    используется числовой суффикс (683).
    let version_suffix = version_dir
        .strip_prefix("qt6_")
        .unwrap_or(&version_dir)
        .to_string();
    let toolchain = "win64_msvc2022_64".to_string();
    let mut want: Vec<String> = vec![format!("qt.qt6.{version_suffix}.{toolchain}")];
    let mut extension_want: Vec<String> = Vec::new();
    let webengine = install_options.iter().any(|o| o == "qt-webengine");
    if webengine {
        extension_want.push(format!(
            "extensions.qtwebengine.{version_suffix}.{toolchain}"
        ));
        want.push(format!(
            "qt.qt6.{version_suffix}.addons.qt5compat.{toolchain}"
        ));
        want.push(format!(
            "qt.qt6.{version_suffix}.addons.qtwebchannel.{toolchain}"
        ));
    }
    // Qt6ShaderTools.dll нужна qtdeclarative (Qt Quick) в рантайме —
    // в base-пакет она не входит, ставим всегда.
    want.push(format!(
        "qt.qt6.{version_suffix}.addons.qtshadertools.{toolchain}"
    ));

    // 3. Метаданные из Updates.xml (desktop + extensions).
    let desktop_base = format!("desktop/{version_dir}/{version_dir}");
    let desktop_xml = fetch_text(
        &format!("{repo}/{desktop_base}/Updates.xml"),
        index,
        total,
        task_id,
        tool_id,
        session_id,
        sink,
        Arc::clone(&abort),
    )
    .await
    .map_err(|e| format!("Не удалось получить каталог пакетов Qt: {e}"))?;
    let want_refs: Vec<&str> = want.iter().map(|s| s.as_str()).collect();
    let mut packages = parse_updates(&desktop_xml, &desktop_base, &want_refs);
    if webengine {
        let ext_base = format!("extensions/qtwebengine/{version_suffix}/msvc2022_64");
        let ext_xml = fetch_text(
            &format!("{repo}/{ext_base}/Updates.xml"),
            index,
            total,
            task_id,
            tool_id,
            session_id,
            sink,
            Arc::clone(&abort),
        )
        .await
        .map_err(|e| format!("Не удалось получить каталог модуля WebEngine: {e}"))?;
        let ext_refs: Vec<&str> = extension_want.iter().map(|s| s.as_str()).collect();
        packages.extend(parse_updates(&ext_xml, &ext_base, &ext_refs));
    }
    if packages.is_empty() {
        return Err(
            "В репозитории не найдены пакеты Qt 6.8 — возможно, каталог версии изменился"
                .to_string(),
        );
    }
    let expected_total = want.len() + extension_want.len();
    if packages.len() < expected_total {
        // Честно сообщаем, каких пакетов не хватило (а не молча ставим без них).
        let found: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        let missing: Vec<&str> = want
            .iter()
            .chain(extension_want.iter())
            .filter(|w| !found.contains(&w.as_str()))
            .map(|s| s.as_str())
            .collect();
        return Err(format!(
            "В репозитории не хватает пакетов: {}",
            missing.join(", ")
        ));
    }

    // 4. Скачивание и распаковка по очереди.
    let archive_count: usize = packages.iter().map(|p| p.extracts.len()).sum();
    let mut done_archives = 0usize;
    for pkg in &packages {
        if pkg.extracts.is_empty() {
            return Err(format!(
                "Пакет {} не содержит операций извлечения",
                pkg.name
            ));
        }
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!("tc:info Пакет: {}", pkg.name),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        for (target_arg, archive) in &pkg.extracts {
            done_archives += 1;
            if abort.load(Ordering::SeqCst) {
                return Err("Отменено пользователем".to_string());
            }

            let target = resolve_target(target_arg, &install_dir, &qt_version);
            let url = format!(
                "{repo}/{}/{}/{}{archive}",
                pkg.base_path, pkg.name, pkg.version
            );
            // Уникальный temp-файл на каждый архив (без предсказуемого имени).
            let ext = Path::new(archive)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".bin".to_string());
            let dest = console::tracked_temp_file("qt-pkg", &ext);

            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:info [{done_archives}/{archive_count}] Скачивание: {archive}"
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            console::download(
                &url,
                &dest,
                None, // у репозитория Qt нет контрольных сумм в Updates.xml — честный unverified
                index,
                total,
                task_id,
                tool_id,
                session_id,
                sink,
                Arc::clone(&abort),
            )
            .await
            .map_err(|e| format!("Пакет {}: {e}", pkg.name))?;

            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!("tc:info Распаковка в: {}", target.to_string_lossy()),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            extract_archive(
                &dest,
                &target,
                index,
                total,
                task_id,
                tool_id,
                session_id,
                sink,
                Arc::clone(&abort),
            )
            .await
            .map_err(|e| format!("Пакет {}: {e}", pkg.name))?;

            let _ = std::fs::remove_file(&dest);
        }
    }

    // 5. PATH: qmake/мета-тулсы Qt должны быть видны из терминала.
    if !def.path_entries.is_empty() {
        sink.emit(console::event(
            ToolchainEventType::TaskPhaseChanged {
                phase: TaskPhase::UpdatingPath,
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        if let Err(e) = path_service::add_to_user_path(&def.path_entries).await {
            eprintln!("[toolchain] не удалось добавить PATH для {tool_id}: {e}");
        }
    }
    if let Err(e) = path_service::sync_process_path().await {
        eprintln!("[toolchain] не удалось обновить PATH процесса: {e}");
    }

    // 6. Проверка установки тем же discovery, что и в проверке окружения.
    sink.emit(console::event(
        ToolchainEventType::TaskPhaseChanged {
            phase: TaskPhase::Verifying,
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));
    match discovery::detect_tool(def).await {
        ToolStatus::Installed { version } => Ok((version, None)),
        _ => Err("Установка не подтвердилась (qmake не найден)".to_string()),
    }
}

// ============================================================
// Тесты (чистая логика — без сети)
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<Updates>
 <PackageUpdate>
  <Name>qt.qt6.683.win64_msvc2022_64</Name>
  <Version>6.8.3-0-202503201308</Version>
  <Dependencies>qt.tools.qtcreator</Dependencies>
  <DownloadableArchives>qtbase.7z, qtsvg.7z</DownloadableArchives>
  <Operations>
   <Operation name="Extract">
    <Argument>@TargetDir@/6.8.3/msvc2022_64</Argument>
    <Argument>qtbase.7z</Argument>
   </Operation>
   <Operation name="Extract">
    <Argument>@TargetDir@/6.8.3/msvc2022_64/bin</Argument>
    <Argument>qtsvg.7z</Argument>
   </Operation>
  </Operations>
 </PackageUpdate>
 <PackageUpdate>
  <Name>qt.qt6.683.addons.qt5compat.win64_msvc2022_64</Name>
  <Version>6.8.3-0-202503201308</Version>
  <DownloadableArchives>qt5compat.7z</DownloadableArchives>
  <Operations>
   <Operation name="Extract">
    <Argument>@TargetDir@/6.8.3/msvc2022_64</Argument>
    <Argument>qt5compat.7z</Argument>
   </Operation>
  </Operations>
 </PackageUpdate>
 <PackageUpdate>
  <Name>qt.qt6.610.addons.qtshadertools.win64_msvc2022_64</Name>
  <Version>6.10.3-0-202505201308</Version>
  <DownloadableArchives>qtshadertools.7z</DownloadableArchives>
  <Operations>
   <Operation name="Extract">
    <Argument>@TargetDir@/6.10.3/msvc2022_64</Argument>
    <Argument>qtshadertools.7z</Argument>
   </Operation>
  </Operations>
 </PackageUpdate>
</Updates>"#;

    #[test]
    fn parse_picks_requested_packages_only() {
        let want = ["qt.qt6.683.addons.qt5compat.win64_msvc2022_64"];
        let pkgs = parse_updates(SAMPLE_XML, "desktop/qt6_683/qt6_683", &want);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].version, "6.8.3-0-202503201308");
        assert_eq!(
            pkgs[0].extracts,
            vec![(
                "@TargetDir@/6.8.3/msvc2022_64".to_string(),
                "qt5compat.7z".to_string()
            )]
        );
    }

    #[test]
    fn parse_keeps_extract_order_and_targets() {
        let want = ["qt.qt6.683.win64_msvc2022_64"];
        let pkgs = parse_updates(SAMPLE_XML, "desktop/qt6_683/qt6_683", &want);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].extracts.len(), 2);
        // второй Extract — bin-каталог (d3dcompiler/opengl32sw живут в bin/)
        assert_eq!(
            pkgs[0].extracts[1],
            (
                "@TargetDir@/6.8.3/msvc2022_64/bin".to_string(),
                "qtsvg.7z".to_string()
            )
        );
    }

    #[test]
    fn parse_skips_unwanted_packages() {
        let pkgs = parse_updates(SAMPLE_XML, "desktop/qt6_683/qt6_683", &[]);
        assert!(pkgs.is_empty());
    }

    #[test]
    fn version_suffix_variants() {
        assert_eq!(version_from_suffix("683").as_deref(), Some("6.8.3"));
        assert_eq!(version_from_suffix("680").as_deref(), Some("6.8.0"));
        assert_eq!(version_from_suffix("6103").as_deref(), Some("6.10.3"));
        assert_eq!(version_from_suffix("6110").as_deref(), Some("6.11.0"));
        assert_eq!(version_from_suffix("6120").as_deref(), Some("6.12.0"));
        assert_eq!(version_from_suffix("68"), None);
        assert_eq!(version_from_suffix("abc"), None);
        assert_eq!(version_from_suffix(""), None);
    }

    #[test]
    fn listing_picks_latest_68x() {
        let listing = r#"<html><body>
<a href="/online/qtsdkrepository/windows_x86/desktop/">Parent</a>
<a href="/online/qtsdkrepository/windows_x86/desktop/qt6_681/">qt6_681/</a>
<a href="/online/qtsdkrepository/windows_x86/desktop/qt6_683/">qt6_683/</a>
<a href="/online/qtsdkrepository/windows_x86/desktop/qt6_680/">qt6_680/</a>
<a href="/online/qtsdkrepository/windows_x86/desktop/qt6_6103/">qt6_6103/</a>
<a href="/online/qtsdkrepository/windows_x86/desktop/dev/">dev/</a>
</body></html>"#;
        assert_eq!(
            pick_latest_version_dir(listing),
            Some(("qt6_683".to_string(), "6.8.3".to_string()))
        );
    }

    #[test]
    fn listing_without_68x_is_none() {
        let listing = r#"<a href="/x/qt6_6103/">qt6_6103/</a><a href="/x/dev/">dev/</a>"#;
        assert_eq!(pick_latest_version_dir(listing), None);
    }

    #[test]
    fn target_dir_replaced_with_install_dir() {
        let target = resolve_target("@TargetDir@/6.8.3/msvc2022_64", Path::new("C:/Qt"), "6.8.3");
        assert_eq!(target, PathBuf::from("C:/Qt/6.8.3/msvc2022_64"));
    }

    #[test]
    fn target_dir_fallback_for_packages_without_ops() {
        let target = resolve_target("", Path::new("C:/Qt"), "6.8.3");
        assert_eq!(target, PathBuf::from("C:/Qt/6.8.3/msvc2022_64"));
    }

    // Временная live-проверка против реального репозитория.
    // Запуск: cargo test --lib live_repo -- --ignored
    #[tokio::test]
    #[ignore]
    async fn live_repo_resolution_and_parsing() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct LiveSink {
            events: Mutex<Vec<String>>,
        }
        impl EventSink for LiveSink {
            fn emit(&self, event: ToolchainEvent) {
                if let ToolchainEventType::TaskProgress { line } = event.event_type {
                    self.events.lock().unwrap().push(line);
                }
            }
        }

        let sink: Arc<dyn EventSink> = Arc::new(LiveSink::default());
        let repo = "https://download.qt.io/online/qtsdkrepository/windows_x86";
        let abort = Arc::new(AtomicBool::new(false));

        let (version_dir, qt_version) =
            resolve_latest_version(repo, 0, 1, "qt", "qt", "s-live", &sink, abort.clone())
                .await
                .expect("resolve_latest_version");
        eprintln!("live: version_dir={version_dir}, version={qt_version}");
        assert!(
            qt_version.starts_with("6.8."),
            "ожидали 6.8.x: {qt_version}"
        );

        let suffix = version_dir.strip_prefix("qt6_").unwrap();
        let toolchain = "win64_msvc2022_64";
        let want = [
            format!("qt.qt6.{suffix}.{toolchain}"),
            format!("qt.qt6.{suffix}.addons.qtshadertools.{toolchain}"),
            format!("qt.qt6.{suffix}.addons.qt5compat.{toolchain}"),
        ];
        let desktop_xml = fetch_text(
            &format!("{repo}/desktop/{version_dir}/{version_dir}/Updates.xml"),
            0,
            1,
            "qt",
            "qt",
            "s-live",
            &sink,
            abort.clone(),
        )
        .await
        .expect("desktop Updates.xml");
        let want_refs: Vec<&str> = want.iter().map(|s| s.as_str()).collect();
        let packages = parse_updates(
            &desktop_xml,
            &format!("desktop/{version_dir}/{version_dir}"),
            &want_refs,
        );
        eprintln!("live: packages = {packages:?}");
        assert_eq!(packages.len(), 3, "все пакеты должны найтись");

        // base-пакет (в XML идёт ПОСЛЕ addons — каталог отсортирован по алфавиту)
        let base = packages
            .iter()
            .find(|p| p.name == format!("qt.qt6.{suffix}.win64_msvc2022_64"))
            .expect("base-пакет");
        assert_eq!(base.extracts.len(), 7, "base: 7 архивов");
        assert!(base.extracts.iter().all(|(t, _)| t.contains("msvc2022_64")));
        assert!(
            base.version.starts_with("6.8.3-0-"),
            "префикс версии: {}",
            base.version
        );

        // URL-шаблон должен существовать: HEAD на первый архив.
        let (target, archive) = &base.extracts[0];
        let url = format!(
            "{repo}/{}/{}/{}{archive}",
            base.base_path, base.name, base.version
        );
        let dest = std::env::temp_dir().join("tc-qt-live-head.txt");
        let script = format!(
            r#"$code = & curl.exe -s -L -o NUL -w '%{{http_code}}' -I {0}
if ($code -eq '200') {{ exit 0 }}
exit 1
"#,
            ps_quote(&url)
        );
        let res = console::run_tool_script(
            "qt",
            &script,
            Some("qt"),
            0,
            1,
            "s-live",
            &sink,
            abort.clone(),
        )
        .await
        .expect("head script");
        assert!(res.success, "HEAD {url} должен отвечать 200");
        eprintln!("live: OK — {url}, target={target}");
        let _ = std::fs::remove_file(&dest);
    }
}
