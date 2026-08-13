# Toolchain Icons — static/images/

Иконки системных инструментов (toolchain), показываемые на экранах
**Environment check** (мастер создания проекта) и **Окружение** (health).

## Как это работает

- Каждый инструмент в `src-tauri/src/modules/toolchain/tools.json` имеет
  поле `"icon": "<filename>"`.
- Бэкенд присылает это имя фронтенду в `ToolRequirement.icon` /
  `InstallTask.icon` / `CheckProgressEvent.icon` / `ToolHealth.icon`.
- Фронтенд рендерит `<img src="/images/<filename>">` (компонент `TechIcon.svelte`).
- Если файла нет или `icon` = null — показывается дефолтная SVG-шестерёнка
  (просто пропустите ненужный файл, ничего не сломается).

## Как добавить иконку

1. Скачайте файл и положите в **эту папку** (`static/images/`) **строго**
   с именем из таблицы ниже (регистр важен).
2. Форматы: svg (preferred), png, webp — как указано в таблице.
3. Перезапустите приложение (имена компилируются в сборку SvelteKit
   только из `static/` при `vite dev`/`build`).

## Таблица файлов

| Инструмент (tools.json id) | Что показывает | Файл | Статус |
|---|---|---|---|
| winget | Winget (App Installer) | `winget.svg` | ⬇ нужен |
| node | Node.js | `node.svg` | ✓ есть |
| npm | NPM | `npm.svg` | ✓ есть |
| python | Python | `python.svg` | ✓ есть |
| pip | Pip | `pip.svg` | ⬇ нужен |
| rust | Rust (rustup) | `rust.png` | ✓ есть |
| rustc | Rustc | `rust.png` | ✓ (общий) |
| cargo | Cargo | `rust.png` | ✓ (общий) |
| tauri-cli | Tauri CLI | `tauri.svg` | ✓ есть |
| go | Go | `go.webp` | ✓ есть |
| java | Java (Temurin JDK) | `java.svg` | ✓ есть |
| kotlin | Kotlin (kotlinc) | `kotlin.svg` | ✓ есть |
| kafka | Apache Kafka | `kafka.svg` | ✓ есть |
| grafana | Grafana | `grafana.svg` | ✓ есть |
| terraform | Terraform | `terra.svg` | ✓ есть |
| firebase | Firebase CLI | `firebase.svg` | ✓ есть |
| dotnet | .NET SDK | `dotnet.svg` | ⬇ нужен |
| csharprepl | C# REPL (CSharpRepl) | `csharp.svg` | ✓ есть |
| msvc-build-tools | MSVC Build Tools (C++) | `msvc.svg` | ⬇ нужен |
| zig | Zig | `zig.svg` | ✓ есть |
| dart | Dart SDK | `dart.svg` | ✓ есть |
| php | PHP | `php.webp` | ✓ есть |
| composer | Composer | `composer.svg` | ⬇ нужен |
| swift | Swift | `swift.svg` | ✓ есть |
| erlang | Erlang/OTP | `erlang.svg` | ⬇ нужен |
| elixir | Elixir | `elixir.svg` | ✓ есть |
| gleam | Gleam | `gleam.svg` | ✓ есть |
| flutter | Flutter SDK | `flutter.svg` | ✓ есть |
| maven | Maven | `maven.svg` | ✓ есть |
| gradle | Gradle | `gradle.svg` | ✓ есть |
| postgresql | PostgreSQL | `postgresql.svg` | ✓ есть |
| redis | Redis (Memurai) | `redis.svg` | ✓ есть |
| mongodb | MongoDB | `mongodb.svg` | ✓ есть |
| mysql | MySQL | `mysql.svg` | ⬇ нужен |
| sqlite | SQLite | `sqlite.svg` | ✓ есть |
| docker | Docker Desktop | `docker.svg` | ✓ есть |
| git | Git | `git.png` | ✓ есть |
| vscode | VS Code | `vscode.svg` | ⬇ нужен |
| cmake | CMake | `cmake.svg` | ⬇ нужен |
| make | Make | `make.svg` | ⬇ нужен |
| curl | Curl | `curl.svg` | ⬇ нужен |
| tar | Tar | `tar.svg` | ⬇ нужен |
| unity | Unity Editor | `unity.svg` | ✓ есть |
| unreal | Unreal Engine | `unreal-engine.svg` | ✓ есть |
| godot | Godot | `godot.svg` | ✓ есть |
| android | Android SDK | `android.svg` | ✓ есть |
| qt | Qt | `qt.svg` | ✓ есть |
| xcodebuild | Xcode (xcodebuild) | `xcodebuild.svg` | ⬇ нужен |

**Скачать нужно 13 файлов:** `winget.svg`, `pip.svg`, `dotnet.svg`, `msvc.svg`,
`composer.svg`, `erlang.svg`, `mysql.svg`, `vscode.svg`, `cmake.svg`,
`make.svg`, `curl.svg`, `tar.svg`, `xcodebuild.svg` — брендовые иконки легко
находятся в репозитории [simple-icons](https://simpleicons.org/) или в
официальных гайдлайнах продуктов.