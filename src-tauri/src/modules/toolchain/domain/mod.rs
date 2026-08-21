// ============================================================
// Domain — движок чтения и диагностики окружения
// ============================================================
// Новый read-only слой Toolchain (контракт docs/toolchain-contract.md):
//
//   models      — размерности, презентационные состояния, job/события;
//   detect      — живое обнаружение одного инструмента + здоровье;
//   path_report — диагностика PATH (только чтение);
//   score       — документированная формула оценки окружения;
//   cache       — честный кэш снапшотов (stale-пометка);
//   engine      — жизненный цикл заданий скана (job/reconnect/cancel).
//
// Правила слоя:
//   - скан НИКОГДА не мутирует машину (установка/PATH/файлы проекта
//     запрещены; только пробы и health-объявления каталога);
//   - scan-failed ≠ missing: ошибка опроса не выдаётся за отсутствие;
//   - события без идентичности операции не эмитятся;
//   - кэш честно помечается устаревшим.

pub mod cache;
pub mod detect;
pub mod engine;
pub mod models;
pub mod path_report;
pub mod probe;
pub mod profile;
pub mod score;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Ограниченно-параллельное отображение с детерминированным порядком
/// результата: `f(item_i)` кладётся в результат под индексом i,
/// независимо от порядка завершения. Используется движком скана для
/// батчей инструментов (health по выбранным тулам и т.п.).
///
/// Сам движок скана использует Semaphore напрямую (слоты + события);
/// эта обёртка — для простых батчей и для тестов границы параллелизма.
pub async fn bounded_map<T, R, F, Fut>(items: Vec<T>, limit: usize, f: F) -> Vec<R>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static + Clone,
    Fut: std::future::Future<Output = R> + Send + 'static,
{
    let semaphore = Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let slots: Arc<std::sync::Mutex<Vec<Option<R>>>> = Arc::new(std::sync::Mutex::new(
        (0..items.len()).map(|_| None).collect(),
    ));

    let mut set = tokio::task::JoinSet::new();
    for (index, item) in items.into_iter().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore живёт до конца функции");
        let slots = Arc::clone(&slots);
        let f = f.clone();
        set.spawn(async move {
            let value = f(item).await;
            slots.lock().expect("slots poisoned")[index] = Some(value);
            drop(permit);
        });
    }
    while set.join_next().await.is_some() {}

    let mut guard = slots.lock().expect("slots poisoned");
    guard
        .iter_mut()
        .map(|slot| slot.take().expect("слот обязан быть заполнен"))
        .collect()
}

/// Счётчик одновременных задач (для тестов границы параллелизма).
#[cfg(test)]
#[derive(Default)]
pub struct ConcurrencyGauge {
    current: AtomicUsize,
    max_seen: AtomicUsize,
}

#[cfg(test)]
impl ConcurrencyGauge {
    pub fn enter(&self) -> usize {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_seen.fetch_max(now, Ordering::SeqCst);
        now
    }

    pub fn exit(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn max_seen(&self) -> usize {
        self.max_seen.load(Ordering::SeqCst)
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Граница параллелизма: при лимите N максимум одновременных задач
    /// не превышает N, даже если задач сильно больше.
    #[tokio::test]
    async fn bounded_map_respects_concurrency_limit() {
        for limit in [1usize, 3] {
            let gauge = Arc::new(ConcurrencyGauge::default());
            let items: Vec<usize> = (0..12).collect();
            let g = Arc::clone(&gauge);
            let results = bounded_map(items, limit, move |_i| {
                let g = Arc::clone(&g);
                async move {
                    g.enter();
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    g.exit();
                    _i * 2
                }
            })
            .await;

            assert_eq!(results.len(), 12);
            // Детерминированный порядок результата.
            assert_eq!(results[0], 0);
            assert_eq!(results[7], 14);
            assert_eq!(results[11], 22);
            assert!(
                gauge.max_seen() <= limit,
                "параллелизм {} превысил лимит {}",
                gauge.max_seen(),
                limit
            );
        }
    }

    /// Порядок результатов детерминирован даже при разном времени задач.
    #[tokio::test]
    async fn bounded_map_ordering_is_deterministic() {
        let items = vec![10usize, 1, 100];
        let results = bounded_map(items, 4, |delay_ms| async move {
            tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
            delay_ms
        })
        .await;
        assert_eq!(results, vec![10, 1, 100]);
    }
}
