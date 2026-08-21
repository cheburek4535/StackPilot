// ============================================================
// Кэш снапшотов (domain/cache.rs)
// ============================================================
// Правило честного кэша: последний валидный снапшот отдаётся
// НЕМЕДЛЕННО, но с явной пометкой stale, если он старше порога
// свежести. Кэш никогда не притворяется живыми данными.
//
// Персистентность: toolchain/scan-snapshot.json рядом со state.json
// (атомарная запись tmp+rename). Битый/отсутствующий файл — просто
// «кэша нет».

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::models::EnvironmentSnapshot;

/// Порог свежести по умолчанию: 5 минут.
pub const DEFAULT_FRESHNESS: Duration = Duration::from_secs(5 * 60);

const SNAPSHOT_FILE: &str = "scan-snapshot.json";

/// RFC3339-метка текущего момента (тот же формат, что console::timestamp).
fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

/// Разбирает RFC3339-метку в SystemTime (для расчёта возраста).
fn parse_rfc3339(ts: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.into())
}

/// Возраст метки в секундах относительно текущего момента.
pub fn age_seconds_of(timestamp: &str) -> u64 {
    let Some(then) = parse_rfc3339(timestamp) else {
        return u64::MAX;
    };
    SystemTime::now()
        .duration_since(then)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Кэш последнего снапшота. Потокобезопасен (внутренний Mutex).
pub struct SnapshotCache {
    path: PathBuf,
    freshness: Duration,
    inner: std::sync::Mutex<Option<EnvironmentSnapshot>>,
}

impl SnapshotCache {
    /// `dir` — каталог данных toolchain (там же, где state.json).
    pub fn new(dir: &Path) -> Self {
        Self::with_freshness(dir, DEFAULT_FRESHNESS)
    }

    pub fn with_freshness(dir: &Path, freshness: Duration) -> Self {
        Self {
            path: dir.join(SNAPSHOT_FILE),
            freshness,
            inner: std::sync::Mutex::new(None),
        }
    }

    /// Последний валидный снапшот (память → диск) с пересчитанными
    /// age_seconds/stale/from_cache. None — кэша нет вообще.
    pub fn get(&self) -> Option<EnvironmentSnapshot> {
        // Память приоритетна: она всегда не старше диска.
        let mut cached = self.inner.lock().expect("snapshot cache poisoned").clone();
        if cached.is_none() {
            cached = self.load_from_disk();
            if let Some(snap) = &cached {
                *self.inner.lock().expect("snapshot cache poisoned") = Some(snap.clone());
            }
        }

        cached.map(|mut snap| {
            snap.age_seconds = age_seconds_of(&snap.finished_at);
            snap.stale = snap.age_seconds > self.freshness.as_secs();
            snap.from_cache = true;
            snap
        })
    }

    /// Сохраняет снапшот (в память и на диск атомарно).
    pub fn store(&self, snapshot: &EnvironmentSnapshot) -> Result<(), String> {
        *self.inner.lock().expect("snapshot cache poisoned") = Some(snapshot.clone());

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Не удалось создать {}: {e}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(snapshot)
            .map_err(|e| format!("Снапшот не сериализуется: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, raw)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Не удалось сохранить {}: {e}", self.path.display()))?;
        Ok(())
    }

    fn load_from_disk(&self) -> Option<EnvironmentSnapshot> {
        let raw = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

/// Проставляет снапшоту момент завершения «сейчас» (при записи в кэш).
pub fn stamp_finished(snapshot: &mut EnvironmentSnapshot) {
    snapshot.finished_at = now_rfc3339();
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::domain::models::{
        AdminCapability, DetectionOutcome, PathReport, PlatformApplicability, Provenance,
        ScoreSummary, StatusCounts, ToolScanResult, ToolState, VersionAssessment,
    };
    use std::time::UNIX_EPOCH;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tc-cache-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_snapshot(finished_at: String) -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            snapshot_id: "snap-1".to_string(),
            job_id: "job-1".to_string(),
            scan_id: "scan-1".to_string(),
            os: "windows".to_string(),
            os_version: "test".to_string(),
            arch: "x86_64".to_string(),
            package_managers: vec![],
            disk: vec![],
            admin: AdminCapability::default(),
            started_at: finished_at.clone(),
            finished_at,
            complete: true,
            cancelled: false,
            tools: vec![ToolScanResult {
                tool_id: "node".to_string(),
                display: "Node.js".to_string(),
                category: "language".to_string(),
                icon: None,
                detection: DetectionOutcome::NotDetected,
                installs: vec![],
                path_findings: vec![],
                health: None,
                applicability: PlatformApplicability::Installable,
                capabilities: Default::default(),
                provenance: Provenance::Unknown,
                bundled_with: None,
                version_assessment: VersionAssessment::Unknown,
                state: ToolState::Missing,
                error: None,
                duration_ms: 1,
            }],
            path_report: PathReport::default(),
            score: ScoreSummary::default(),
            summary: StatusCounts::from_results(&[]),
            warnings: vec![],
            errors: vec![],
            active_jobs: vec![],
            age_seconds: 0,
            stale: false,
            from_cache: false,
        }
    }

    #[test]
    fn empty_cache_returns_none() {
        let cache = SnapshotCache::new(&temp_dir("empty"));
        assert!(cache.get().is_none());
    }

    #[test]
    fn fresh_snapshot_is_not_stale() {
        let dir = temp_dir("fresh");
        let cache = SnapshotCache::new(&dir);
        let snap = sample_snapshot(now_rfc3339());
        cache.store(&snap).unwrap();

        let got = cache.get().unwrap();
        assert!(!got.stale, "свежий снапшот не помечается устаревшим");
        assert!(got.from_cache);
        assert_eq!(got.snapshot_id, "snap-1");
    }

    #[test]
    fn old_snapshot_marked_stale_not_hidden() {
        let dir = temp_dir("stale");
        // Кэш с порогом свежести 60 секунд; снапшоту час.
        let cache = SnapshotCache::with_freshness(&dir, Duration::from_secs(60));
        let hour_ago = chrono::Local::now() - chrono::Duration::hours(1);
        let snap = sample_snapshot(hour_ago.to_rfc3339());
        cache.store(&snap).unwrap();

        let got = cache.get().expect("старый снапшот обязан ОТДАВАТЬСЯ");
        assert!(got.stale, "устаревший кэш помечается stale");
        assert!(got.age_seconds >= 3600);
        assert_eq!(
            got.tools.len(),
            1,
            "данные не выбрасываются — только пометка"
        );
    }

    #[test]
    fn snapshot_survives_reload_from_disk() {
        let dir = temp_dir("reload");
        let snap = sample_snapshot(now_rfc3339());
        {
            let cache = SnapshotCache::new(&dir);
            cache.store(&snap).unwrap();
        }
        // «Новый запуск приложения»: память пуста, читаем с диска.
        let cache2 = SnapshotCache::new(&dir);
        let got = cache2.get().unwrap();
        assert_eq!(got.job_id, "job-1");
        assert!(got.from_cache);
    }

    #[test]
    fn broken_cache_file_tolerated() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join(SNAPSHOT_FILE), "{ not json").unwrap();
        let cache = SnapshotCache::new(&dir);
        assert!(cache.get().is_none(), "битый кэш = «кэша нет», не ошибка");
    }

    #[test]
    fn age_parse_garbage_is_max() {
        assert_eq!(age_seconds_of("not-a-date"), u64::MAX);
    }
}
