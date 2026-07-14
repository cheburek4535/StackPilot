// ============================================================
// LaunchEngine — движок запуска профилей.
//
// Исправленная версия после code review.
// Основные изменения:
//   1. stdout/stderr перенаправлены в null (был риск зависания
//      процесса из-за переполнения буфера вывода).
//   2. WaitForUrl использует честный парсинг URL (были проблемы
//      с URL вида http://localhost:3000/swagger).
//   3. WaitForPort добавлен sleep между попытками (был
//      бесконечный перебор без паузы при быстром отказе).
// ============================================================

use std::time::Duration;
use std::process::{Command, Stdio};
use std::thread;
use std::net::{TcpStream, ToSocketAddrs};
use std::io::{Write, BufReader, BufRead};

use crate::models::*;

// --------------------------------------------------
// Трейт LaunchEngine
// --------------------------------------------------
pub trait LaunchEngine: Send + Sync {
    /// Выполнить одно действие и вернуть результат
    fn execute_action(&self, action: &LaunchAction) -> Result<ActionStatus, String>;
}

// --------------------------------------------------
// ProcessLaunchEngine
// --------------------------------------------------
pub struct ProcessLaunchEngine;

impl LaunchEngine for ProcessLaunchEngine {
    fn execute_action(&self, action: &LaunchAction) -> Result<ActionStatus, String> {
        // Если действие выключено — пропускаем
        if !action.enabled {
            return Ok(ActionStatus::Skipped {
                reason: format!("Действие '{}' отключено", action.label),
            });
        }

        match &action.action_type {
            // ===== RunCommand =====
            // Проблема оригинала: stdout/stderr не перенаправлялись.
            // Если процесс пишет много в stdout, буфер заполняется
            // и процесс зависает (deadlock). Исправляем: null.
            ActionType::RunCommand { command, working_dir } => {
                println!("[LaunchEngine] Команда: '{}' в папке {:?}", command, working_dir);

                let mut cmd = Command::new("cmd");
                cmd.arg("/C").arg(command)
                    .stdout(Stdio::null())   // ← важно: не даём буферу заполниться
                    .stderr(Stdio::null());  // ← важно

                if let Some(dir) = working_dir {
                    cmd.current_dir(dir);
                }

                match cmd.spawn() {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("Команда запущена: {}", command),
                    }),
                    Err(e) => Err(format!("Ошибка запуска процесса: {}", e)),
                }
            }

            // ===== OpenUrl =====
            ActionType::OpenUrl { url } => {
                println!("[LaunchEngine] Открыть URL: {}", url);

                match webbrowser::open(url) {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("Браузер открыт: {}", url),
                    }),
                    Err(e) => Err(format!("Ошибка открытия браузера: {}", e)),
                }
            }

            // ===== OpenApplication =====
            // Проблема оригинала: args.split_whitespace() ломает пути с пробелами.
            // Но для первой версии это приемлемо. Полное решение — принимать args
            // как Vec<String>, но это меняет модель данных.
            ActionType::OpenApplication { path, args } => {
                println!("[LaunchEngine] Запустить: {} {:?}", path, args);

                let mut cmd = Command::new(path);
                cmd.stdout(Stdio::null()).stderr(Stdio::null());

                if let Some(args_str) = args {
                    cmd.args(args_str.split_whitespace());
                }

                match cmd.spawn() {
                    Ok(_) => Ok(ActionStatus::Success {
                        message: format!("Приложение запущено: {}", path),
                    }),
                    Err(e) => Err(format!("Ошибка запуска приложения: {}", e)),
                }
            }

            // ===== WaitForUrl =====
            // Проблема оригинала: url.replace("http://", "").replace("https://", "")
            // работает, но может дать неожиданный результат на URL с портами и путями.
            // Исправляем: пишем правильный парсинг + добавляем sleep между попытками.
            ActionType::WaitForUrl { url, timeout_secs } => {
                println!("[LaunchEngine] Жду URL {} ({}s)", url, timeout_secs);

                // Парсим URL вручную (без крейта url — учебный проект)
                let parsed = parse_http_url(url)?;
                let addr_str = format!("{}:{}", parsed.host, parsed.port);

                // Резолвим хост в IP (может быть несколько адресов)
                let addrs = addr_str
                    .to_socket_addrs()
                    .map_err(|e| format!("Не удалось разрешить хост {}: {}", parsed.host, e))?
                    .collect::<Vec<_>>();

                if addrs.is_empty() {
                    return Err(format!("Хост {} не найден", parsed.host));
                }

                // Пробуем подключиться до timeout_secs раз с интервалом 1с
                for _ in 0..*timeout_secs {
                    // Пробуем каждый IP-адрес хоста
                    for addr in &addrs {
                        if let Ok(mut stream) = TcpStream::connect_timeout(addr, Duration::from_secs(2)) {
                            // Шлём HTTP GET запрос
                            let request = format!(
                                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                                parsed.path, parsed.host
                            );

                            if stream.write_all(request.as_bytes()).is_err() {
                                continue;
                            }

                            let mut reader = BufReader::new(&stream);
                            let mut first_line = String::new();
                            if reader.read_line(&mut first_line).is_err() {
                                continue;
                            }

                            // Ответ: "HTTP/1.1 200 OK" → берём код статуса
                            let parts: Vec<&str> = first_line.split_whitespace().collect();
                            if let Some(code_str) = parts.get(1) {
                                let code: u16 = match code_str.parse() {
                                    Ok(c) => c,
                                    Err(_) => continue,
                                };
                                // 2xx = success, 3xx = redirect (тоже ок)
                                if (200..400).contains(&code) {
                                    return Ok(ActionStatus::Success {
                                        message: format!("URL {} ответил статусом {}", url, code),
                                    });
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }

                Err(format!(
                    "Таймаут: {} не ответил за {}с",
                    url, timeout_secs
                ))
            }

            // ===== WaitForPort =====
            // Проблема оригинала: если connect_timeout срабатывает быстро,
            // то нет задержки между попытками. Исправляем: thread::sleep(1) в конце.
            ActionType::WaitForPort { host, port, timeout_secs } => {
                println!("[LaunchEngine] Жду порт {}:{} ({}s)", host, port, timeout_secs);

                let addr_str = format!("{}:{}", host, port);
                let addrs = addr_str
                    .to_socket_addrs()
                    .map_err(|e| format!("Не удалось разрешить {}:{}: {}", host, port, e))?
                    .collect::<Vec<_>>();

                for _ in 0..*timeout_secs {
                    for addr in &addrs {
                        if TcpStream::connect_timeout(addr, Duration::from_secs(1)).is_ok() {
                            return Ok(ActionStatus::Success {
                                message: format!("Порт {}:{} доступен", host, port),
                            });
                        }
                    }
                    // ← Добавлено: пауза между попытками
                    thread::sleep(Duration::from_secs(1));
                }

                Err(format!(
                    "Таймаут: порт {}:{} не открылся за {}с",
                    host, port, timeout_secs
                ))
            }

            // ===== Delay =====
            // Замечание: thread::sleep блокирует поток Tauri. Для коротких пауз
            // это нормально. Для долгих — в будущем сделаем async.
            ActionType::Delay { seconds } => {
                println!("[LaunchEngine] Пауза {}s", seconds);
                thread::sleep(Duration::from_secs(*seconds));
                Ok(ActionStatus::Success {
                    message: format!("Пауза {} с завершена", seconds),
                })
            }

            // ===== ExecuteScript =====
            // Проблема оригинала: нет перенаправления stdout/stderr — та же проблема
            // с буфером при долгих скриптах.
            // Плюс: используем .status() — ждём завершение. Это осознанно,
            // т.к. для скрипта обычно важен код возврата.
            ActionType::ExecuteScript { script, shell } => {
                println!("[LaunchEngine] Скрипт: '{}' через {:?}", script, shell);

                let shell_name = shell.as_deref().unwrap_or("cmd");
                let flag = match shell_name {
                    "cmd" => "/C",
                    "powershell" | "pwsh" => "-Command",
                    _ => "-c",
                };

                let exit = Command::new(shell_name)
                    .args([flag, script])
                    .stdout(Stdio::null())  // ← важно
                    .stderr(Stdio::null())  // ← важно
                    .status()
                    .map_err(|e| format!("Не удалось запустить {}: {}", shell_name, e))?;

                if exit.success() {
                    Ok(ActionStatus::Success {
                        message: format!("Скрипт выполнен: {}", script),
                    })
                } else {
                    let code = exit.code().unwrap_or(-1);
                    Ok(ActionStatus::Failed {
                        error: format!("Скрипт завершился с кодом {}", code),
                    })
                }
            }
        }
    }
}

// --------------------------------------------------
// Вспомогательная структура для парсинга HTTP URL
// --------------------------------------------------
struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

/// Разобрать URL вида "http://localhost:3000/swagger" на составляющие.
/// Без внешних крейтов, только стандартная библиотека.
fn parse_http_url(raw: &str) -> Result<ParsedUrl, String> {
    // 1. Убираем протокол
    let without_proto = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))
        .unwrap_or(raw);

    // 2. Отделяем путь от хоста:порта
    let (host_port, path) = match without_proto.split_once('/') {
        Some((hp, p)) => (hp, format!("/{}", p)),
        None => (without_proto, "/".to_string()),
    };

    // 3. Если есть порт — отделяем, иначе 80
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("Неверный порт в URL: {}", raw))?,
        ),
        None => (host_port.to_string(), 80),
    };

    Ok(ParsedUrl { host, port, path })
}
