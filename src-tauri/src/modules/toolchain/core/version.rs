use std::cmp::{self, Ordering};

pub fn parse_version(raw: &str) -> Result<Vec<u32>, String> {
    let clean_numbers: Result<Vec<u32>, String> = raw
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>())
        .collect::<Result<Vec<u32>, _>>()
        .map_err(|err| format!("Error in parse version: {err}"));

    match clean_numbers {
        Ok(vec) => {
            if vec.is_empty() {
                Err("Empty version".into())
            } else {
                Ok(vec)
            }
        },
        Err(e) => Err(e)
    }
}

pub fn compare(a: &[u32], b: &[u32]) -> Ordering {
    let max_len = cmp::max(a.len(), b.len());
    
    let a_iter = a.iter().copied().chain(std::iter::repeat(0)).take(max_len);
    let b_iter = b.iter().copied().chain(std::iter::repeat(0)).take(max_len);

    a_iter.cmp(b_iter)
}

/// true, если установленная версия не ниже минимальной.
/// На этапе 2 min носит advisory-характер и проверку не блокирует;
/// задействуется на этапе 6 (health-отчёты). Пока не зовётся — отсюда
/// allow(dead_code), чтобы не мусорить предупреждениями.
#[allow(dead_code)]
pub fn meets_min(installed: &[u32], min: &[u32]) -> bool {
    compare(installed, min) != Ordering::Less
}

// ============================================================
// Тесты
// ============================================================
// Тесты пишутся в том же файле, что и код, в специальном модуле
// #[cfg(test)] — он компилируется только при запуске `cargo test`
// и никогда не попадает в релизный бинарь. `use super::*` импортирует
// всё из родительского модуля (parse_version, compare, meets_min).
// Запуск: cargo test  (или точечно: cargo test parse_simple_version)

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn parse_simple_version() {
        assert_eq!(parse_version("1.2.3"), Ok(vec![1, 2, 3]));
    }

    #[test]
    fn parse_with_v_prefix() {
        assert_eq!(parse_version("v22.12.0"), Ok(vec![22, 12, 0]));
    }

    #[test]
    fn parse_version_with_suffix() {
        // Суффикс -rc2 содержит цифру, поэтому попадает в парс.
        // Для сравнения с min/recommended это безопасно: 3.13.1-rc2 >= 3.13.1.
        assert_eq!(parse_version("3.13.1-rc2"), Ok(vec![3, 13, 1, 2]));
    }

    #[test]
    fn parse_version_embedded_in_text() {
        // «node v22.12.0» — типичный вывод `node --version`
        assert_eq!(parse_version("node v22.12.0"), Ok(vec![22, 12, 0]));
    }

    #[test]
    fn parse_garbage_is_err() {
        assert!(parse_version("abc").is_err());
        assert!(parse_version("").is_err());
        assert!(parse_version("   ").is_err());
    }

    #[test]
    fn compare_one_point_nine_vs_one_point_ten() {
        // Классическая ловушка: "1.10" < "1.9" как строки, но не как версии
        assert_eq!(compare(&[1, 9], &[1, 10]), Ordering::Less);
        assert_eq!(compare(&[1, 10], &[1, 9]), Ordering::Greater);
    }

    #[test]
    fn compare_ignores_trailing_zeros() {
        // 2.40.0 == 2.40 — недостающие части считаются нулями
        assert_eq!(compare(&[2, 40, 0], &[2, 40]), Ordering::Equal);
    }

    #[test]
    fn meets_min_checks_boundaries() {
        assert!(meets_min(&[2, 40, 0], &[2, 40]));
        assert!(meets_min(&[2, 40], &[2, 40]));
        assert!(!meets_min(&[2, 39, 9], &[2, 40]));
    }
}
