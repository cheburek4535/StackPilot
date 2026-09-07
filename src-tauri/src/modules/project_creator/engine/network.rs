// ============================================================================
// Классификация сетевых действий и советы при недоступности реестров.
//
// Сетевые шаги (terraform init, npm install, pip install, dotnet restore,
// cardo add и т.п.) — единственные, кто может «внезапно» сломаться из-за
// окружения, а не кода: реестр недоступен, соединение рвётся, регион
// блокирует registry. Движок:
//
//   1. не верит первой ошибке — повторяет сетевую команду несколько раз
//      (NETWORK_MAX_ATTEMPTS) с паузой NETWORK_RETRY_BACKOFF;
//   2. если сеть так и не поднялась — даёт пользователю понятную ошибку с
//      советом (VPN/прокси/зеркало), а не просто «Command failed»;
//   3. итоговый сетевой сбой НЕ прерывает генерацию (engine переводит его в
//      PartialFailure): файлы проекта уже созданы, команду можно повторить
//      вручную после починки сети;
//   4. НЕ сетевые ошибки (битый манифест, отсутствующий CLI, ошибка сборки)
//      по-прежнему обрабатываются строго по on_error рецепта: Abort остаётся
//      Abort. Различие — по маркерам сети в диагностике (см.
//      has_network_failure_markers), а не только по команде.
// ============================================================================

use std::time::Duration;

use serde_json::Value;

/// Сколько всего попыток (включая первую) даётся сетевой команде.
pub const NETWORK_MAX_ATTEMPTS: u32 = 3;

/// Пауза между повторами сетевой команды.
pub const NETWORK_RETRY_BACKOFF: Duration = Duration::from_millis(1000);

/// Фрагменты диагностики, по которым сбой классифицируется как сетевая
/// проблема (недоступный реестр/хост, TLS, таймаут соединения и т.п.).
/// Сверка по нижнему регистру.
const NETWORK_ERROR_MARKERS: &[&str] = &[
    // Resolve/DNS
    "could not resolve host",
    "failed to resolve",
    "dns lookup",
    "dns resolution",
    "dns_error",
    "eai_again",
    "eai_noname",
    // Уровень соединения
    "etimedout",
    "econnrefused",
    "econnreset",
    "enotfound",
    "ehostdown",
    "ehostunreach",
    "connection refused",
    "connection reset",
    "reset by peer",
    "failed to connect",
    "unable to connect",
    "could not connect",
    "network is unreachable",
    "no route to host",
    "host unreachable",
    "deadline exceeded",
    "timed out",
    "timeout",
    "fetch failed",
    "failed to fetch",
    // Реестры пакетов/провайдеров
    "registry.terraform.io",
    "registry.npmjs.org",
    "registry.yarnpkg.com",
    "packagist.org",
    "proxy.golang.org",
    "crates.io",
    "raw.githubusercontent.com",
    "github.com",
    "pypi.org",
    "files.pythonhosted.org",
    "start.spring.io",
    "registry",
    "failed to query",
    "could not retrieve",
    "giving up after",
    "give up after",
    "unable to download",
    "cannot download",
    "download failed",
    "failed to download",
    "while downloading",
    // TLS / сертификаты / прокси
    "certificate verify failed",
    "self-signed certificate",
    "tls handshake",
    "ssl error",
    "proxy",
    "winhttp",
    "winsock",
    // HTTP-статусы недоступного сервера / rate-limit
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway",
    "http 502",
    "http 503",
    "http 504",
    "too many requests",
    "rate limit",
    "curl: (6",
    "curl: (7",
    "curl: (28",
    "curl: (35",
    "curl: (56",
];

/// Похоже ли сообщение об ошибке на проблему сети/реестра.
pub fn has_network_failure_markers(text: &str) -> bool {
    let lower = text.to_lowercase();
    NETWORK_ERROR_MARKERS.iter().any(|m| lower.contains(m))
}

/// Нормализованный базовый идентификатор команды: нижний регистр, без пути,
/// без типичных расширений исполняемых файлов
/// (npx.cmd → npx, venv\Scripts\pip.exe → pip).
fn command_basename(command: &str) -> String {
    let forward = command.replace('\\', "/");
    let base = forward.rsplit('/').next().unwrap_or(&forward);
    let mut name = base.to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".dll"] {
        if let Some(stripped) = name.strip_suffix(ext) {
            name = stripped.to_string();
            break;
        }
    }
    name
}

/// Токены аргументов без ведущих «-» (--offline → offline, -n → n).
/// Используются для различения «npm --version» (локально) и
/// «npm install» (сеть), «go mod init» (локально) и «go get» (сеть) и т.д.
fn arg_tokens(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|a| a.trim_start_matches('-').to_lowercase())
        .collect()
}

fn any_token(tokens: &[String], verbs: &[&str]) -> bool {
    tokens.iter().any(|t| verbs.contains(&t.as_str()))
}

fn tokens_have(tokens: &[String], needle: &str) -> bool {
    tokens.iter().any(|t| t == needle)
}

/// Является ли запуск команды «сетевым действием» — запросом во внешний
/// реестр пакетов/провайдеров/шаблонов (а не локальной операцией вроде
/// --version, git add, cargo init).
pub fn is_network_command(command: &str, args: &[String]) -> bool {
    let base = command_basename(command);
    if tokens_have(&arg_tokens(args), "offline")
        || tokens_have(&arg_tokens(args), "no-index")
        || tokens_have(&arg_tokens(args), "no-install")
    {
        return false;
    }
    let tokens = arg_tokens(args);
    match base.as_str() {
        // ---- Node.js / JS ----
        "npm" => any_token(
            &tokens,
            &[
                "install",
                "i",
                "add",
                "ci",
                "dedupe",
                "update",
                "outdated",
                "uninstall",
                "rm",
                "publish",
                "pack",
            ],
        ),
        "npx" => !any_token(&tokens, &["version", "v"]),
        "pnpm" | "yarn" | "yarnpkg" | "bun" => any_token(
            &tokens,
            &[
                "install", "i", "add", "ci", "upgrade", "update", "remove", "rm", "dlx", "create",
            ],
        ),
        "deno" => any_token(
            &tokens,
            &["install", "add", "cache", "uninstall", "upgrade"],
        ),
        // ---- Python ----
        // `python -m pip install ...` — сеть; `python -m venv ...` / `--version` — нет.
        "python" | "python3" | "py" => {
            tokens_have(&tokens, "pip") && any_token(&tokens, &["install", "download"])
        }
        "pip" | "pip3" | "pipx" => any_token(&tokens, &["install", "download", "uninstall"]),
        "uv" => any_token(&tokens, &["add", "install", "sync", "pip", "remove"]),
        "poetry" => any_token(&tokens, &["install", "add", "lock", "update", "remove"]),
        "conda" => any_token(&tokens, &["install", "create", "update", "remove"]),
        // ---- Go ----
        "go" => {
            any_token(&tokens, &["get", "install"])
                || tokens_have(&tokens, "mod")
                    && (any_token(&tokens, &["download", "tidy", "vendor", "verify"])
                        || !any_token(&tokens, &["init", "edit"]))
        }
        // ---- Rust ----
        "cargo" => any_token(
            &tokens,
            &["add", "fetch", "install", "uninstall", "search", "update"],
        ),
        // ---- .NET / NuGet ----
        "dotnet" | "nuget" => !any_token(&tokens, &["version", "v"]),
        // ---- JVM (Maven/Gradle) ----
        "mvn" | "mvnw" | "maven" | "gradle" | "gradlew" => {
            !any_token(&tokens, &["version", "v", "offline"])
        }
        // ---- PHP / Composer ----
        "composer" => !any_token(&tokens, &["version", "v", "about"]),
        "php" => tokens.iter().any(|t| t.ends_with(".phar")),
        // ---- Dart / Flutter ----
        "flutter" | "dart" => any_token(
            &tokens,
            &["pub", "create", "add", "get", "upgrade", "fetch", "install"],
        ),
        // ---- Terraform / OpenTofu ----
        "terraform" | "tofu" | "opentofu" => any_token(
            &tokens,
            &[
                "init",
                "apply",
                "plan",
                "destroy",
                "get",
                "providers",
                "required-version",
            ],
        ),
        // ---- Git ----
        "git" => any_token(
            &tokens,
            &["clone", "fetch", "pull", "push", "submodule", "lfs"],
        ),
        // ---- Поставка исходников напрямую ----
        "zig" => any_token(&tokens, &["fetch", "build"]),
        "curl" | "wget" | "aria2c" => !any_token(&tokens, &["version"]),
        "helm" | "kubectl" | "kustomize" => {
            any_token(&tokens, &["install", "upgrade", "repo", "pull", "apply"])
        }
        _ => false,
    }
}

/// Команда и аргументы из generator_config (scaffold/cli-генераторы).
pub fn generator_command_and_args(config: &Value) -> (String, Vec<String>) {
    let command = config
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    let args = config
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    (command, args)
}

/// Является ли Generate-шаг (scaffold какого-либо CLI) сетевым действием.
/// spring-boot качает проект с start.spring.io — всегда сеть.
pub fn is_network_generator(generator_id: &str, config: &Value) -> bool {
    if generator_id == "spring-boot" {
        return true;
    }
    if generator_id != "scaffold" {
        return false;
    }
    let (command, args) = generator_command_and_args(config);
    !command.is_empty() && is_network_command(&command, &args)
}

/// Понятный совет при устойчивом сетевом сбое.
pub fn network_failure_hint(program: &str, args: &[String]) -> String {
    let command_line = {
        let mut line = program.to_string();
        for arg in args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    };
    let registry = match command_basename(program).as_str() {
        "terraform" | "tofu" | "opentofu" => {
            "the Terraform provider registry (registry.terraform.io)"
        }
        "npm" | "npx" | "pnpm" | "yarn" | "yarnpkg" | "bun" | "deno" => "the npm package registry",
        "pip" | "pip3" | "pipx" | "python" | "python3" | "py" | "uv" | "poetry" | "conda" => {
            "the Python package index (PyPI)"
        }
        "go" => "the Go module proxy",
        "cargo" => "crates.io",
        "dotnet" | "nuget" => "NuGet",
        "mvn" | "mvnw" | "maven" | "gradle" | "gradlew" => {
            "Maven Central / the Gradle Plugin Portal"
        }
        "composer" | "php" => "Packagist (the Composer repository)",
        "flutter" | "dart" => "pub.dev",
        "zig" => "the upstream project archive",
        "git" => "the remote Git repository",
        "curl" | "wget" | "aria2c" => "the remote server",
        // spring-boot generator не имеет `program` в конфиге — программа пустая
        "" => "Spring Initializr (start.spring.io)",
        _ => "the package/plugin registry",
    };
    format!(
        "{} {}\n\n\
         This step could not reach {} over the network — the generated project\n\
         is left incomplete only for this step.\n\n\
         Suggestions:\n\
         \x20 - check your internet connection and that the registry is reachable;\n\
         \x20 - if the registry is blocked in your region, enable a VPN or a proxy;\n\
         \x20 - or configure a registry/providers mirror for the tool, then re-run\n\
         \x20   the command manually once the network is available.",
        "Network error", command_line, registry,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn classifies_terraform_init_as_network_but_version_as_local() {
        assert!(is_network_command("terraform", &args(&["init"])));
        assert!(is_network_command("terraform", &args(&["apply"])));
        assert!(is_network_command("terraform", &args(&["destroy"])));
        assert!(!is_network_command("terraform", &args(&["version"])));
        assert!(!is_network_command("terraform", &args(&["validate"])));
    }

    #[test]
    fn classifies_node_ecosystem_by_args() {
        assert!(is_network_command("npm", &args(&["install"])));
        assert!(is_network_command("npm", &args(&["ci"])));
        assert!(is_network_command(
            "npx",
            &args(&["-p", "typescript", "tsc", "--init"])
        ));
        assert!(!is_network_command("npm", &args(&["run", "build"])));
        assert!(!is_network_command("npm", &args(&["--version"])));
        assert!(!is_network_command("npx", &args(&["--version"])));
        assert!(!is_network_command(
            "node",
            &args(&["-e", "console.log(1)"])
        ));
    }

    #[test]
    fn classifies_python_and_pip() {
        assert!(is_network_command(
            r"C:\dev\myapp\backend\venv\Scripts\python.exe",
            &args(&["-m", "pip", "install", "-r", "requirements.txt"])
        ));
        assert!(is_network_command("pip", &args(&["install", "alembic"])));
        assert!(!is_network_command("python", &args(&["--version"])));
        assert!(!is_network_command(
            "python",
            &args(&["-m", "venv", "venv"])
        ));
    }

    #[test]
    fn classifies_go_and_cargo() {
        assert!(is_network_command(
            "go",
            &args(&["get", "github.com/gin-gonic/gin@latest"])
        ));
        assert!(!is_network_command("go", &args(&["mod", "init", "myapp"])));
        assert!(is_network_command(
            "cargo",
            &args(&["add", "axum", "tokio"])
        ));
        assert!(!is_network_command(
            "cargo",
            &args(&["init", "--name", "x"])
        ));
    }

    #[test]
    fn classifies_dotnet_and_composer_and_flutter() {
        assert!(is_network_command(
            "dotnet",
            &args(&["new", "webapi", "-n", "myapp", "-o", "."])
        ));
        assert!(!is_network_command("dotnet", &args(&["--version"])));
        assert!(is_network_command(
            "composer",
            &args(&["create-project", "laravel/laravel"])
        ));
        assert!(!is_network_command("composer", &args(&["--version"])));
        assert!(!is_network_command("flutter", &args(&["--version"])));
        assert!(is_network_command(
            "flutter",
            &args(&["create", "frontend"])
        ));
    }

    #[test]
    fn php_with_phar_is_network_but_script_check_is_local() {
        assert!(is_network_command(
            "php",
            &args(&[
                "-d",
                "extension=fileinfo",
                r"C:\composer.phar",
                "create-project",
                "laravel/laravel"
            ])
        ));
        assert!(!is_network_command(
            "php",
            &args(&["-d", "extension=fileinfo", "-r", "echo 'x';"])
        ));
    }

    #[test]
    fn offline_flags_disable_network_classification() {
        assert!(!is_network_command(
            "pip",
            &args(&["install", "--no-index", "-r", "requirements.txt"])
        ));
        assert!(!is_network_command("npm", &args(&["install", "--offline"])));
    }

    #[test]
    fn generator_classification() {
        let scaffold = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "my-app"]
        });
        assert!(is_network_generator("scaffold", &scaffold));
        assert!(is_network_generator("spring-boot", &serde_json::json!({})));
        let local = serde_json::json!({
            "command": "echo",
            "args": ["init"]
        });
        assert!(!is_network_generator("scaffold", &local));
    }

    #[test]
    fn detects_terraform_registry_failure() {
        let text = "Error: Failed to query available provider packages\nCould not retrieve the list of available versions for provider kreuzwerker/docker: could not connect to registry.terraform.io: failed to request discovery document: GET https://registry.terraform.io/.well-known/terraform.json giving up after 4 attempts: context deadline exceeded";
        assert!(has_network_failure_markers(text));
    }

    #[test]
    fn does_not_flag_code_errors_as_network() {
        assert!(!has_network_failure_markers("npm ERR! code EJSONPARSE"));
        assert!(!has_network_failure_markers("error: could not run `rustc`"));
        assert!(!has_network_failure_markers(
            "Command failed (exited with status 1)"
        ));
    }

    #[test]
    fn hint_mentions_terraform_registry_and_vpn() {
        let hint = network_failure_hint("terraform", &args(&["init"]));
        assert!(hint.contains("registry.terraform.io"), "{hint}");
        assert!(hint.contains("VPN"), "{hint}");
        assert!(hint.contains("terraform init"), "{hint}");
    }
}
