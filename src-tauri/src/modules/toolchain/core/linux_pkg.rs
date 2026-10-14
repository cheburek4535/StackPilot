// ============================================================
// Linux: сборка команд менеджера пакетов (в т.ч. Arch/pacman)
// ============================================================
// В tools.json linux-источники записаны именами Debian/Ubuntu-пакетов
// (docker.io, nodejs, openjdk-21-jdk...). На Arch они называются иначе,
// а часть пакетов живёт только в AUR. Здесь — перевод имён, выбор
// AUR-помощника и пост-настройка служб (docker, БД), без которой на Arch
// «установлено» ещё не значит «работает». Повышение прав при установке
// делает общий console::run_elevated (см. installer::source_requires_elevation);
// здесь with_privileges нужен только синхронному удалению.

/// Как поставить пакет на Arch.
#[derive(Debug, PartialEq, Eq)]
pub enum ArchPkg {
    /// Пакеты официальных репозиториев (pacman).
    Repo(&'static [&'static str]),
    /// Только AUR (нужен yay/paru).
    Aur(&'static [&'static str]),
}

/// Имя пакета каталога (Debian-стиль) → пакеты Arch.
/// None — перевода нет, id используется как есть.
pub fn arch_package(id: &str) -> Option<ArchPkg> {
    use ArchPkg::{Aur, Repo};
    Some(match id {
        "nodejs" => Repo(&["nodejs", "npm"]),
        "python3" => Repo(&["python", "python-pip"]),
        "golang" => Repo(&["go"]),
        "openjdk-21-jdk" => Repo(&["jdk21-openjdk"]),
        "dotnet-sdk-10.0" => Repo(&["dotnet-sdk"]),
        "redis-server" => Repo(&["redis"]),
        "mysql-server" => Repo(&["mariadb"]),
        "docker.io" => Repo(&["docker", "docker-compose"]),
        "code" => Repo(&["code"]),
        "postgresql" => Repo(&["postgresql"]),
        "kafka" => Repo(&["kafka"]),
        "terraform" => Repo(&["terraform"]),
        "dart" => Repo(&["dart"]),
        "mongodb-org" => Aur(&["mongodb-bin"]),
        "swiftlang" => Aur(&["swift-bin"]),
        "flutter" => Aur(&["flutter-bin"]),
        _ => return None,
    })
}

/// AUR-помощник, если установлен.
pub fn aur_helper() -> Option<&'static str> {
    ["paru", "yay"]
        .into_iter()
        .find(|h| which::which(h).is_ok())
}

/// Запущены ли мы от root (повышение не нужно).
#[cfg(unix)]
pub fn is_root() -> bool {
    // SAFETY: geteuid не имеет побочных эффектов.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}

/// Оборачивает команду в pkexec (графический polkit-запрос пароля),
/// если процесс не root. Используется удалением; установка повышает
/// права через console::run_elevated. Без pkexec — понятная ошибка вместо
/// невидимого sudo, который завис бы в ожидании пароля на TTY.
pub fn with_privileges(program: &str, args: Vec<String>) -> Result<(String, Vec<String>), String> {
    if is_root() {
        return Ok((program.to_string(), args));
    }
    if which::which("pkexec").is_err() {
        return Err(
            "Для установки нужны права администратора, но pkexec (polkit) не найден. \
             Установите polkit или запустите приложение от root."
                .to_string(),
        );
    }
    let mut full = vec![program.to_string()];
    full.extend(args);
    Ok(("pkexec".to_string(), full))
}

/// Пост-настройка службы после установки пакета Arch (выполняется в том же
/// повышенном вызове, что и pacman). Скрипт статичен; пользователь берётся
/// из PKEXEC_UID (pkexec) или SUDO_USER (sudo), а не подставляется в строку.
pub fn arch_post_install(pkg_id: &str) -> Option<&'static str> {
    Some(match pkg_id {
        "docker.io" => {
            // Демон + доступ без sudo (после перелогина).
            "systemctl enable --now docker.service; \
             u=${PKEXEC_UID:+$(id -nu \"$PKEXEC_UID\" 2>/dev/null)}; u=${u:-${SUDO_USER:-}}; \
             [ -n \"$u\" ] && [ \"$u\" != root ] && usermod -aG docker \"$u\"; true"
        }
        "postgresql" => {
            // На Arch кластер не создаётся автоматически.
            "if [ ! -s /var/lib/postgres/data/PG_VERSION ]; then \
             su -l postgres -c \"initdb --locale=C.UTF-8 --encoding=UTF8 -D /var/lib/postgres/data\"; fi; \
             systemctl enable --now postgresql.service; true"
        }
        "mysql-server" => {
            "if [ ! -d /var/lib/mysql/mysql ]; then \
             mariadb-install-db --user=mysql --basedir=/usr --datadir=/var/lib/mysql; fi; \
             systemctl enable --now mariadb.service; true"
        }
        "redis-server" => "systemctl enable --now redis.service; true",
        _ => return None,
    })
}

/// Ставится ли пакет каталога на этой системе из AUR (только Arch-системы:
/// pacman без apt-get/dnf). Такой источник запускается без внешней элевации.
pub fn is_arch_aur_install(id: &str) -> bool {
    matches!(arch_package(id), Some(ArchPkg::Aur(_)))
        && which::which("pacman").is_ok()
        && which::which("apt-get").is_err()
        && which::which("dnf").is_err()
}

/// Команда для Arch: (program, args). Не повышает права — для pacman это
/// делает вызывающий (run_elevated), AUR-помощник просит pkexec сам.
pub fn build_arch_command(
    id: &str,
    extra: &[String],
) -> Result<(String, Vec<String>), String> {
    let mapped = arch_package(id);
    let (pkgs, aur): (Vec<String>, bool) = match &mapped {
        Some(ArchPkg::Repo(p)) => (p.iter().map(|s| s.to_string()).collect(), false),
        Some(ArchPkg::Aur(p)) => (p.iter().map(|s| s.to_string()).collect(), true),
        None => (vec![id.to_string()], false),
    };

    if aur {
        let Some(helper) = aur_helper() else {
            return Err(format!(
                "Пакет «{}» есть только в AUR. Установите AUR-помощник (yay или paru) и повторите.",
                pkgs.join(", ")
            ));
        };
        // Помощник запускается от пользователя (makepkg не работает от root)
        // и сам просит повышение через pkexec.
        let mut args: Vec<String> = ["-S", "--noconfirm", "--needed", "--sudo", "pkexec"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.extend(pkgs);
        args.extend(extra.iter().cloned());
        return Ok((helper.to_string(), args));
    }

    let mut pacman: Vec<String> = ["-S", "--noconfirm", "--needed"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    pacman.extend(pkgs);
    pacman.extend(extra.iter().cloned());

    match arch_post_install(id) {
        Some(post) => {
            // sh -c '<pacman ... "$@"> && <post>' sh <pacman-args>: пакеты
            // передаются позиционными аргументами, не склеиваются в строку.
            let script = format!("pacman \"$@\" && {{ {post} }}");
            let mut args = vec!["-c".to_string(), script, "sh".to_string()];
            args.extend(pacman);
            Ok(("sh".to_string(), args))
        }
        None => Ok(("pacman".to_string(), pacman)),
    }
}

/// Команда удаления для Arch.
pub fn build_arch_remove(id: &str) -> Result<(String, Vec<String>), String> {
    let pkgs: Vec<String> = match arch_package(id) {
        Some(ArchPkg::Repo(p)) | Some(ArchPkg::Aur(p)) => p.iter().map(|s| s.to_string()).collect(),
        None => vec![id.to_string()],
    };
    let mut args: Vec<String> = ["-Rs", "--noconfirm"].iter().map(|s| s.to_string()).collect();
    args.extend(pkgs);
    with_privileges("pacman", args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debian_names_are_translated() {
        assert_eq!(
            arch_package("docker.io"),
            Some(ArchPkg::Repo(&["docker", "docker-compose"]))
        );
        assert_eq!(arch_package("mysql-server"), Some(ArchPkg::Repo(&["mariadb"])));
        assert!(matches!(arch_package("mongodb-org"), Some(ArchPkg::Aur(_))));
        assert_eq!(arch_package("git"), None);
    }

    #[test]
    fn docker_and_databases_get_service_setup() {
        for id in ["docker.io", "postgresql", "mysql-server", "redis-server"] {
            assert!(arch_post_install(id).is_some(), "{id}");
        }
        assert!(arch_post_install("git").is_none());
    }

    #[test]
    fn plain_package_without_post_install_uses_pacman_needed() {
        let (prog, args) = build_arch_command("git", &[]).unwrap();
        assert_eq!(prog, "pacman");
        assert_eq!(args, ["-S", "--noconfirm", "--needed", "git"]);
    }

    #[test]
    fn service_setup_runs_in_same_shell_with_packages_as_argv() {
        let (prog, args) = build_arch_command("docker.io", &[]).unwrap();
        assert_eq!(prog, "sh");
        assert!(args[1].starts_with("pacman \"$@\" && {"));
        assert_eq!(&args[3..], ["-S", "--noconfirm", "--needed", "docker", "docker-compose"]);
    }
}

