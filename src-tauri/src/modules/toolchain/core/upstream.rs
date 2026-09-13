// ============================================================
// Резолверы актуальных версий (upstream.rs)
// ============================================================
// Защита от устаревания каталога: версии инструментов больше не
// зашиваются намертво в tools.json — для источников с url_template
// и/или version_resolver актуальная версия запрашивается у ОФИЦИАЛЬНЫХ
// апстримов на лету, при установке и при сканировании окружения.
//
// Принципы надёжности:
//   - HTTP-клиент — системный curl (есть на Windows 10+, macOS, почти
//     всех Linux): без новых зависимостей, TLS у системы;
//   - кэш с TTL (по умолчанию 6 ч) для сканов; УСТАНОВКА всегда
//     резолвит СВЕЖУЮ версию (обходя кэш) — на свежий релиз не
//     наступит «кэшированная вчерашняя ссылка»;
//   - статичные значения tools.json остаются страховкой: резолвер
//     недоступен → используем зашитую версию (устаревшее лучше
//     сломанного), источник при этом честно помечается в логе;
//   - каждая операция ограничена таймаутом, вывод — лимитом размера;
//   - все резолверы кроссплатформенны (ни одной Windows-специфики).
//
// Формат резолверов — декларативный (models::VersionResolver), логика
// парсинга каждого апстрима живёт здесь.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::modules::toolchain::models::{ToolDefinition, VersionResolver};

use super::version;

// ------------------------------------------------------------
// HTTP
// ------------------------------------------------------------

/// Таймаут одного HTTP-запроса резолвера: версия должна приходить
/// быстро, а не висеть скан (статичная страховка всегда на месте).
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Максимальный размер ответа резолвера (индексные файлы небольшие;
/// граница защищает от «бесконечного» потока со сломанного сервера).
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// GET по HTTPS через системный curl: `-fsSL` (тихий, fail на ошибку,
/// редиректы), таймаут, User-Agent. Возвращает stdout или Err.
async fn http_get(url: &str) -> Result<String, String> {
    use tokio::process::Command as TokioCommand;
    use tokio::time::timeout;

    let mut cmd = TokioCommand::new("curl");
    cmd.args([
        "-fsSL",
        "--max-time",
        "20",
        "-A",
        "StackPilot/1.2 (toolchain version resolver)",
        "--",
        url,
    ]);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);

    let output = timeout(HTTP_TIMEOUT + Duration::from_secs(5), cmd.output())
        .await
        .map_err(|_| format!("Таймаут запроса версии: {url}"))?
        .map_err(|e| format!("Не удалось запустить curl (нужен для версии {url}): {e}"))?;

    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        let why = why.trim();
        return Err(if why.is_empty() {
            format!("HTTP-запрос версии не удался (код {}): {url}", output.status)
        } else {
            format!("HTTP-запрос версии не удался ({why}): {url}")
        });
    }

    if output.stdout.len() > MAX_RESPONSE_BYTES {
        return Err(format!("Ответ версии слишком большой: {url}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Проверяет доступность curl один раз (кэш результата).
fn curl_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| which::which("curl").is_ok())
}

// ------------------------------------------------------------
// Кэш
// ------------------------------------------------------------

/// Время жизни кэша резолверов. Сканы используют кэш (быстро),
/// установки резолвят свежую версию в обход кэша.
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

struct CachedResolution {
    resolved: ResolvedVersion,
    at: Instant,
}

impl Clone for CachedResolution {
    fn clone(&self) -> Self {
        Self {
            resolved: self.resolved.clone(),
            at: self.at,
        }
    }
}

fn cache() -> &'static Mutex<HashMap<String, CachedResolution>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedResolution>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ------------------------------------------------------------
// Модель результата
// ------------------------------------------------------------

/// Результат резолва: версия + (опционально) готовый URL и SHA-256.
/// URL/SHA-256 отсутствуют, когда апстрим отдаёт только номер версии
/// (тогда URL собирается из url_template каталога).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub version: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
}

impl ResolvedVersion {
    fn version_only(version: &str) -> Self {
        Self {
            version: version.to_string(),
            url: None,
            sha256: None,
        }
    }
}

/// Заполняет шаблон URL плейсхолдером версии:
/// `https://x/{version}/a-{version}.zip` → подстановка версии.
/// Неизвестных плейсхолдеров в шаблонах быть не должно — вернётся Err.
pub fn fill_template(template: &str, version: &str) -> Result<String, String> {
    let out = template.replace("{version}", version);
    // Оставшиеся `{...}` — опечатка в каталоге: честная ошибка,
    // а не мусорный URL с фигурными скобками.
    if out.contains('{') || out.contains('}') {
        return Err(format!(
            "Шаблон URL содержит незаполненный плейсхолдер: {template}"
        ));
    }
    Ok(out)
}

// ------------------------------------------------------------
// Публичный API
// ------------------------------------------------------------

/// Резолвит актуальную версию для источника/политики каталога.
///
/// `fresh`:
///   - true — установка: всегда свежий запрос к апстриму; при неудаче
///     используется кэш (если есть), иначе Err (caller откатывается
///     на статичный URL каталога);
///   - false — скан/UI: кэш с TTL, при простое — свежий запрос.
pub async fn resolve(
    resolver: &VersionResolver,
    fresh: bool,
) -> Result<ResolvedVersion, String> {
    if !curl_available() {
        return Err("curl недоступен — резолвер версии не может работать".to_string());
    }

    let key = resolver.cache_key();

    // Установка: свежий запрос, кэш — только страховка при сбое.
    if fresh {
        match fetch(resolver).await {
            Ok(resolved) => {
                cache().lock().unwrap_or_else(|p| p.into_inner()).insert(
                    key,
                    CachedResolution {
                        resolved: resolved.clone(),
                        at: Instant::now(),
                    },
                );
                return Ok(resolved);
            }
            Err(e) => {
                if let Some(cached) = cache()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&key)
                    .cloned()
                {
                    log::warn!(
                        "[toolchain] резолвер {key} недоступен ({e}) — используется кэшированная версия {}",
                        cached.resolved.version
                    );
                    return Ok(cached.resolved);
                }
                return Err(e);
            }
        }
    }

    // Скан: кэш с TTL.
    if let Some(cached) = cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key)
        .cloned()
    {
        if cached.at.elapsed() < CACHE_TTL {
            return Ok(cached.resolved);
        }
    }
    match fetch(resolver).await {
        Ok(resolved) => {
            cache()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    key,
                    CachedResolution {
                        resolved: resolved.clone(),
                        at: Instant::now(),
                    },
                );
            Ok(resolved)
        }
        Err(e) => Err(e),
    }
}

/// Эффективная рекомендуемая версия инструмента: результат резолвера
/// (кэш с TTL) либо статичная recommended из tools.json.
/// Установленные ниже неё инструменты получают UpdateAvailable.
pub async fn effective_recommended(def: &ToolDefinition) -> Option<String> {
    let Some(resolver) = def.versions.resolver.as_ref() else {
        return def.versions.recommended.clone();
    };
    match resolve(resolver, false).await {
        Ok(r) => {
            log::info!(
                "[toolchain] recommended {}: статика {} → резолв {}",
                def.id,
                def.versions.recommended.as_deref().unwrap_or("-"),
                r.version
            );
            Some(r.version)
        }
        Err(e) => {
            log::debug!(
                "[toolchain] резолвер {} недоступен ({e}) — статичная recommended",
                def.id
            );
            def.versions.recommended.clone()
        }
    }
}

/// Резолвит recommended для пачки инструментов параллельно.
/// Возвращает map tool_id → эффективная recommended (резолв или статика).
/// Ограничено дедлайном: незавершённые к дедлайну остаются статичными —
/// скан не обязан ждать сеть.
pub async fn resolve_recommended_batch(
    definitions: &[ToolDefinition],
    deadline: Duration,
) -> HashMap<String, String> {
    use tokio::task::JoinSet;

    let mut set: JoinSet<(String, String)> = JoinSet::new();
    for def in definitions {
        let id = def.id.clone();
        if def.versions.resolver.is_none() {
            if let Some(static_rec) = def.versions.recommended.clone() {
                set.spawn(async move { (id, static_rec) });
            }
            continue;
        }
        let def = def.clone();
        set.spawn(async move {
            let rec = effective_recommended(&def).await;
            (id, rec.unwrap_or_default())
        });
    }

    let mut out: HashMap<String, String> = HashMap::new();
    let collect = async {
        while let Some(res) = set.join_next().await {
            if let Ok((id, rec)) = res {
                out.insert(id, rec);
            }
        }
    };
    if tokio::time::timeout(deadline, collect).await.is_err() {
        log::warn!(
            "[toolchain] резолв recommended не уложился в дедлайн — остальные инструменты останутся со статичными версиями"
        );
        set.abort_all();
    }
    out
}

// ------------------------------------------------------------
// Резолверы
// ------------------------------------------------------------

async fn fetch(resolver: &VersionResolver) -> Result<ResolvedVersion, String> {
    match resolver {
        VersionResolver::Ziglang => resolve_ziglang().await,
        VersionResolver::Hashicorp { product } => resolve_hashicorp(product).await,
        VersionResolver::Github { repo, asset } => resolve_github(repo, asset).await,
        VersionResolver::PhpWindows { branch } => resolve_php_windows(branch).await,
        VersionResolver::Gradle => resolve_gradle().await,
        VersionResolver::MavenApache => resolve_maven().await,
        VersionResolver::ApacheKafka => resolve_kafka().await,
        VersionResolver::Grafana => resolve_grafana().await,
        VersionResolver::Swift => resolve_swift().await,
        VersionResolver::MongoDb => resolve_mongodb().await,
        VersionResolver::Winget { id } => resolve_winget(id).await,
        VersionResolver::DotnetChannel { channel } => resolve_dotnet_channel(channel).await,
    }
}

/// Выбор «последнего стабильного» из списка версий: отбрасываются
/// пре-релизы (содержат -, rc, alpha, beta, dev, preview) и
/// неразбираемые номера; выбирается максимальная по version::compare.
fn latest_stable<'a>(versions: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    versions
        .filter(|v| {
            let lower = v.to_ascii_lowercase();
            !lower.contains('-')
                && !lower.contains("rc")
                && !lower.contains("alpha")
                && !lower.contains("beta")
                && !lower.contains("dev")
                && !lower.contains("preview")
                && !lower.contains('+')
                && version::parse_version(v).is_ok()
        })
        .max_by(|a, b| {
            version::compare(
                &version::parse_version(a).unwrap_or_default(),
                &version::parse_version(b).unwrap_or_default(),
            )
        })
}

// --- Zig: ziglang.org/download/index.json --------------------------

async fn resolve_ziglang() -> Result<ResolvedVersion, String> {
    let body = http_get("https://ziglang.org/download/index.json").await?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|e| format!("index.json zig не разобрался: {e}"))?;
    let Some(obj) = json.as_object() else {
        return Err("index.json zig: не объект".to_string());
    };
    let Some(version) = latest_stable(obj.keys().map(String::as_str)) else {
        return Err("index.json zig: стабильный релиз не найден".to_string());
    };
    let entry = obj.get(version).and_then(|v| v.as_object());
    let windows = entry
        .and_then(|e| e.get("x86_64-windows"))
        .and_then(|w| w.as_object());
    let url = windows
        .and_then(|w| w.get("tarball"))
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let sha256 = windows
        .and_then(|w| w.get("shasum"))
        .and_then(|s| s.as_str())
        .map(str::to_string);
    Ok(ResolvedVersion {
        version: version.to_string(),
        url,
        sha256,
    })
}

// --- HashiCorp: releases.hashicorp.com/<product>/index.json ---------

/// Парсит index.json HashiCorp: `{"name": ..., "versions": {v: {builds: [...]}}}`.
/// Возвращает ВСЕ стабильные windows/amd64 релизы, отсортированные по
/// убыванию версии (первый — самый свежий). Индекс НЕ отсортирован
/// гарантированно (порядок вставки 0.1.0 → 1.16.0), поэтому сортировка
/// числовая, а не лексикографическая.
fn parse_hashicorp_candidates(body: &str) -> Vec<(String, String, String)> {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let Some(versions) = json.get("versions").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut candidates: Vec<(Vec<u32>, String, String, String)> = Vec::new();
    for version in versions.keys() {
        if version.contains('-') || version::parse_version(version).is_err() {
            continue; // пре-релизы (rc/beta/alpha/dev) пропускаются
        }
        // Версии без builds (старые релизы без манифеста) — не повод
        // отказываться от функции: переход к следующей версии.
        let Some(entry) = versions.get(version) else {
            continue;
        };
        let Some(builds) = entry.get("builds").and_then(|b| b.as_array()) else {
            continue;
        };
        let Some(build) = builds.iter().find_map(|b| {
            let b = b.as_object()?;
            (b.get("os")?.as_str() == Some("windows") && b.get("arch")?.as_str() == Some("amd64"))
                .then_some(b)
        }) else {
            continue;
        };
        let Some(url) = build.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let Some(filename) = build.get("filename").and_then(|f| f.as_str()) else {
            continue;
        };
        if !url.starts_with("https://") {
            continue;
        }
        candidates.push((
            version::parse_version(version).unwrap_or_default(),
            version.to_string(),
            url.to_string(),
            filename.to_string(),
        ));
    }
    candidates.sort_by(|a, b| version::compare(&b.0, &a.0));
    candidates
        .into_iter()
        .map(|(_, version, url, filename)| (version, url, filename))
        .collect()
}

/// Совместимая обёртка для тестов: самый свежий стабильный
/// windows/amd64 релиз.
#[cfg(test)]
fn parse_hashicorp_index(body: &str, _product: &str) -> Option<(String, String, String)> {
    parse_hashicorp_candidates(body).into_iter().next()
}

/// HEAD-проверка доступности URL: HashiCorp удаляет артефакты старых
/// релизов, и index.json может перечислять версию, чьи бинари уже
/// отдаются 404 (или релиз был отозван). Возвращает true только на 2xx.
async fn http_head_ok(url: &str) -> bool {
    use tokio::process::Command as TokioCommand;
    use tokio::time::timeout;

    let null_device = if cfg!(target_os = "windows") { "NUL" } else { "/dev/null" };
    let mut cmd = TokioCommand::new("curl");
    cmd.args(["-fsSIL", "--max-time", "20", "-o", null_device, "--", url]);
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);
    matches!(
        timeout(HTTP_TIMEOUT + Duration::from_secs(5), cmd.status()).await,
        Ok(Ok(st)) if st.success()
    )
}

/// SHA-256 из отдельного файла SHA256SUMS релиза HashiCorp
/// (`<hash>  <filename>`), если он доступен.
async fn hashicorp_sha256(product: &str, version: &str, filename: &str) -> Option<String> {
    let sums_url =
        format!("https://releases.hashicorp.com/{product}/{version}/{product}_{version}_SHA256SUMS");
    let body = http_get(&sums_url).await.ok()?;
    let line = body.lines().find(|l| l.trim_end().ends_with(filename))?;
    line.split_whitespace().next().map(str::to_string)
}

/// Резолвер HashiCorp: берёт список стабильных windows/amd64 релизов по
/// убыванию и выбирает ПЕРВЫЙ, чей артефакт реально доступен (HEAD 2xx).
/// Защита от «фантомных» версий в index.json: HashiCorp удаляет бинари
/// отозванных/старых релизов, и новейшая версия может отдаваться 404
/// (именно так выглядел сбой terraform 1.16.2: версия в индексе была,
/// файла на CDN — нет).
async fn resolve_hashicorp(product: &str) -> Result<ResolvedVersion, String> {
    let url = format!("https://releases.hashicorp.com/{product}/index.json");
    let body = http_get(&url).await?;
    let candidates = parse_hashicorp_candidates(&body);
    if candidates.is_empty() {
        return Err(format!(
            "index.json {product}: стабильный windows/amd64 релиз не найден"
        ));
    }
    for (version, candidate_url, filename) in candidates.iter().take(8) {
        if http_head_ok(candidate_url).await {
            let sha256 = hashicorp_sha256(product, version, filename).await;
            return Ok(ResolvedVersion {
                version: version.clone(),
                url: Some(candidate_url.clone()),
                sha256,
            });
        }
        log::debug!(
            "[toolchain] hashicorp {product} {version}: артефакт недоступен (HEAD не 2xx) — пробую предыдущий"
        );
    }
    // Все проверенные версии недоступны: честная ошибка, caller откатится
    // на статичный URL каталога (тоже может быть 404 — но хотя бы понятно,
    // что проблема у апстрима, а не в установщике).
    Err(format!(
        "index.json {product}: ни один из последних стабильных релизов не доступен на CDN (404)"
    ))
}

// --- GitHub: api.github.com/repos/<repo>/releases/latest -------------

/// Нормализует версию из тега релиза: срезает ЛЮБОЙ нецифровой префикс
/// — "v2.4.20" → "2.4.20", "OTP-29.0.5" → "29.0.5", "8.10.1" → "8.10.1".
/// Префикс "v" — де-факто стандарт, но Erlang ("OTP-") и другие не
/// обязаны ему следовать.
fn github_version_from_tag(tag: &str) -> String {
    let start = tag
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(0);
    tag[start..].to_string()
}

/// Ищет ассет релиза: при паттерне с {version} имя собирается из
/// нормализованной версии тега; без {version} — точное имя ассета
/// (elixir: "elixir-otp-29.zip" — имя не содержит версии elixir).
fn github_asset_url(
    json: &Value,
    asset_pattern: &str,
    version: &str,
) -> Option<String> {
    let assets = json.get("assets")?.as_array()?;
    let want = if asset_pattern.contains("{version}") {
        asset_pattern.replace("{version}", version)
    } else {
        asset_pattern.to_string()
    };
    assets.iter().find_map(|a| {
        let a = a.as_object()?;
        if a.get("name")?.as_str() != Some(want.as_str()) {
            return None;
        }
        a.get("browser_download_url")?.as_str().map(str::to_string)
    })
}

async fn resolve_github(repo: &str, asset: &str) -> Result<ResolvedVersion, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = http_get(&url).await?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|e| format!("релиз GitHub {repo} не разобрался: {e}"))?;
    let tag = json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("релиз GitHub {repo}: нет tag_name"))?;
    let version = github_version_from_tag(tag);
    let asset_url = github_asset_url(&json, asset, &version);
    Ok(ResolvedVersion {
        version,
        url: asset_url,
        sha256: None,
    })
}

// --- Swift: www.swift.org/api/v1/install/releases.json ----------------

/// Парсит releases.json (массив релизов): максимальный стабильный
/// `name` (например "6.3.3"; пре-релизы не помечены, но имена с
/// суффиксами отсекаются latest_stable'ом).
fn parse_swift_releases(body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    let releases = json.as_array()?;
    let names: Vec<&str> = releases
        .iter()
        .filter_map(|r| r.get("name").and_then(|n| n.as_str()))
        .collect();
    latest_stable(names.into_iter()).map(str::to_string)
}

async fn resolve_swift() -> Result<ResolvedVersion, String> {
    let body = http_get("https://www.swift.org/api/v1/install/releases.json").await?;
    let version = parse_swift_releases(&body)
        .ok_or_else(|| "swift.org: стабильный релиз не найден".to_string())?;
    Ok(ResolvedVersion::version_only(&version))
}

// --- MongoDB: downloads.mongodb.org/current.json ----------------------

/// Парсит current.json: максимальный production-релиз (не release
/// candidate) с windows/x86_64 base-сборкой; отдаёт версию, URL и SHA-256.
/// ВАЖНО: сборки лежат в ключе `downloads` (не `builds`).
fn parse_mongodb_current(body: &str) -> Option<(String, String, String)> {
    let json: Value = serde_json::from_str(body).ok()?;
    let versions = json.get("versions")?.as_array()?;
    let mut best: Option<(Vec<u32>, String, String, String)> = None;
    for entry in versions {
        let entry = entry.as_object()?;
        let version = entry.get("version")?.as_str()?;
        let production = entry
            .get("production_release")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let rc = entry
            .get("release_candidate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !production || rc || version::parse_version(version).is_err() {
            continue;
        }
        let downloads = entry.get("downloads")?.as_array()?;
        let win = downloads.iter().find_map(|d| {
            let d = d.as_object()?;
            (d.get("target")?.as_str() == Some("windows")
                && d.get("arch")?.as_str() == Some("x86_64")
                && d.get("edition")?.as_str() == Some("base"))
                .then_some(d)
        })?;
        let url = win.get("archive")?.get("url")?.as_str()?;
        let sha256 = win.get("archive")?.get("sha256")?.as_str()?;
        let parsed = version::parse_version(version).unwrap_or_default();
        if best.as_ref().is_none_or(|(p, _, _, _)| {
            version::compare(&parsed, p) == std::cmp::Ordering::Greater
        }) {
            best = Some((parsed, version.to_string(), url.to_string(), sha256.to_string()));
        }
    }
    best.map(|(_, version, url, sha256)| (version, url, sha256))
}

async fn resolve_mongodb() -> Result<ResolvedVersion, String> {
    let body = http_get("https://downloads.mongodb.org/current.json").await?;
    let (version, url, sha256) = parse_mongodb_current(&body)
        .ok_or_else(|| "downloads.mongodb.org: windows base-релиз не найден".to_string())?;
    Ok(ResolvedVersion {
        version,
        url: Some(url),
        sha256: Some(sha256),
    })
}

// --- PHP Windows: windows.php.net/downloads/releases/ ----------------

/// Парсит HTML-листинг каталога релизов windows.php.net: ищет
/// свежайший `php-<branch>.<patch>-nts-Win32-vs17-x64.zip`.
fn parse_php_windows_listing(body: &str, branch: &str) -> Option<(String, String)> {
    let escaped_branch = regex::escape(branch);
    let re = regex::Regex::new(&format!(
        r#"href="([^"]*php-{escaped_branch}\.(\d+)-nts-Win32-vs17-x64\.zip)""#
    ))
    .expect("корректный regex");
    let mut best: Option<(u64, String, String)> = None;
    for cap in re.captures_iter(body) {
        let href = cap.get(1)?.as_str().to_string();
        let patch: u64 = cap.get(2)?.as_str().parse().ok()?;
        if best.as_ref().is_none_or(|(p, _, _)| patch > *p) {
            best = Some((patch, href, format!("{branch}.{patch}")));
        }
    }
    best.map(|(_, href, version)| (version, href))
}

async fn resolve_php_windows(branch: &str) -> Result<ResolvedVersion, String> {
    let body = http_get("https://windows.php.net/downloads/releases/").await?;
    let Some((version, href)) = parse_php_windows_listing(&body, branch) else {
        return Err(format!("windows.php.net: билд ветки {branch} (nts vs17 x64) не найден"));
    };
    let url = if href.starts_with("http") {
        href
    } else {
        format!("https://windows.php.net/downloads/releases/{href}")
    };
    Ok(ResolvedVersion {
        version,
        url: Some(url),
        sha256: None,
    })
}

// --- Gradle: services.gradle.org/versions/current --------------------

async fn resolve_gradle() -> Result<ResolvedVersion, String> {
    let body = http_get("https://services.gradle.org/versions/current").await?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|e| format!("версия Gradle не разобралась: {e}"))?;
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "services.gradle.org: нет version".to_string())?
        .to_string();
    let url = json
        .get("downloadUrl")
        .and_then(|u| u.as_str())
        .map(str::to_string);
    // SHA-256 идёт отдельным файлом (.sha256) — догружаем, он маленький.
    let sha256 = match json.get("checksumUrl").and_then(|u| u.as_str()) {
        Some(checksum_url) => match http_get(checksum_url).await {
            Ok(text) => {
                let hash = text.split_whitespace().next().unwrap_or("").to_string();
                (!hash.is_empty()).then_some(hash)
            }
            Err(_) => None,
        },
        None => None,
    };
    Ok(ResolvedVersion {
        version,
        url,
        sha256,
    })
}

// --- Maven: repo.maven.apache.org maven-metadata.xml ------------------

/// Парсит maven-metadata.xml: выбирает максимальную СТАБИЛЬНУЮ версию
/// из `<version>` списка (maven может выставлять `<release>` на
/// пре-релиз, как было с 4.0.0-rc-6 — такие версии не берём).
fn parse_maven_metadata(body: &str) -> Option<String> {
    // Прямой <release>/<latest> — только если это стабильный номер.
    for tag in ["release", "latest"] {
        let re = regex::Regex::new(&format!(r#"<{tag}>\s*([^<]+?)\s*</{tag}>"#)).unwrap();
        if let Some(cap) = re.captures(body) {
            let version = cap.get(1)?.as_str().trim();
            if !version.is_empty()
                && !version.contains('-')
                && version::parse_version(version).is_ok()
            {
                return Some(version.to_string());
            }
        }
    }
    // Иначе — максимальный стабильный из полного списка версий.
    let re = regex::Regex::new(r#"<version>\s*([^<]+?)\s*</version>"#).unwrap();
    let versions: Vec<&str> = re
        .captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| m.as_str().trim()))
        .collect();
    latest_stable(versions.into_iter()).map(str::to_string)
}

async fn resolve_maven() -> Result<ResolvedVersion, String> {
    let body = http_get(
        "https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/maven-metadata.xml",
    )
    .await?;
    let Some(version) = parse_maven_metadata(&body) else {
        return Err("maven-metadata.xml: актуальная версия не найдена".to_string());
    };
    Ok(ResolvedVersion::version_only(&version))
}

// --- Kafka: downloads.apache.org/kafka/ -------------------------------

/// Парсит HTML-листинг каталога версий Kafka (`3.9.0/`, `4.0.0/`…):
/// возвращает максимальную стабильную версию.
fn parse_kafka_versions_listing(body: &str) -> Option<String> {
    let re = regex::Regex::new(r#"href="(\d+\.\d+\.\d+)/""#).unwrap();
    latest_stable(
        re.captures_iter(body)
            .filter_map(|c| c.get(1).map(|m| m.as_str())),
    )
    .map(str::to_string)
}

/// Парсит листинг каталога конкретной версии: ищет `kafka_2.13-<v>.tgz`
/// (scala-суффикс не зашит — берётся из фактического имени файла).
fn parse_kafka_files_listing(body: &str, version: &str) -> Option<String> {
    let escaped = regex::escape(version);
    let re = regex::Regex::new(&format!(r#"href="(kafka_[^"]*?-{escaped}\.tgz)""#)).unwrap();
    re.captures(body)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

async fn resolve_kafka() -> Result<ResolvedVersion, String> {
    let listing = http_get("https://downloads.apache.org/kafka/").await?;
    let Some(version) = parse_kafka_versions_listing(&listing) else {
        return Err("downloads.apache.org/kafka/: актуальная версия не найдена".to_string());
    };
    let dir_url = format!("https://downloads.apache.org/kafka/{version}/");
    let dir_listing = http_get(&dir_url).await?;
    let Some(file) = parse_kafka_files_listing(&dir_listing, &version) else {
        return Err(format!("каталог Kafka {version}: архив не найден"));
    };
    Ok(ResolvedVersion {
        version,
        url: Some(format!("{dir_url}{file}")),
        sha256: None,
    })
}

// --- Grafana: grafana.com API (листинг dl.grafana.com отдаёт 404) -----

/// Парсит ответ grafana.com/api/downloads/grafana/versions: первый
/// элемент с channels.stable == true (список отсортирован по убыванию
/// даты релиза). URL собирается шаблоном каталога.
fn parse_grafana_versions(body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    let items = json.get("items")?.as_array()?;
    for item in items {
        let Some(version) = item.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        let stable = item
            .get("channels")
            .and_then(|c| c.get("stable"))
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        if stable && version::parse_version(version).is_ok() {
            return Some(version.to_string());
        }
    }
    None
}

async fn resolve_grafana() -> Result<ResolvedVersion, String> {
    let body = http_get("https://grafana.com/api/downloads/grafana/versions").await?;
    let version = parse_grafana_versions(&body)
        .ok_or_else(|| "grafana.com: стабильная версия не найдена".to_string())?;
    Ok(ResolvedVersion::version_only(&version))
}

// --- winget: show --id ------------------------------------------------

/// Вытаскивает версию из локализованного вывода `winget show`:
/// ищем строку вида "Version: 1.2.3" (или "Версия: …") — winget
/// переводит заголовок, поэтому перечислены распространённые.
fn parse_winget_version(output: &str) -> Option<String> {
    for label in ["Version:", "Версия:", "版本:", "版本："] {
        if let Some(line) = output.lines().find(|l| l.contains(label)) {
            let after = line.split_once(label).map(|(_, v)| v).unwrap_or("");
            let version = after.trim().trim_matches(['"', '\'', '`']).to_string();
            if !version.is_empty() && version::parse_version(&version).is_ok() {
                return Some(version);
            }
        }
    }
    None
}

async fn resolve_winget(id: &str) -> Result<ResolvedVersion, String> {
    use tokio::process::Command as TokioCommand;
    use tokio::time::timeout;

    let mut cmd = TokioCommand::new("winget");
    cmd.args([
        "show",
        "--id",
        id,
        "--exact",
        "--accept-source-agreements",
        "--disable-interactivity",
    ]);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);

    let output = timeout(Duration::from_secs(90), cmd.output())
        .await
        .map_err(|_| format!("winget show {id}: таймаут"))?
        .map_err(|e| format!("winget show {id}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = parse_winget_version(&stdout)
        .ok_or_else(|| format!("winget show {id}: версия не распознана"))?;
    Ok(ResolvedVersion::version_only(&version))
}

// --- .NET: dotnetcli release-metadata ---------------------------------

/// Парсит releases-index.json: в массиве `releases-index` ищет канал
/// (channel-version) и берёт его latest-sdk.
fn parse_dotnet_index(body: &str, channel: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    let releases = json.get("releases-index")?.as_array()?;
    releases.iter().find_map(|entry| {
        let entry = entry.as_object()?;
        if entry.get("channel-version")?.as_str() != Some(channel) {
            return None;
        }
        entry.get("latest-sdk")?.as_str().map(str::to_string)
    })
}

async fn resolve_dotnet_channel(channel: &str) -> Result<ResolvedVersion, String> {
    let body = http_get("https://dotnetcli.blob.core.windows.net/dotnet/release-metadata/releases-index.json").await?;
    let version = parse_dotnet_index(&body, channel)
        .ok_or_else(|| format!("releases-index.json: канал {channel} не найден"))?;
    Ok(ResolvedVersion::version_only(&version))
}

// ------------------------------------------------------------
// Интеграция с установкой
// ------------------------------------------------------------

/// Эффективный URL источника: если задан version_resolver/url_template —
/// свежая версия апстрима подставляется в шаблон (или берётся готовый
/// URL резолвера). Статичный url остаётся страховкой при недоступности
/// апстрима.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSourceUrl {
    /// URL для скачивания (None — источника без URL нет вовсе).
    pub url: Option<String>,
    /// SHA-256 (из резолвера или каталога; None — unverified).
    pub sha256: Option<String>,
    /// Резолвнутая версия (None — источник без резолвера/недоступен).
    pub version: Option<String>,
}

pub async fn effective_source_url(
    resolver: Option<&VersionResolver>,
    url_template: Option<&str>,
    static_url: Option<&str>,
    static_sha256: Option<&str>,
) -> Result<EffectiveSourceUrl, String> {
    let static_fallback = || EffectiveSourceUrl {
        url: static_url.map(str::to_string),
        sha256: static_sha256.map(str::to_string),
        version: None,
    };

    // Без резолвера — чистая статика (обратная совместимость).
    let Some(resolver) = resolver else {
        return Ok(static_fallback());
    };

    // Установка: свежий резолв (обход кэша), кэш — страховка при сбое.
    let resolved = match resolve(resolver, true).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!(
                "[toolchain] резолвер {key} недоступен ({e}) — статичный URL",
                key = resolver.cache_key()
            );
            return Ok(static_fallback());
        }
    };

    let url = match &resolved.url {
        Some(u) => Some(u.clone()),
        None => match url_template {
            Some(template) => Some(fill_template(template, &resolved.version)?),
            None => static_url.map(str::to_string),
        },
    };
    Ok(EffectiveSourceUrl {
        url,
        sha256: resolved.sha256.or_else(|| static_sha256.map(str::to_string)),
        version: Some(resolved.version),
    })
}

// ------------------------------------------------------------
// Тесты (парсеры — оффлайн, без сети)
// ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_template_substitutes_version() {
        assert_eq!(
            fill_template(
                "https://releases.hashicorp.com/terraform/{version}/terraform_{version}_windows_amd64.zip",
                "1.16.0"
            )
            .unwrap(),
            "https://releases.hashicorp.com/terraform/1.16.0/terraform_1.16.0_windows_amd64.zip"
        );
    }

    #[test]
    fn fill_template_rejects_unfilled_placeholders() {
        assert!(fill_template("https://x/{version}/{arch}.zip", "1.0").is_err());
    }

    #[test]
    fn latest_stable_skips_prereleases_and_junk() {
        let versions = [
            "0.15.2",
            "0.16.0",
            "0.17.0-dev.2125",
            "0.16.0-rc2",
            "garbage",
        ];
        assert_eq!(latest_stable(versions.iter().copied()), Some("0.16.0"));
    }

    #[test]
    fn latest_stable_compares_numerically() {
        let versions = ["0.9.9", "0.10.0", "0.11.0"];
        assert_eq!(latest_stable(versions.iter().copied()), Some("0.11.0"));
    }

    #[test]
    fn ziglang_parser_picks_stable_windows_build() {
        let body = r#"{
            "master": {"x86_64-windows": {"tarball": "https://ziglang.org/builds/zig-master.zip", "shasum": "aa"}},
            "0.15.2": {"x86_64-windows": {"tarball": "https://ziglang.org/download/0.15.2/zig-x86_64-windows-0.15.2.zip", "shasum": "bb"}},
            "0.16.0": {"x86_64-windows": {"tarball": "https://ziglang.org/download/0.16.0/zig-x86_64-windows-0.16.0.zip", "shasum": "cc"}}
        }"#;
        let json: Value = serde_json::from_str(body).unwrap();
        let obj = json.as_object().unwrap();
        let version = latest_stable(obj.keys().map(String::as_str)).unwrap();
        assert_eq!(version, "0.16.0");
        let windows = obj[version]["x86_64-windows"].as_object().unwrap();
        assert_eq!(
            windows["tarball"].as_str().unwrap(),
            "https://ziglang.org/download/0.16.0/zig-x86_64-windows-0.16.0.zip"
        );
        assert_eq!(windows["shasum"].as_str().unwrap(), "cc");
    }

    #[test]
    fn hashicorp_parser_picks_first_stable_windows_build() {
        // Реальная форма index.json: {"name": ..., "versions": {v: {builds: [...]}}}.
        let body = r#"{
            "name": "terraform",
            "versions": {
                "1.16.0-rc1": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://rc-url", "filename": "rc.zip"}]},
                "1.16.0": {"builds": [
                    {"os": "linux", "arch": "amd64", "url": "https://linux-url", "filename": "l.zip"},
                    {"os": "windows", "arch": "amd64", "url": "https://win-url", "filename": "w.zip"}
                ]},
                "1.15.9": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://old-url", "filename": "old.zip"}]}
            }
        }"#;
        let (version, url, filename) = parse_hashicorp_index(body, "terraform").unwrap();
        assert_eq!(version, "1.16.0");
        assert_eq!(url, "https://win-url");
        assert_eq!(filename, "w.zip");
    }

    #[test]
    fn hashicorp_candidates_are_sorted_descending() {
        // Индекс не отсортирован (порядок вставки 0.1.0 → 1.16.2):
        // кандидаты обязаны идти по убыванию числовой версии — резолвер
        // выбирает первого «живого», т.е. САМЫЙ свежий доступный релиз.
        let body = r#"{
            "versions": {
                "1.9.9": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://x/1.9.9", "filename": "a.zip"}]},
                "1.16.2": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://x/1.16.2", "filename": "b.zip"}]},
                "1.10.1": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://x/1.10.1", "filename": "c.zip"}]},
                "1.16.1": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://x/1.16.1", "filename": "d.zip"}]}
            }
        }"#;
        let candidates = parse_hashicorp_candidates(body);
        let versions: Vec<&str> = candidates.iter().map(|(v, _, _)| v.as_str()).collect();
        assert_eq!(versions, vec!["1.16.2", "1.16.1", "1.10.1", "1.9.9"]);
    }

    #[test]
    fn hashicorp_candidates_skip_prereleases_and_non_windows() {
        let body = r#"{
            "versions": {
                "1.17.0-rc1": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://rc", "filename": "rc.zip"}]},
                "1.16.3": {"builds": [{"os": "linux", "arch": "amd64", "url": "https://linux", "filename": "l.zip"}]},
                "1.16.2": {"builds": [{"os": "windows", "arch": "amd64", "url": "https://win", "filename": "w.zip"}]}
            }
        }"#;
        let candidates = parse_hashicorp_candidates(body);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "1.16.2");
    }

    #[test]
    fn hashicorp_sha256sums_line_parses() {
        let sums = "abc123  terraform_1.16.0_windows_amd64.zip\nother  x.zip";
        let line = sums
            .lines()
            .find(|l| l.trim_end().ends_with("terraform_1.16.0_windows_amd64.zip"))
            .unwrap();
        assert_eq!(
            line.split_whitespace().next(),
            Some("abc123"),
            "хэш берётся из строки SHA256SUMS"
        );
    }

    #[test]
    fn github_version_normalizes_any_prefix() {
        assert_eq!(github_version_from_tag("v2.4.20"), "2.4.20");
        assert_eq!(github_version_from_tag("OTP-29.0.5"), "29.0.5");
        assert_eq!(github_version_from_tag("8.10.1"), "8.10.1");
        assert_eq!(github_version_from_tag("release-1.2.3"), "1.2.3");
    }

    #[test]
    fn github_asset_matches_pattern_or_exact_name() {
        let body = r#"{
            "tag_name": "OTP-29.0.6",
            "assets": [
                {"name": "otp_win64_29.0.5.exe", "browser_download_url": "old"},
                {"name": "otp_win64_29.0.6.exe", "browser_download_url": "new"},
                {"name": "elixir-otp-29.zip", "browser_download_url": "elixir-zip"}
            ]
        }"#;
        let json: Value = serde_json::from_str(body).unwrap();
        let version = github_version_from_tag(json["tag_name"].as_str().unwrap());
        assert_eq!(version, "29.0.6");
        // Паттерн с {version} собирается из нормализованной версии тега.
        assert_eq!(
            github_asset_url(&json, "otp_win64_{version}.exe", &version).as_deref(),
            Some("new")
        );
        // Паттерн без {version} — точное имя (elixir-ассет без версии в имени).
        assert_eq!(
            github_asset_url(&json, "elixir-otp-29.zip", &version).as_deref(),
            Some("elixir-zip")
        );
        assert_eq!(github_asset_url(&json, "no-such-asset.zip", &version), None);
    }

    #[test]
    fn swift_releases_picks_latest_stable() {
        let body = r#"[
            {"name": "3.0", "tag": "swift-3.0-RELEASE"},
            {"name": "5.10.1", "tag": "swift-5.10.1-RELEASE"},
            {"name": "6.3.3", "tag": "swift-6.3.3-RELEASE"},
            {"name": "6.3.3-DEVELOPMENT-SNAPSHOT", "tag": "swift-6.3.3-DEVELOPMENT-SNAPSHOT-2026-09-01-a"}
        ]"#;
        assert_eq!(parse_swift_releases(body).as_deref(), Some("6.3.3"));
    }

    #[test]
    fn mongodb_current_picks_newest_production_windows_build() {
        let body = r#"{"versions": [
            {"version": "8.2.0", "production_release": true, "release_candidate": false, "downloads": [
                {"target": "windows", "arch": "x86_64", "edition": "base", "archive": {"url": "https://fastdl.mongodb.org/windows/mongodb-windows-x86_64-8.2.0.zip", "sha256": "a1"}}
            ]},
            {"version": "8.3.0-rc1", "production_release": false, "release_candidate": true, "downloads": []},
            {"version": "8.3.11", "production_release": true, "release_candidate": false, "downloads": [
                {"target": "linux", "arch": "x86_64", "edition": "base", "archive": {"url": "linux", "sha256": "x"}},
                {"target": "windows", "arch": "x86_64", "edition": "base", "archive": {"url": "https://fastdl.mongodb.org/windows/mongodb-windows-x86_64-8.3.11.zip", "sha256": "b2"}}
            ]}
        ]}"#;
        let (version, url, sha256) = parse_mongodb_current(body).unwrap();
        assert_eq!(version, "8.3.11");
        assert!(url.contains("8.3.11"));
        assert_eq!(sha256, "b2");
    }

    #[test]
    fn github_asset_matches_pattern() {
        let body = r#"{
            "tag_name": "v2.2.0",
            "assets": [
                {"name": "kotlin-compiler-2.1.20.zip", "browser_download_url": "old"},
                {"name": "kotlin-compiler-2.2.0.zip", "browser_download_url": "new"}
            ]
        }"#;
        let json: Value = serde_json::from_str(body).unwrap();
        let tag = json["tag_name"].as_str().unwrap();
        let version = tag.strip_prefix('v').unwrap();
        assert_eq!(version, "2.2.0");
        let asset_name = "kotlin-compiler-{version}.zip".replace("{version}", version);
        let url = json["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|a| {
                let a = a.as_object().unwrap();
                (a["name"] == asset_name).then(|| a["browser_download_url"].as_str().unwrap())
            })
            .unwrap();
        assert_eq!(url, "new");
    }

    #[test]
    fn php_listing_finds_latest_branch_build() {
        let body = r#"
<a href="https://windows.php.net/downloads/releases/php-8.4.23-nts-Win32-vs17-x64.zip">php-8.4.23</a>
<a href="https://windows.php.net/downloads/releases/php-8.4.25-nts-Win32-vs17-x64.zip">php-8.4.25</a>
<a href="https://windows.php.net/downloads/releases/php-8.5.0-nts-Win32-vs17-x64.zip">php-8.5.0</a>
"#;
        let (version, href) = parse_php_windows_listing(body, "8.4").unwrap();
        assert_eq!(version, "8.4.25");
        assert!(href.contains("8.4.25"));
        // Другая ветка не смешивается.
        let (version, _) = parse_php_windows_listing(body, "8.5").unwrap();
        assert_eq!(version, "8.5.0");
    }

    #[test]
    fn gradle_current_parses() {
        let body = r#"{"version": "9.2.0", "downloadUrl": "https://services.gradle.org/distributions/gradle-9.2.0-bin.zip", "checksumUrl": "https://services.gradle.org/distributions/gradle-9.2.0-bin.zip.sha256"}"#;
        let json: Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["version"], "9.2.0");
        assert!(json["downloadUrl"].as_str().unwrap().contains("9.2.0"));
    }

    #[test]
    fn maven_metadata_prefers_stable_release() {
        // Реальная ситуация: maven выставил <release> на пре-релиз 4.0.0-rc-6 —
        // стабильная версия берётся из списка версий.
        let body = r#"<metadata><versioning><release>4.0.0-rc-6</release><latest>4.0.0-rc-6</latest><versions><version>3.9.9</version><version>3.9.11</version><version>4.0.0-rc-6</version></versions></versioning></metadata>"#;
        assert_eq!(parse_maven_metadata(body).as_deref(), Some("3.9.11"));
    }

    #[test]
    fn maven_metadata_falls_back_to_versions_list() {
        let body = r#"<metadata><versioning><latest>3.9.12</latest><versions><version>3.9.10</version><version>3.9.12</version></versions></versioning></metadata>"#;
        assert_eq!(parse_maven_metadata(body).as_deref(), Some("3.9.12"));
    }

    #[test]
    fn kafka_listing_parses_latest_and_scala_suffix() {
        let versions = r#"
<a href="3.8.1/">3.8.1/</a>
<a href="3.9.0/">3.9.0/</a>
<a href="4.0.0/">4.0.0/</a>
"#;
        assert_eq!(parse_kafka_versions_listing(versions).as_deref(), Some("4.0.0"));
        let files = r#"<a href="kafka_2.13-4.0.0.tgz">kafka_2.13-4.0.0.tgz</a>"#;
        assert_eq!(
            parse_kafka_files_listing(files, "4.0.0").as_deref(),
            Some("kafka_2.13-4.0.0.tgz")
        );
    }

    #[test]
    fn grafana_api_parses_first_stable() {
        let body = r#"{"items": [
            {"version": "13.3.0-34546732426", "channels": {"stable": false}},
            {"version": "13.2.1", "channels": {"stable": true}},
            {"version": "13.2.0", "channels": {"stable": true}}
        ]}"#;
        assert_eq!(parse_grafana_versions(body).as_deref(), Some("13.2.1"));
    }

    #[test]
    fn winget_version_parses_localized_labels() {
        let en = "Found PostgreSQL.PostgreSQL.17 [PostgreSQL.PostgreSQL.17] Version 17.5\nVersion: 17.5.0\nPublisher: PostgreSQL";
        assert_eq!(parse_winget_version(en).as_deref(), Some("17.5.0"));
        let ru = "Найден PostgreSQL.PostgreSQL.17 [PostgreSQL.PostgreSQL.17] Версия 17.5\nВерсия: 17.5.0";
        assert_eq!(parse_winget_version(ru).as_deref(), Some("17.5.0"));
        assert_eq!(parse_winget_version("no version here"), None);
    }

    #[test]
    fn dotnet_index_parses_channel_sdk() {
        let body = r#"{"releases-index": [
            {"channel-version": "8.0", "latest-release": "8.0.31", "latest-sdk": "8.0.425"},
            {"channel-version": "10.0", "latest-release": "10.0.12", "latest-sdk": "10.0.401"}
        ]}"#;
        assert_eq!(
            parse_dotnet_index(body, "10.0").as_deref(),
            Some("10.0.401")
        );
        assert_eq!(parse_dotnet_index(body, "9.0"), None);
    }

    #[test]
    fn resolver_cache_key_is_stable_and_distinct() {
        let a = VersionResolver::Hashicorp {
            product: "terraform".into(),
        };
        let b = VersionResolver::Hashicorp {
            product: "vault".into(),
        };
        assert_ne!(a.cache_key(), b.cache_key());
        assert_eq!(
            a.cache_key(),
            VersionResolver::Hashicorp {
                product: "terraform".into()
            }
            .cache_key()
        );
    }

    #[tokio::test]
    async fn effective_recommended_falls_back_to_static_without_resolver() {
        let def = ToolDefinition {
            id: "no-resolver".into(),
            category: "utility".into(),
            display: "No Resolver".into(),
            description: String::new(),
            icon: None,
            detection: Default::default(),
            versions: crate::modules::toolchain::models::VersionRules {
                min: None,
                recommended: Some("1.2".into()),
                resolver: None,
            },
            sources: Default::default(),
            size_mb: 0,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        };
        assert_eq!(effective_recommended(&def).await.as_deref(), Some("1.2"));
    }

    #[test]
    fn fill_template_is_cross_platform() {
        // Никакой Windows-специфики в шаблонах — только https URL.
        let t = fill_template("https://x/{version}/f.zip", "9.9.9").unwrap();
        assert_eq!(t, "https://x/9.9.9/f.zip");
    }

    // Для покрытия путей без сети: резолверы должны ЧЕСТНО падать,
    // а не паниковать — статичная страховка вызывающего сработает.
    #[tokio::test]
    async fn curl_missing_is_an_error_not_panic() {
        // curl на машинах разработки есть; проверяем только форму ошибки.
        if curl_available() {
            let err = resolve(
                &VersionResolver::Hashicorp {
                    product: "definitely-no-such-product-xyz".into(),
                },
                true,
            )
            .await;
            assert!(err.is_err(), "несуществующий продукт обязан дать Err");
        }
    }

    /// Оффлайн-проверка каркаса: cURL не нужен, если резолвер падает
    /// раньше сети (этот тест не зависит от сети).
    #[test]
    fn static_path_of_effective_source_url_works_offline() {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(effective_source_url(
                None,
                None,
                Some("https://static.example.com/x.zip"),
                Some("abc"),
            ))
            .unwrap();
        assert_eq!(result.url.as_deref(), Some("https://static.example.com/x.zip"));
        assert_eq!(result.sha256.as_deref(), Some("abc"));
        assert_eq!(result.version, None);
    }

    /// Живая проверка резолверов по РЕАЛЬНЫМ апстримам. По умолчанию
    /// игнорируется (сеть), запуск для проверки каталога:
    ///   cargo test --lib live_resolvers -- --ignored --nocapture
    /// Каждый резолвер обязан вернуть непустую версию и (где применимо)
    /// https-URL — это контракт против «протухания» каталога.
    #[tokio::test]
    #[ignore = "сетевой тест: проверяет реальные апстримы"]
    async fn live_resolvers_return_versions() {
        let cases: Vec<(&str, VersionResolver, bool)> = vec![
            ("ziglang", VersionResolver::Ziglang, true),
            (
                "hashicorp/terraform",
                VersionResolver::Hashicorp {
                    product: "terraform".into(),
                },
                true,
            ),
            (
                "github/kotlin",
                VersionResolver::Github {
                    repo: "JetBrains/kotlin".into(),
                    asset: "kotlin-compiler-{version}.zip".into(),
                },
                true,
            ),
            (
                "php_windows/8.4",
                VersionResolver::PhpWindows {
                    branch: "8.4".into(),
                },
                true,
            ),
            ("gradle", VersionResolver::Gradle, true),
            ("maven", VersionResolver::MavenApache, false),
            ("kafka", VersionResolver::ApacheKafka, true),
            // URL grafana собирается шаблоном каталога (API версий
            // не отдаёт ссылок на сборки).
            ("grafana", VersionResolver::Grafana, false),
            // URL swift собирается шаблоном каталога.
            ("swift", VersionResolver::Swift, false),
            // MongoDB отдаёт и URL, и SHA-256 прямо в current.json.
            ("mongodb", VersionResolver::MongoDb, true),
            (
                "github/erlang",
                VersionResolver::Github {
                    repo: "erlang/otp".into(),
                    asset: "otp_win64_{version}.exe".into(),
                },
                true,
            ),
            (
                "github/elixir",
                VersionResolver::Github {
                    repo: "elixir-lang/elixir".into(),
                    asset: "elixir-otp-29.zip".into(),
                },
                true,
            ),
            (
                "github/gleam",
                VersionResolver::Github {
                    repo: "gleam-lang/gleam".into(),
                    asset: "gleam-v{version}-x86_64-pc-windows-msvc.zip".into(),
                },
                true,
            ),
            (
                "github/cmake",
                VersionResolver::Github {
                    repo: "Kitware/CMake".into(),
                    asset: "cmake-{version}-windows-x86_64.msi".into(),
                },
                true,
            ),
            (
                "github/redis",
                VersionResolver::Github {
                    repo: "redis-windows/redis-windows".into(),
                    asset: "Redis-{version}-Windows-x64-cygwin.zip".into(),
                },
                true,
            ),
            (
                "dotnet/10.0",
                VersionResolver::DotnetChannel {
                    channel: "10.0".into(),
                },
                false,
            ),
        ];
        for (label, resolver, needs_url) in cases {
            let resolved = resolve(&resolver, true)
                .await
                .unwrap_or_else(|e| panic!("{label}: резолвер упал: {e}"));
            assert!(
                !resolved.version.is_empty(),
                "{label}: пустая версия"
            );
            assert!(
                crate::modules::toolchain::core::version::parse_version(&resolved.version).is_ok(),
                "{label}: версия не разбирается: {}",
                resolved.version
            );
            if needs_url {
                let url = resolved
                    .url
                    .as_deref()
                    .unwrap_or_else(|| panic!("{label}: нет URL"));
                assert!(url.starts_with("https://"), "{label}: URL не https: {url}");
            }
            eprintln!(
                "live resolver {label}: version={} url={:?} sha256={:?}",
                resolved.version, resolved.url, resolved.sha256
            );
        }
    }

    /// Живая проверка winget-резолвера (Windows; требует установленный
    /// winget и доступ к источнику). Отдельно от остальных: долгий.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore = "сетевой тест: winget show"]
    async fn live_winget_resolver_returns_version() {
        let resolved = resolve(
            &VersionResolver::Winget {
                id: "PostgreSQL.PostgreSQL.17".into(),
            },
            true,
        )
        .await
        .expect("winget show обязан вернуть версию");
        assert!(!resolved.version.is_empty());
        eprintln!("live resolver winget: version={}", resolved.version);
    }
}