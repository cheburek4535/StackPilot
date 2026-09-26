use crate::modules::project_creator::generators::SCAFFOLD_TARGET;

fn no_cancel() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}
    use super::*;
    use crate::modules::project_creator::engine::process::{
        CommandRunner, ExecutionEventSink, ProcessErrorKind, ProcessExecutionError, ProcessOutput,
        ProcessSpec,
    };

    fn context() -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: vec!["typescript".into()],
            frameworks: vec!["nextjs".into()],
            // Полная сессия мастера: фичи выбраны явно (default() — всё off).
            git_init: true,
            vscode_config: true,
            ..Default::default()
        }
    }

    fn recipe_for(ctx: &WizardContext, folder_name: &str) -> Result<Recipe, String> {
        compose_recipe(
            &ProjectLayout::compute(ctx),
            ctx,
            folder_name,
            Path::new("."),
        )
    }

    fn cmd_args(step: &Step) -> Vec<String> {
        match step {
            Step::Command { args, .. } => args.clone(),
            other => panic!("ожидался Command, получили {:?}", other.id()),
        }
    }

    /// Путь канонического venv из шага py_venv_create на любой платформе:
    /// Windows — последний аргумент `python -m venv <путь>`, Unix — конфиг
    /// генератора "python-venv" (ensurepip-less дистрибутивы).
    fn venv_path_of(step: &Step) -> String {
        match step {
            Step::Command { args, .. } => args.last().cloned().unwrap_or_default(),
            Step::Generate {
                generator_id,
                generator_config,
                ..
            } => {
                assert_eq!(
                    generator_id, "python-venv",
                    "создание venv на Unix — генератор python-venv"
                );
                generator_config
                    .get("venv_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            }
            other => panic!("ожидался Command/Generate, получили {:?}", other.id()),
        }
    }

    fn gen_args(config: &serde_json::Value) -> Vec<String> {
        config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn gen_strs(config: &serde_json::Value, key: &str) -> Vec<String> {
        config
            .get(key)
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn crc32_matches_zig_std_hash() {
        // Значения сверены с `std.hash.Crc32.hash(...)` (Zig 0.16.0) — те же,
        // что Zig использует для fingerprint в build.zig.zon.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"my_app"), 0x542B_89B0);
        assert_eq!(crc32(b"other_name"), 0x3767_28D4);
        assert_eq!(crc32(b"ziginit"), 0x36AD_9EBC);
    }

    #[test]
    fn zig_package_name_is_valid_identifier() {
        assert_eq!(zig_package_name("my-app"), "my_app");
        assert_eq!(zig_package_name("My App"), "my_app");
        assert_eq!(zig_package_name("myApp"), "myapp");
        // имя-ключевое слово получает суффикс (`.name = .fn` не парсится)
        assert_eq!(zig_package_name("fn"), "fn_");
        assert_eq!(zig_package_name("switch"), "switch_");
        // не начинается с цифры, не пустое, не длиннее 32 байт
        assert_eq!(zig_package_name("123"), "_123");
        assert_eq!(zig_package_name("!!!"), "app");
        let long = zig_package_name(&"a".repeat(50));
        assert!(long.len() <= 32, "{long}");
    }

    #[test]
    fn zap_zon_has_valid_fingerprint_and_paths() {
        // Зон с interpolated-именем: fingerprint обязан содержать crc32 имени
        // в верхних 32 битах (Zig 0.14+), paths — присутствовать.
        let steps = steps_for_framework(
            "zap",
            "C:\\dev\\myapp",
            "my_app",
            &context(),
            &ProjectLayout::compute(&context()),
        );
        let zon = steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path.ends_with("build.zig.zon")))
            .unwrap_or_else(|| panic!("должен быть шаг записи build.zig.zon"));
        match zon {
            Step::WriteFile { content, .. } => {
                let pkg_name = zig_package_name("my_app");
                let expected = format!(
                    ".name = .{pkg_name},\n    .version = \"0.1.0\",\n    .minimum_zig_version = \"0.14.0\",\n    .paths = .{{\"\"}},\n    .fingerprint = 0x{:016x},",
                    (u64::from(crc32(pkg_name.as_bytes())) << 32) | 0xCAFE_BABE
                );
                assert!(content.contains(&expected), "zon: {content}");
            }
            other => panic!("ожидался WriteFile, получили {:?}", other.id()),
        }
    }

    #[test]
    fn zap_fetch_uses_zap_v0101() {
        let steps = steps_for_framework(
            "zap",
            "C:\\dev\\myapp",
            "my_app",
            &context(),
            &ProjectLayout::compute(&context()),
        );
        let fetch = steps
            .iter()
            .find(|s| matches!(s, Step::Command { id, .. } if id == "zap_fetch"))
            .unwrap_or_else(|| panic!("должен быть шаг zap_fetch"));
        let args = cmd_args(fetch);
        assert!(
            args.iter().any(|a| a.contains("v0.10.1.tar.gz")),
            "zap_fetch должен ссылаться на v0.10.1 (старые теги не парсятся Zig 0.13+): {args:?}"
        );
    }

    /// Android-контекст: язык явно назначен фронтенд-стороне (мобильный
    /// клиент), поэтому каноническая раскладка — FrontendOnly, и android
    /// пишет файлы в корень (каталоги не сегментируются). Тесты проверяют
    /// СОДЕРЖИМОЕ gradle-файлов, а не маршрутизацию: маршрут android →
    /// frontend/ в split-стеках покрыт тестами ProjectLayout.
    fn android_context(frameworks: &[&str], languages: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frontend_languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            git_init: true,
            vscode_config: true,
            ..Default::default()
        }
    }

    fn write_paths(steps: &[Step]) -> Vec<&str> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::WriteFile { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    fn write_content<'a>(steps: &'a [Step], path: &str) -> &'a str {
        steps
            .iter()
            .find_map(|s| match s {
                Step::WriteFile {
                    path: p, content, ..
                } if p == path => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("нет шага записи {path}"))
    }

    #[test]
    fn android_generates_full_gradle_project() {
        let ctx = android_context(&["android"], &["kotlin"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        for expected in [
            "settings.gradle.kts",
            "build.gradle.kts",
            "gradle.properties",
            "app/build.gradle.kts",
            "app/src/main/AndroidManifest.xml",
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        ] {
            assert!(
                write_paths(&steps).contains(&expected),
                "нет файла {expected}"
            );
        }
        // без jetpack-compose — никакого compose-плагина и compose-импортов
        assert!(!write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(!main.contains("androidx.compose"), "{main}");
        assert!(main.contains("android.app.Activity"), "{main}");
        // манифест: метка проекта экранирована, тема — платформенная (без res/)
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(manifest.contains("android:label=\"myapp\""), "{manifest}");
    }

    #[test]
    fn gradle_wrapper_working_dir_has_segment_applied_once() {
        // Регрессия: wrapper-шаги получали сегмент ДВАЖДЫ (backend/backend,
        // frontend/frontend) — рабочей директории не существовало, gradle
        // не запускался. into_segment обязан применять сегмент ровно один раз.
        let ctx = ctx_scenario(&["kotlin"], &["ktor", "android"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.framework_dir("ktor").as_deref(), Some("backend"));
        assert_eq!(layout.framework_dir("android").as_deref(), Some("frontend"));

        let ktor_steps = steps_for_framework("ktor", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let ktor_wrapper = ktor_steps
            .iter()
            .find(|s| s.id() == "ktor_gradle_wrapper")
            .expect("ktor_gradle_wrapper должен быть в шагах ktor");
        assert_eq!(
            wd_of(ktor_wrapper),
            "C:\\dev\\myapp/backend",
            "wrapper ktor обязан запускаться в backend/ без двойного сегмента"
        );

        let android_steps = steps_for_framework("android", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let android_wrapper = android_steps
            .iter()
            .find(|s| s.id() == "android_gradle_wrapper")
            .expect("android_gradle_wrapper должен быть в шагах android");
        assert_eq!(
            wd_of(android_wrapper),
            "C:\\dev\\myapp/frontend",
            "wrapper android обязан запускаться в frontend/ без двойного сегмента"
        );
    }

    #[test]
    fn android_with_java_language_generates_java_activity() {
        let ctx = android_context(&["android"], &["java"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let main = write_content(
            &steps,
            "app/src/main/java/com/example/app/MainActivity.java",
        );
        assert!(
            main.contains("public class MainActivity extends Activity"),
            "{main}"
        );
        let app_build = write_content(&steps, "app/build.gradle.kts");
        assert!(!app_build.contains("org.jetbrains.kotlin"), "{app_build}");
    }

    #[test]
    fn android_with_jetpack_compose_generates_compose_project() {
        let ctx = android_context(&["android", "jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let root = write_content(&steps, "build.gradle.kts");
        assert!(
            root.contains("org.jetbrains.kotlin.plugin.compose"),
            "{root}"
        );
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("setContent"), "{main}");
        assert!(main.contains("androidx.compose.material3"), "{main}");
        // jetpack-compose рядом с android не пишет свои файлы (иначе —
        // дубликаты путей, см. duplicate_framework_write_paths)
        let compose_steps = steps_for_framework(
            "jetpack-compose",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(compose_steps.is_empty(), "{compose_steps:?}");
    }

    #[test]
    fn jetpack_compose_standalone_generates_full_project() {
        let ctx = android_context(&["jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework(
            "jetpack-compose",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(write_paths(&steps).contains(&"app/build.gradle.kts"));
        assert!(write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("setContent"), "{main}");
    }

    #[test]
    fn android_files_escape_user_text() {
        let ctx = WizardContext {
            project_name: Some("My \"App\" $v1".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: vec!["kotlin".into()],
            frontend_languages: vec!["kotlin".into()],
            frameworks: vec!["android".into()],
            ..Default::default()
        };
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "My \"App\" $v1",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(
            manifest.contains("android:label=\"My &quot;App&quot; $v1\""),
            "{manifest}"
        );
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("Hello from My \\\"App\\\" \\$v1!"), "{main}");
    }

    #[test]
    fn frontend_frameworks_use_project_subfolder() {
        // Фронтенды скаффолдятся в frontend/ (ScaffoldGenerator), а не в
        // корне: иначе они перезапишут package.json бэкенда (express+nextjs
        // и т.п.). solidjs остаётся обычным Command, создающим подпапку
        // <project_name>.
        for fw_id in ["nextjs", "nuxt", "sveltekit", "expo", "react"] {
            let steps = steps_for_framework(
                fw_id,
                "C:\\dev\\myapp",
                "myapp",
                &context(),
                &ProjectLayout::compute(&context()),
            );
            let scaffold = steps
                .iter()
                .find(|s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"))
                .unwrap_or_else(|| panic!("{fw_id}: scaffold-шаг должен быть в плане"));
            match scaffold {
                Step::Generate {
                    generator_config, ..
                } => {
                    assert_eq!(
                        generator_config.get("target_dir").and_then(|v| v.as_str()),
                        Some("frontend"),
                        "{fw_id} должен скаффолдиться в frontend/: {generator_config}"
                    );
                }
                _ => panic!("{fw_id}: scaffold — Generate"),
            }
        }

        // solidjs — creates_project_and_may_prompt (create-solid может
        // задавать вопросы), каркас кладётся в frontend/ через плейсхолдер,
        // пост-условие — package.json.
        let steps = steps_for_framework(
            "solidjs",
            "C:\\dev\\myapp",
            "myapp",
            &context(),
            &ProjectLayout::compute(&context()),
        );
        let create = steps
            .iter()
            .find(|s| s.id() == "solid_init")
            .unwrap_or_else(|| panic!("solidjs: шаг solid_init должен быть в плане"));
        match create {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_project_and_may_prompt"),
                    "{generator_config}"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "solidjs скаффолдится в frontend/: {generator_config}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["package.json"],
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                assert!(
                    args.contains(&"__TARGET__".to_string()),
                    "create-solid получает имя проекта через плейсхолдер: {args:?}"
                );
            }
            _ => panic!("solid_init — Generate scaffold"),
        }
    }

    #[test]
    fn split_layout_puts_frameworks_into_segments() {
        // nextjs + fastapi: фронтенд — в frontend/, сервер — в backend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let next_steps = steps_for_framework(
            "nextjs",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let scaffold = next_steps
            .iter()
            .find(
                |s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"),
            )
            .expect("nextjs: scaffold-шаг должен быть в плане");
        match scaffold {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
            }
            _ => panic!("nextjs — Generate"),
        }

        let api_steps = steps_for_framework(
            "fastapi",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(
            api_steps.iter().any(
                |s| matches!(s, Step::WriteFile { path, .. } if path == "backend/src/main.py")
            ),
            "fastapi должен писать в backend/src/main.py"
        );
    }

    #[test]
    fn solo_backend_framework_stays_in_root() {
        // Только fastapi (без фронтенда) — сегментации нет, файлы в корне
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];

        let api_steps = steps_for_framework(
            "fastapi",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(
            api_steps
                .iter()
                .any(|s| matches!(s, Step::WriteFile { path, .. } if path == "src/main.py")),
            "fastapi без фронтенда пишет в корень"
        );
    }

    #[test]
    fn compose_recipe_creates_segment_dirs_for_split_stack() {
        // Полный план для nextjs + fastapi: создаются папки backend/ и frontend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "backend/ и frontend/ должны создаваться: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "backend/ и frontend/ должны создаваться: {mkdirs:?}"
        );
    }

    #[test]
    fn inplace_framework_follows_its_own_side() {
        // express — бэкенд-фреймворк (side=backend в wizard_tree). Даже если
        // typescript назначен «фронтенд»-языком, express работает в backend/
        // (в новой модели комбинация «express + бэкенд на другом языке»
        // блокируется валидацией, а здесь проверяется размещение файлов).
        let mut ctx = context();
        ctx.languages = vec!["javascript".into(), "typescript".into()];
        ctx.backend_languages = vec!["javascript".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["express".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );

        // express-файлы пишутся в backend/ (своя сторона), а не в frontend/
        for (step_id, expected_path) in [
            ("express_index", "backend/src/index.js"),
            ("express_package", "backend/package.json"),
        ] {
            let step = recipe
                .steps
                .iter()
                .find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::WriteFile {
                    path, overwrite, ..
                } => {
                    assert_eq!(
                        path, expected_path,
                        "{step_id} должен писать в {expected_path}"
                    );
                    assert!(*overwrite, "{step_id} обязан перезаписать заглушку языка");
                }
                _ => panic!("{step_id} — WriteFile"),
            }
        }
    }

    #[test]
    fn standalone_backend_plus_frontend_segments() {
        // aspnetcore (standalone, backend) + nextjs (standalone, frontend) —
        // валидный полный стек: каждый фреймворк создаётся в своём сегменте.
        let mut ctx = context();
        ctx.languages = vec!["csharp".into(), "typescript".into()];
        ctx.backend_languages = vec!["csharp".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["aspnetcore".into(), "nextjs".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );

        // aspnetcore (dotnet new webapi) выполняется в backend/
        let asp = recipe
            .steps
            .iter()
            .find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command { working_dir, .. } => {
                let wd = working_dir
                    .as_deref()
                    .expect("aspnetcore должен работать в backend/");
                assert!(
                    wd.ends_with("backend"),
                    "aspnetcore должен работать в backend/, а не в корне: {wd}"
                );
            }
            _ => panic!("aspnet_new — Command"),
        }

        // nextjs (create-next-app) — scaffold-генератор: каталогом становится
        // сегмент frontend/ (ScaffoldGenerator выполнит CLI с "." внутри)
        let next = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
            }
            _ => panic!("nextjs_create — Generate"),
        }

        // Скаффолд csharp (dotnet new console) подавлен aspnetcore,
        // js-скаффолд подавлен nextjs
        for suppressed in ["dotnet_new", "package_json", "js_src_index"] {
            assert!(
                !recipe.steps.iter().any(|s| s.id() == suppressed),
                "шаг {suppressed} не должен выполняться: aspnetcore/nextjs создают каркас сами"
            );
        }
    }

    #[test]
    fn aspnetcore_plus_maui_isolated_in_segments_no_matryoshka() {
        // ASP.NET Core (backend) + MAUI (frontend): оба на csharp (category
        // "both"), стороны дают side фреймворков (aspnetcore=backend,
        // maui=frontend). dotnet new webapi/maui выполняются ВНУТРИ своих
        // сегментов с -o . — проект ложится прямо в backend/ или frontend/,
        // без вложенной папки test16/test16.
        let mut ctx = context();
        ctx.languages = vec!["csharp".into()];
        ctx.frameworks = vec!["aspnetcore".into(), "maui".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Обе стороны: backend/ и frontend/ создаются движком
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "aspnetcore обязан получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "maui обязан получить frontend/: {mkdirs:?}"
        );

        // dotnet new webapi: работает в backend/ с -o . — без вложенной папки
        let asp = recipe
            .steps
            .iter()
            .find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "webapi обязан работать внутри ./backend"
                );
                assert!(args.contains(&"-o".to_string()) && args.contains(&".".to_string()),
                    "webapi обязан идти с -o . (проект прямо в backend/, без test16/test16): {args:?}");
                assert!(args.contains(&"-n".to_string()), "{args:?}");
            }
            _ => panic!("aspnet_new — Command"),
        }

        // dotnet new maui: работает в frontend/ с -o .
        let maui = recipe
            .steps
            .iter()
            .find(|s| s.id() == "maui_new")
            .expect("maui_new должен быть в плане");
        match maui {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "maui обязан работать внутри ./frontend"
                );
                assert!(args.contains(&"-o".to_string()) && args.contains(&".".to_string()),
                    "maui обязан идти с -o . (проект прямо в frontend/, без test16/test16): {args:?}");
            }
            _ => panic!("maui_new — Command"),
        }

        // Generic csharp-скаффолд (dotnet new console) подавлен обоими
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "dotnet_new"),
            "dotnet new console не нужен: каркасы создают aspnetcore/maui"
        );
    }

    #[test]
    fn django_sanitizes_name_and_isolates_in_backend() {
        // Django-фикс: django-admin startproject получает санитизированное
        // имя (дефис → подчёркивание — Python-пакет), а при обеих сторонах
        // работает в backend/ с "." — manage.py не появляется в корне.
        let mut ctx = context();
        ctx.project_name = Some("my-test-app".into());
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];

        let recipe = recipe_for(&ctx, "my-test-app").expect("recipe must build");

        let django = recipe
            .steps
            .iter()
            .find(|s| s.id() == "django_start")
            .expect("django_start должен быть в плане");
        match django {
            Step::Command {
                args, working_dir, ..
            } => {
                assert!(
                    args.contains(&"my_test_app".to_string()),
                    "имя проекта санитизируется (дефис → подчёркивание): {args:?}"
                );
                assert!(
                    args.contains(&".".to_string()),
                    "startproject создаёт проект в текущем каталоге: {args:?}"
                );
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)"
                );
            }
            _ => panic!("django_start — Command"),
        }
    }

    #[test]
    fn tauri_pipeline_scaffolds_frontend_init_and_patches_config() {
        // Integrated-раскладка: tauri (side="either" && scaffold="root") —
        // владелец корня. Rust остаётся в корне рядом с оболочкой, react
        // скаффолдится в frontend/; затем npx @tauri-apps/cli init в корне
        // (пути на ../frontend/dist) и Rust-патч src-tauri/tauri.conf.json.
        // create-tauri-app убран из пайплайна (его фронтенд-каркас в корне
        // был пустым без node_modules). Движок папки backend//frontend/ НЕ
        // предсоздаёт: frontend/ появляется из скаффолда react.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.backend_languages = vec!["rust".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // create-tauri-app создаёт Cargo.toml сам — language-скаффолд подавлен
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "cargo_init"),
            "cargo init не нужен: tauri создаёт каркас"
        );
        // Старый CLI-first шаг полностью убран
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_create"),
            "create-tauri-app убран из пайплайна"
        );
        // Мандат integrated: движок НЕ предсоздаёт backend//frontend/
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !mkdirs.iter().any(|d| d == "backend" || d == "frontend"),
            "integrated: движок не создаёт сегментные папки, их создают скаффолдеры: {mkdirs:?}"
        );
        // Компаньон react скаффолдится отдельно (не подавляется tauri)
        let react_scaffold = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match react_scaffold {
            Step::Generate {
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "react скаффолдится в frontend/: {generator_config}"
                );
                assert_eq!(on_error, &ErrorMode::Skip);
            }
            _ => panic!("vite_create — Generate"),
        }

        // Фронтенд-генератор выполняется ПЕРВЫМ — tauri init откладывается
        // в конец фазы скаффолдинга (движок откладывает tauri-шаги).
        let vite_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "vite_create")
            .unwrap();
        let init_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        assert!(vite_idx < init_idx, "фронтенд скаффолдится ДО tauri init");
        match &recipe.steps[init_idx] {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                // Способность generates_root_shell: tauri init раскладывает
                // shell в текущем каталоге, каталог проекта не создаёт.
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell"),
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                assert!(args.iter().any(|a| a == "init"), "{args:?}");
                assert!(
                    args.iter().any(|a| a == "--ci"),
                    "init должен быть неинтерактивным: {args:?}"
                );
                let dist_idx = args
                    .iter()
                    .position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "из корня путь на frontend/dist — ../frontend/dist: {args:?}"
                );
                assert_eq!(
                    args[args
                        .iter()
                        .position(|a| a == "--before-dev-command")
                        .unwrap()
                        + 1],
                    "cd frontend && npm run dev",
                    "{args:?}"
                );
                assert_eq!(
                    args[args
                        .iter()
                        .position(|a| a == "--before-build-command")
                        .unwrap()
                        + 1],
                    "cd frontend && npm run build",
                    "{args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне (integrated): {generator_config}"
                );
                // Пост-условие: tauri.conf.json И Cargo.toml обязаны появиться
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                    "{generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate scaffold"),
        }

        // Rust-патч tauri.conf.json — сразу после tauri init, в корне
        let patch_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "tauri_config_patch")
            .expect("tauri_config_patch должен быть в плане");
        assert!(init_idx < patch_idx, "патч конфига идёт после tauri init");
        match &recipe.steps[patch_idx] {
            Step::Generate {
                generator_id,
                generator_config,
                condition,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("identifier").and_then(|v| v.as_str()),
                    Some("com.myapp")
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dir")
                        .and_then(|v| v.as_str()),
                    Some("frontend")
                );
                assert_eq!(
                    generator_config.get("tauri_dir").and_then(|v| v.as_str()),
                    Some(""),
                    "конфиг живёт в src-tauri/ в корне: {generator_config}"
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dist")
                        .and_then(|v| v.as_str()),
                    Some("../frontend/dist"),
                    "frontendDist из корня — ../frontend/dist: {generator_config}"
                );
                // Патч выполняется только если tauri init создал конфиг
                // (пост-условие скаффолда) — вторичных ENOENT-ошибок нет.
                assert_eq!(
                    condition,
                    &Some(StepCondition::FileExists {
                        path: "src-tauri/tauri.conf.json".into()
                    }),
                    "патч конфига зависит от пост-условия tauri init"
                );
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // При компаньоне vite-vanilla-скаффолд и явный npm install tauri не нужны
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_web_scaffold"),
            "компаньон сам скаффолдит frontend/"
        );
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_web_install"),
            "компаньон: npm install делает финальная фаза"
        );

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("./frontend"));
            }
            _ => panic!("npm_install — Command"),
        }

        // Баг шаблонизатора: патч имени package.json работает в frontend/
        // (там лежит package.json), а не в корне
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "package.json лежит в frontend/"
                );
                assert!(
                    args[1].contains("j.name='myapp'"),
                    "патч должен писать project_name: {:?}",
                    args
                );
            }
            _ => panic!("react_pkg_name — Command"),
        }
    }

    #[test]
    fn tauri_without_companion_scaffolds_vanilla_frontend() {
        // tauri без react/vue/svelte: vite (vanilla-ts) скаффолдит frontend/,
        // npm install внутри frontend/, затем tauri init В КОРНЕ (integrated:
        // tauri — владелец корня, frontendDist — ../frontend/dist). Generic
        // js/ts-скаффолд в корне подавлен (его заглушки конфликтовали бы
        // с tauri-каркасом).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        assert!(
            !recipe.steps.iter().any(|s| s.id() == "package_json"),
            "generic js-скаффолд не нужен: фронтенд создаёт vite-vanilla"
        );
        let web = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_web_scaffold")
            .expect("tauri_web_scaffold должен быть в плане");
        match web {
            Step::Generate {
                generator_config, ..
            } => {
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("vanilla-ts"),
                    "typescript → vanilla-ts"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
            }
            _ => panic!("tauri_web_scaffold — Generate"),
        }

        // npm install — явный шаг внутри frontend/ (компаньона нет, финальная
        // фаза про tauri-фронтенд не знает)
        let install = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_web_install")
            .expect("tauri_web_install должен быть в плане");
        match install {
            Step::Command {
                working_dir,
                command,
                ..
            } => {
                assert_eq!(command, "npm");
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "install работает внутри frontend/ без join-сегмента: {working_dir:?}"
                );
            }
            _ => panic!("tauri_web_install — Command"),
        }

        // Фронтенд-генератор → npm install → tauri init → патч конфига
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("tauri_web_scaffold") < idx("tauri_web_install"),
            "скаффолд до install"
        );
        assert!(
            idx("tauri_web_install") < idx("tauri_init"),
            "install до tauri init"
        );

        match &recipe.steps[idx("tauri_init")] {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell"),
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                let dist_idx = args
                    .iter()
                    .position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "из корня путь на frontend/dist — ../frontend/dist: {args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне (integrated): {generator_config}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                    "{generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate scaffold"),
        }

        match &recipe.steps[idx("tauri_config_patch")] {
            Step::Generate {
                generator_id,
                generator_config,
                condition,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("tauri_dir").and_then(|v| v.as_str()),
                    Some("")
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dist")
                        .and_then(|v| v.as_str()),
                    Some("../frontend/dist")
                );
                assert_eq!(
                    condition,
                    &Some(StepCondition::FileExists {
                        path: "src-tauri/tauri.conf.json".into()
                    }),
                    "патч конфига зависит от пост-условия tauri init"
                );
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // Финальная фаза про tauri-фронтенд не знает: install уже сделан
        // явным шагом, новых npm_install в финале нет
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            0,
            "фронтенд tauri ставится явным шагом, финальных npm_install быть не должно"
        );

        // Патч имени package.json для tauri — в frontend/
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_pkg_name")
            .expect("tauri_pkg_name должен быть в плане");
        match patch {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "package.json лежит в frontend/"
                );
            }
            _ => panic!("tauri_pkg_name — Command"),
        }
    }

    #[test]
    fn tauri_with_svelte_companion_keeps_shell_in_root() {
        // tauri + svelte: integrated — оболочка остаётся в корне, svelte
        // (vite-компаньон) скаффолдится в frontend/ отдельным шагом, движок
        // не предсоздаёт сегментные папки. tauri init — в корне.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !mkdirs.iter().any(|d| d == "backend" || d == "frontend"),
            "integrated: сегментные папки не предсоздаются: {mkdirs:?}"
        );

        let svelte = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("svelte скаффолдится через vite_create");
        match svelte {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "svelte в frontend/: {generator_config}"
                );
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("svelte-ts"),
                    "{args:?}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        match init {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                let dist_idx = args.iter().position(|a| a == "--frontend-dist").unwrap();
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "tauri init в корне смотрит на ../frontend/dist: {args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне: {generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate"),
        }
    }

    #[test]
    fn layout_class_and_framework_placement_are_canonical() {
        // Каноническая раскладка: класс и каталоги фреймворков/языков —
        // единственное решение ProjectLayout::compute, и оно видно в
        // предпросмотре (LayoutSummary).
        let assert_placement =
            |ctx: &WizardContext, expected_class: &str, expected: &[(&str, &str)]| {
                let layout = ProjectLayout::compute(ctx);
                let summary = layout.to_summary(ctx);
                assert_eq!(
                    summary.class, expected_class,
                    "фреймворки: {:?}",
                    ctx.frameworks
                );
                assert_eq!(summary.generated_directories, layout.eager_dirs());
                for (fw, dir) in expected {
                    assert_eq!(
                        layout.framework_dir(fw).unwrap_or_else(|| ".".to_string()),
                        *dir,
                        "фреймворк {fw} должен лежать в {dir}"
                    );
                }
            };

        // nest + nextjs: обе стороны даже при единственном typescript → separated
        let mut ctx = context();
        ctx.frameworks = vec!["nest".into(), "nextjs".into()];
        assert_placement(
            &ctx,
            "separated",
            &[("nest", "backend"), ("nextjs", "frontend")],
        );

        // laravel + react: php + typescript → separated
        let mut ctx = context();
        ctx.languages = vec!["php".into(), "typescript".into()];
        ctx.frameworks = vec!["laravel".into(), "react".into()];
        assert_placement(
            &ctx,
            "separated",
            &[("laravel", "backend"), ("react", "frontend")],
        );

        // django + vue: python + typescript → separated, django работает в backend/
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.frameworks = vec!["django".into(), "vue".into()];
        assert_placement(
            &ctx,
            "separated",
            &[("django", "backend"), ("vue", "frontend")],
        );

        // tauri + svelte: connected — оболочка владеет корнем, svelte в frontend/
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let layout = ProjectLayout::compute(&ctx);
        let summary = layout.to_summary(&ctx);
        assert_eq!(summary.class, "connected");
        assert_eq!(summary.root_owner.as_deref(), Some("tauri"));
        assert_eq!(
            layout
                .framework_dir("tauri")
                .unwrap_or_else(|| ".".to_string()),
            "."
        );
        assert_eq!(
            layout
                .framework_dir("svelte")
                .unwrap_or_else(|| ".".to_string()),
            "frontend"
        );
        assert_eq!(
            layout
                .language_dir("rust")
                .unwrap_or_else(|| ".".to_string()),
            ".",
            "rust — язык оболочки, остаётся в корне"
        );
        assert!(
            layout.eager_dirs().is_empty(),
            "connected не предсоздаёт сегментные папки"
        );

        // zig-cli + flutter: zig → backend (по zig-cli), dart → frontend (по flutter)
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "flutter".into()];
        assert_placement(
            &ctx,
            "separated",
            &[("zig-cli", "backend"), ("flutter", "frontend")],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(
            layout
                .language_dir("zig")
                .unwrap_or_else(|| ".".to_string()),
            "backend"
        );
        assert_eq!(
            layout
                .language_dir("dart")
                .unwrap_or_else(|| ".".to_string()),
            "frontend"
        );

        // gin + solidjs: go → backend, solidjs → frontend
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["gin".into(), "solidjs".into()];
        assert_placement(
            &ctx,
            "separated",
            &[("gin", "backend"), ("solidjs", "frontend")],
        );

        // fastapi один: backend-only, всё в корне, папки не создаются
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        assert_placement(&ctx, "backend-only", &[("fastapi", ".")]);

        // nextjs один: separated — движок сегментирует обе стороны,
        // scaffold-генератор уходит в frontend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        assert_placement(&ctx, "separated", &[("nextjs", "frontend")]);

        // electron + django: неинтегрированная клиентская оболочка + REST
        // API-бэкенд → shell-client-api (клиент в frontend/, API в backend/)
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["electron".into(), "django".into()];
        assert_placement(
            &ctx,
            "shell-client-api",
            &[("electron", "frontend"), ("django", "backend")],
        );

        // electron один: оболочка без явного REST API-бэкенда — сегментация
        // всё равно включена (клиент в frontend/, движок держит backend/)
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["electron".into()];
        assert_placement(&ctx, "shell-client-api", &[("electron", "frontend")]);

        // Пустой стек (нет ни языков, ни фреймворков) → custom
        let mut ctx = context();
        ctx.languages = vec![];
        ctx.frameworks = vec![];
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "custom");
        assert!(layout.eager_dirs().is_empty());
    }

    #[test]
    fn readme_step_uses_context_locale() {
        // README локализуется на бэкенде по context.readme_locale: выбор
        // языка — данные, а не ветвление генератора.
        let mut ctx = context();
        ctx.readme_locale = Some("ru".into());

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        match find_step(&recipe, "readme") {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("## Обзор"), "{content}");
                assert!(content.contains("Сгенерировано StackPilot"), "{content}");
                assert!(!content.contains("readme."), "raw key leaked: {content}");
            }
            _ => panic!("readme — WriteFile"),
        }
    }

    #[test]
    fn docker_and_readme_follow_the_canonical_layout() {
        // Split (laravel + react + postgresql): Dockerfile Рё .dockerignore
        // живут в backend/, docker-compose собирает app из backend/. README
        // показывает каноническую структуру.
        let mut ctx = context();
        ctx.languages = vec!["php".into(), "typescript".into()];
        ctx.frameworks = vec!["laravel".into(), "react".into()];
        ctx.tools = vec!["postgresql".into()];
        ctx.docker = true;

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let dockerfile = recipe.steps.iter().find(|s| s.id() == "dockerfile");
        if let Some(Step::WriteFile { path, .. }) = dockerfile {
            assert_eq!(
                path, "backend/Dockerfile",
                "Dockerfile собирается из backend/"
            );
        }
        let ignore = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_ignore")
            .unwrap();
        match ignore {
            Step::WriteFile { path, .. } => assert_eq!(path, "backend/.dockerignore"),
            _ => panic!("docker_ignore — WriteFile"),
        }
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .unwrap();
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("build: backend/"),
                    "compose собирает app из backend/: {content}"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
        }
        let readme = recipe.steps.iter().find(|s| s.id() == "readme").unwrap();
        match readme {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("backend"),
                    "README описывает сегменты: {content}"
                );
                assert!(
                    content.contains("frontend"),
                    "README описывает сегменты: {content}"
                );
            }
            _ => panic!("readme — WriteFile"),
        }

        // BackendOnly (fastapi + postgresql): Dockerfile и compose — в корне
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["postgresql".into()];
        ctx.docker = true;

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let ignore = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_ignore")
            .unwrap();
        match ignore {
            Step::WriteFile { path, .. } => {
                assert_eq!(path, ".dockerignore", "backend-only: всё в корне")
            }
            _ => panic!("docker_ignore — WriteFile"),
        }
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .unwrap();
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("build: ."),
                    "backend-only: сборка из корня: {content}"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
        }
    }

    #[test]
    fn root_scaffold_forced_to_subdir_still_runs_first() {
        // Strict Subdir Mandate: python (backend) + typescript (frontend) —
        // обе стороны, поэтому django (scaffold="root" в wizard_tree.json)
        // принудительно работает как subdir: бэкенд в backend/, фронтенд в
        // frontend/. Порядок фаз сохраняется: 1. Root CLI (django) →
        // 2. Subdir (react) → 3. Инструменты (prisma) → 4. Конфиги
        // (docker-compose).
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        ctx.tools = vec!["prisma".into(), "postgresql".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "django (root→subdir) получает backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "react получает frontend/: {mkdirs:?}"
        );

        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("django_start") < idx("vite_create"),
            "root CLI идёт до subdir-скаффолда"
        );
        assert!(
            idx("vite_create") < idx("prisma_init"),
            "subdir-скаффолд идёт до инструментов"
        );
        assert!(
            idx("prisma_init") < idx("docker_compose"),
            "инструменты идут до конфигов"
        );

        // django-admin startproject работает в backend/, а не в корне
        match &recipe.steps[idx("django_start")] {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)"
                );
            }
            _ => panic!("django_start — Command"),
        }

        // react (subdir) НЕ подавляется django (у django нет компаньонов)
        assert!(
            recipe.steps.iter().any(|s| s.id() == "vite_create"),
            "django не поглощает react — vite-скаффолд остаётся"
        );

        // НЕТ матрёшки: ScaffoldGenerator выполняет create-vite ВНУТРИ
        // frontend/ с "." — а не создаёт вложенную папку myapp/ в корне django
        let vite = &recipe.steps[idx("vite_create")];
        match vite {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "vite скаффолдится в frontend/: {generator_config}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("./frontend"));
            }
            _ => panic!("npm_install — Command"),
        }
        assert!(
            idx("npm_install_0") > idx("readme"),
            "npm install — в финальной фазе, после шаблонизации"
        );
    }

    #[test]
    fn nest_plus_nextjs_decoupled_twin_isolates_backend_and_frontend() {
        // Strict Decoupled Twin Rule: nest (backend, scaffold="root" в JSON) +
        // nextjs (frontend) — обе стороны (nest = side=backend, nextjs =
        // side=frontend даже при единственном языке typescript), поэтому
        // nest принудительно работает как subdir: бэкенд целиком в backend/
        // (nest-cli.json, tsconfig.build.json, package.json — только там),
        // фронтенд — в frontend/. В корне нет package.json и node_modules —
        // только оркестрационные файлы движка.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "nextjs".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком ДО запуска CLI
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "nest обязан получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "nextjs обязан получить frontend/: {mkdirs:?}"
        );
        let create_idx = |p: &str| {
            recipe
                .steps
                .iter()
                .position(|s| matches!(s, Step::CreateDirectory { path, .. } if path == p))
                .unwrap_or_else(|| panic!("{p}/ должен создаваться движком"))
        };
        let nest_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        assert!(
            create_idx("backend") < nest_idx,
            "backend/ создаётся до запуска nest"
        );
        assert!(
            create_idx("frontend")
                < recipe
                    .steps
                    .iter()
                    .position(|s| s.id() == "nextjs_create")
                    .unwrap(),
            "frontend/ создаётся до запуска nextjs"
        );

        // nest: "." + --yes + --skip-install + --skip-git, работает ВНУТРИ backend/
        let nest = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        match nest {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    args.get(0).map(String::as_str),
                    Some("--yes"),
                    "--yes сразу после npx (prompt «Ok to proceed?»): {args:?}"
                );
                assert_eq!(args.get(3).map(String::as_str), Some("."), "{args:?}");
                assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
                assert!(
                    args.contains(&"--skip-git".to_string()),
                    "git инициализирует движок: {args:?}"
                );
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "nest обязан работать в backend/, а не в корне (Decoupled Twin)"
                );
            }
            _ => panic!("nest_new — Command"),
        }

        // nextjs: ScaffoldGenerator выполняет create-next-app ВНУТРИ frontend/
        // (--skip-install — зависимости в финальной фазе)
        let next = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Generate {
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
                assert_eq!(on_error, &ErrorMode::Skip);
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert!(
                    args.iter().any(|a| a == "--skip-install"),
                    "create-next-app должен идти с --skip-install: {args:?}"
                );
            }
            _ => panic!("nextjs_create — Generate"),
        }

        // npm install ровно 2 раза: backend (nest) + frontend (nextjs) —
        // корневой install НЕ появляется (в корне нет package.json)
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            2,
            "install-шагов должно быть 2: {installs:?}"
        );
        match (&installs[0], &installs[1]) {
            (
                Step::Command {
                    working_dir: w0, ..
                },
                Step::Command {
                    working_dir: w1, ..
                },
            ) => {
                assert_eq!(
                    w0.as_deref(),
                    Some("./backend"),
                    "nest-бэкенд ставится первым"
                );
                assert_eq!(
                    w1.as_deref(),
                    Some("./frontend"),
                    "nextjs-фронтенд ставится вторым"
                );
            }
            _ => panic!("npm_install — Command"),
        }

        // В корне нет generic js-скаффолда: package.json пишется только
        // nest'ом (в backend/) и nextjs (в frontend/)
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "package_json"),
            "package.json не должен создаваться в корне"
        );
    }

    #[test]
    fn templating_runs_last_and_overwrites_cli_files() {
        // P4: README.md/.gitignore/docker-compose пишутся ПОСЛЕ всех
        // CLI-фреймворков и с overwrite=true — иначе create-next-app/nest new
        // перезаписывают/удаляют наш шаблон.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        ctx.git_init = true;
        ctx.docker = true;
        // postgres нужен, чтобы docker-compose сгенерировался (без сервисов
        // compose-шага в плане нет — это отдельный инвариант)
        ctx.tools = vec!["postgresql".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };

        // Шаблонизация ПОСЛЕ скаффолдинга...
        assert!(
            idx("nextjs_create") < idx("readme"),
            "README пишется после CLI"
        );
        assert!(
            idx("nextjs_create") < idx("gitignore"),
            ".gitignore пишется после CLI"
        );
        assert!(
            idx("nextjs_create") < idx("docker_compose"),
            "docker-compose пишется после CLI"
        );
        // ...но до git add/commit и npm install
        assert!(
            idx("git_init") < idx("readme"),
            "git init до шаблонизации — README в коммите"
        );
        assert!(
            idx("readme") < idx("git_add"),
            "README до стартового коммита"
        );
        assert!(
            idx("readme") < idx("npm_install_0"),
            "npm install — после шаблонизации"
        );

        for id in ["readme", "gitignore", "docker_compose"] {
            match &recipe.steps[idx(id)] {
                Step::WriteFile { overwrite, .. } => {
                    assert!(*overwrite, "{id} должен перезаписывать файлы CLI");
                }
                _ => panic!("{id} — WriteFile"),
            }
        }
    }

    #[test]
    fn spring_boot_generation_goes_through_generator() {
        // P3: Spring Boot НЕ качается curl'ом в project.zip (ошибка
        // Initializr писалась в файл и падала на распаковке с «Error opening
        // archive») — вместо этого Step::Generate с генератором
        // "spring-boot", который проверяет HTTP-статус и останавливает
        // пайплайн (Abort) с реальной причиной.
        let mut ctx = context();
        ctx.languages = vec!["java".into()];
        ctx.frameworks = vec!["spring-boot".into()];
        ctx.tools = vec!["postgresql".into(), "redis".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let gen = recipe
            .steps
            .iter()
            .find(|s| s.id() == "spring_init")
            .expect("spring_init должен быть в плане");
        match gen {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "spring-boot");
                assert_eq!(on_error, &ErrorMode::Abort);
                let deps = generator_config
                    .get("dependencies")
                    .and_then(|d| d.as_str());
                assert_eq!(
                    deps,
                    Some("web,data-jpa,postgresql,data-redis"),
                    "зависимости собираются из tools"
                );
                let name = generator_config
                    .get("project_name")
                    .and_then(|n| n.as_str());
                assert_eq!(name, Some("myapp"));
            }
            _ => panic!("spring_init — Generate"),
        }
        // Никаких curl/unzip шагов с project.zip
        assert!(!recipe.steps.iter().any(|s| s.id() == "unzip_spring"));
        assert!(!recipe.steps.iter().any(|s| s.id() == "cleanup_zip"));
    }

    #[test]
    fn prisma_init_is_non_interactive() {
        // Prisma не должен спрашивать «how to set up your database»:
        // провайдер передаётся флагом, npx — с --yes / CI=1 (executor).
        // Версия пинится на 6.x: в Prisma 8+ флаги --datasource-provider и
        // --no-skills удалены, а 6.x совпадает с декларируемой в
        // package.json зависимостью (^6.1.0).
        let mut ctx = context();
        ctx.tools = vec!["prisma".into(), "postgresql".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let step = recipe
            .steps
            .iter()
            .find(|s| s.id() == "prisma_init")
            .expect("prisma_init должен быть в плане");
        match step {
            Step::Command { command, args, .. } => {
                assert_eq!(command, "npx");
                assert!(args.contains(&"--yes".to_string()), "{args:?}");
                assert!(
                    args.contains(&"prisma@6".to_string()),
                    "prisma init обязан быть запинен на 6.x (не latest): {args:?}"
                );
                let provider = args
                    .iter()
                    .position(|a| a == "--datasource-provider")
                    .map(|i| args[i + 1].as_str());
                assert_eq!(provider, Some("postgresql"), "{args:?}");
                // У 6.x нет флага --no-skills (и в 8+ его тоже нет) —
                // агентные артефакты чистит Rust-генератор prisma_cleanup.
                assert!(
                    !args.contains(&"--no-skills".to_string()),
                    "prisma@6 не знает флага --no-skills: {args:?}"
                );
            }
            _ => panic!("prisma_init — Command"),
        }

        // Подстраховка: Rust-генератор принудительно чистит агентные папки
        let cleanup = recipe
            .steps
            .iter()
            .find(|s| s.id() == "prisma_cleanup")
            .expect("prisma_cleanup должен быть в плане");
        match cleanup {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "fs-cleanup");
                assert_eq!(on_error, &ErrorMode::Skip);
                let paths = generator_config.get("paths").and_then(|p| p.as_array());
                assert!(paths.is_some_and(|p| p.iter().any(|v| v == ".agents")));
            }
            _ => panic!("prisma_cleanup — Generate"),
        }
        let cleanup_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "prisma_cleanup")
            .unwrap();
        let init_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "prisma_init")
            .unwrap();
        assert!(init_idx < cleanup_idx, "очистка идёт после init");
    }

    #[test]
    fn segmented_vite_package_name_patched_to_project_name() {
        // Баг шаблонизатора: create-vite frontend → package.json name
        // = "frontend". Движок добавляет пост-шаг, переписывающий name
        // на project_name из WizardContext.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["react".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "патч работает в папке скаффолда"
                );
                assert!(
                    args[1].contains("j.name='myapp'"),
                    "name берётся из project_name, а не из папки frontend: {:?}",
                    args
                );
            }
            _ => panic!("react_pkg_name — Command"),
        }
    }

    #[test]
    fn python_union_requirements_contains_all_deps() {
        // ЕДИНЫЙ requirements.txt python-скаффолда собирает зависимости ВСЕХ
        // python-фреймворков и инструментов (union), а не перезаписывается
        // последним пишущим: fastapi + flask + aiogram + alembic в одном файле,
        // пер-фреймворковые requirements-шаги удалены.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into(), "flask".into(), "aiogram".into()];
        ctx.tools = vec!["alembic".into(), "sqlalchemy".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(recipe.steps.iter().any(|s| s.id() == "pyproject_toml"));
        let reqs = recipe
            .steps
            .iter()
            .find(|s| s.id() == "requirements_txt")
            .unwrap();
        match reqs {
            Step::WriteFile {
                path,
                content,
                overwrite,
                ..
            } => {
                assert_eq!(path, "requirements.txt");
                assert!(
                    !*overwrite,
                    "union-файл создаётся один раз (overwrite=false)"
                );
                assert!(content.contains("fastapi[standard]"), "fastapi: {content}");
                assert!(content.contains("uvicorn"), "uvicorn: {content}");
                assert!(content.contains("flask"), "flask: {content}");
                assert!(content.contains("aiogram"), "aiogram: {content}");
                assert!(content.contains("alembic"), "alembic: {content}");
                assert!(content.contains("sqlalchemy"), "sqlalchemy: {content}");
                assert!(
                    !recipe.steps.iter().any(|s| s.id() == "fastapi_requirements"
                        || s.id() == "flask_requirements"
                        || s.id() == "aiogram_requirements"),
                    "пер-фреймворковые requirements-шаги удалены"
                );
            }
            _ => panic!("requirements_txt — WriteFile"),
        }
    }

    #[test]
    fn inplace_framework_entry_files_overwrite() {
        // Inplace-фреймворки (scaffold=None) перезаписывают entry-файлы,
        // иначе их шаги молча скипались на файлах language-скаффолда.
        for (fw, entry_ids) in [
            ("express", vec!["express_index", "express_package"]),
            ("gin", vec!["gin_main"]),
            ("clap", vec!["clap_main"]),
            ("axum", vec!["axum_main"]),
            ("flask", vec!["flask_app"]),
        ] {
            let steps = steps_for_framework(
                fw,
                "C:\\dev\\myapp",
                "myapp",
                &context(),
                &ProjectLayout::compute(&context()),
            );
            for id in &entry_ids {
                let step = steps
                    .iter()
                    .find(|s| s.id() == id.to_string())
                    .unwrap_or_else(|| panic!("{fw}: шаг {id} должен существовать"));
                match step {
                    Step::WriteFile { overwrite, .. } => {
                        assert!(*overwrite, "{fw}: шаг {id} должен перезаписываться")
                    }
                    _ => panic!("{fw}: {id} — WriteFile"),
                }
            }
        }
    }

    #[test]
    fn plain_frontend_language_keeps_scaffold_next_to_backend_framework() {
        // python-бэкенд (fastapi) + plain TypeScript-фронтенд (без фронтенд-
        // фреймворка): generic-каркас TS должен остаться и попасть в frontend/,
        // иначе фронтенд-сегмент остаётся пустым.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let ts_index = recipe
            .steps
            .iter()
            .find(|s| s.id() == "ts_src_index")
            .expect("generic TS-скаффолд фронтенда должен быть в плане");
        match ts_index {
            Step::WriteFile { path, .. } => assert_eq!(path, "frontend/src/index.ts"),
            other => panic!("ts_src_index — WriteFile, получили {:?}", other.id()),
        }
    }

    #[test]
    fn backend_js_framework_does_not_suppress_frontend_plain_ts() {
        // express (JS-бэкенд-фреймворк, подавляет JS/TS-скаффолд) НЕ должен
        // глушить generic-каркас TypeScript, назначенного на ФРОНТЕНД:
        // это другой сегмент (frontend/), express строит только backend/.
        let mut ctx = context();
        ctx.languages = vec!["javascript".into(), "typescript".into()];
        ctx.backend_languages = vec!["javascript".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["express".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let ts_index = recipe
            .steps
            .iter()
            .find(|s| s.id() == "ts_src_index")
            .expect("frontend TS-скаффолд не должен подавляться backend-express");
        match ts_index {
            Step::WriteFile { path, .. } => assert_eq!(path, "frontend/src/index.ts"),
            other => panic!("ts_src_index — WriteFile, получили {:?}", other.id()),
        }
    }

    #[test]
    fn complex_dual_side_stack_segments_backend_and_frontend() {
        // python + typescript на бэкенд-стороне + react (frontend-фреймворк,
        // side=frontend) — обе стороны, поэтому сегментация ВКЛЮЧЕНА:
        // fastapi живёт в backend/, vite-скаффолд — в frontend/. Корневые
        // файлы оркестрации (dags/, docker-compose, .env.example) — в корне.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into(), "typescript".into()];
        ctx.frontend_languages = vec![];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["airflow".into(), "postgresql".into()];
        ctx.docker = true;

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // react (frontend-фреймворк) даёт обе стороны → backend/ и frontend/
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "fastapi должен получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "react должен получить frontend/: {mkdirs:?}"
        );

        // react: ScaffoldGenerator кладёт vite-проект в frontend/
        let vite = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match vite {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("npx")
                );
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(0).and_then(|v| v.as_str()),
                    Some("create-vite@latest")
                );
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("react-ts"),
                    "typescript → react-ts шаблон: {args:?}"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "vite-проект живёт в frontend/: {generator_config}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install выполняется РОВНО один раз в финальной фазе пайплайна —
        // ВНУТРИ frontend/
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(working_dir.as_deref(), Some("./frontend"));
                assert_eq!(args, &vec!["install".to_string()]);
            }
            _ => panic!("npm_install — Command"),
        }

        // fastapi (inplace): entry-файлы в backend/
        let main = recipe
            .steps
            .iter()
            .find(|s| s.id() == "fastapi_main")
            .expect("fastapi_main должен быть в плане");
        match main {
            Step::WriteFile { path, .. } => assert_eq!(path, "backend/src/main.py"),
            _ => panic!("fastapi_main — WriteFile"),
        }

        // airflow: dags/ директория + пример DAG — в корне (оркестрация)
        assert!(
            recipe
                .steps
                .iter()
                .any(|s| matches!(s, Step::CreateDirectory { path, .. } if path == "dags")),
            "airflow должен создать dags/"
        );
        assert!(
            recipe.steps.iter().any(|s| s.id() == "airflow_example_dag"),
            "airflow должен создать example_dag.py"
        );

        // docker-compose: airflow + postgres
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .expect("docker_compose должен быть в плане");
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("airflow"),
                    "compose должен включать airflow"
                );
                assert!(
                    content.contains("postgres"),
                    "compose должен включать postgres"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
        }

        // .env.example: переменные airflow и postgres
        let env = recipe
            .steps
            .iter()
            .find(|s| s.id() == "env_example")
            .expect("env_example должен быть в плане");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("AIRFLOW__CORE__EXECUTOR"));
                assert!(content.contains("POSTGRES_USER"));
            }
            _ => panic!("env_example — WriteFile"),
        }
    }

    #[test]
    fn guard_legal_pairs_produce_no_duplicates() {
        // Легальные связки разводятся движком по разным путям — guard молчит.
        let cases: Vec<(Vec<&str>, Vec<&str>)> = vec![
            (vec!["gin", "cobra"], vec!["go"]),
            (vec!["axum", "clap"], vec!["rust"]),
            (vec!["zap", "zig-cli"], vec!["zig"]),
            (vec!["fastapi", "aiogram"], vec!["python"]),
            (vec!["electron", "react"], vec!["typescript"]),
            (vec!["tauri", "svelte"], vec!["rust", "typescript"]),
        ];
        for (fws, langs) in cases {
            let mut ctx = context();
            ctx.frameworks = fws.into_iter().map(String::from).collect();
            ctx.languages = langs.into_iter().map(String::from).collect();
            let issues = duplicate_framework_write_paths(&ctx);
            assert!(issues.is_empty(), "ошибки для легальной связки: {issues:?}");
        }
    }

    #[test]
    fn guard_colliding_frameworks_are_reported() {
        // express и fastify пишут src/index.js и package.json в один и тот
        // же каталог (inplace, без сегментации): второй пишущий молча
        // скипнется — сгенерированный проект сломается.
        let mut ctx = context();
        ctx.languages = vec!["javascript".into()];
        ctx.frameworks = vec!["express".into(), "fastify".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(
            issues.iter().any(|i| i.contains("src/index.js")),
            "ожидался конфликт по src/index.js: {issues:?}"
        );
    }

    #[test]
    fn guard_colliding_scaffolds_in_same_dir_are_reported() {
        // Два фронтенд-скаффолдера (react+vue) оба раскладывают каркас в
        // frontend/ — staging-merge второго CLI сломает каркас первого.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["react".into(), "vue".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("frontend") && i.contains("каталог")),
            "ожидался конфликт по каталогу frontend/: {issues:?}"
        );
    }

    #[test]
    fn guard_colliding_root_owners_are_reported() {
        // django и nest — оба root-скаффолдеры: в одно-сторонней раскладке
        // (только backend, без фронтенд-языка) обе генерации идут в корень,
        // вторая сломает первую.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into(), "nest".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("корень") && i.contains("django") && i.contains("nest")),
            "ожидался конфликт владения корнем: {issues:?}"
        );
    }

    #[test]
    fn guard_colliding_php_scaffolds_in_same_dir_are_reported() {
        // laravel и symfony — оба backend-скаффолдеры: в монолите обе
        // генерации сливаются в один каталог (корень) — staging-merge
        // второго CLI сломает каркас первого (composer.json пересекается).
        let mut ctx = context();
        ctx.languages = vec!["php".into()];
        ctx.frameworks = vec!["laravel".into(), "symfony".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(
            issues.iter().any(|i| i.contains("каталог")),
            "ожидался конфликт по каталогу: {issues:?}"
        );
    }

    #[test]
    fn guard_split_stack_with_tauri_react_is_clean() {
        // tauri (either, язык rust → backend/) + react (frontend/) в
        // mono-репозитории не пересекаются по путям.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.backend_languages = vec!["rust".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(issues.is_empty(), "ложные срабатывания: {issues:?}");
    }

    #[test]
    fn every_requires_docker_tool_lands_in_compose() {
        // Каждый инструмент мастера с requires_docker: true обязан давать
        // сервис в collect_docker_services: локально он не ставится
        // (toolchain не требует его для проверки окружения), а
        // docker-compose.yaml — единственный способ его развернуть.
        let raw = include_str!("../knowledge/wizard_tree.json");
        let tree: serde_json::Value =
            serde_json::from_str(raw).expect("wizard_tree.json должен быть корректным JSON");
        let docker_tools: Vec<String> = tree
            .get("tools")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter(|t| {
                        t.get("requires_docker")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
                    .filter_map(|t| t.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        assert!(
            !docker_tools.is_empty(),
            "в мастере должны быть docker-инструменты"
        );
        for tool in &docker_tools {
            let services = content::collect_docker_services(std::slice::from_ref(tool), 3000);
            assert!(
                !services.is_empty(),
                "requires_docker-инструмент {tool} не создаёт сервис в docker-compose"
            );
        }
    }

    #[test]
    fn alembic_init_runs_after_venv_setup_via_venv_binary() {
        // Проблема: `alembic init` вызывался ДО создания venv и pip install —
        // системная команда не находилась. Порядок обязан быть таким:
        // py_venv_create → py_pip_install → alembic_init, и сам alembic
        // вызывается строго через бинарь виртуального окружения.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["alembic".into(), "sqlalchemy".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };

        assert!(
            idx("py_venv_create") < idx("py_pip_install"),
            "venv создаётся до pip install"
        );
        assert!(
            idx("py_pip_install") < idx("alembic_init"),
            "pip install обязан идти ДО alembic init"
        );

        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command { command, .. } => {
                assert!(
                    command.contains("venv"),
                    "alembic должен вызываться из venv, а не системно: {command}"
                );
            }
            _ => panic!("alembic_init — Command"),
        }

        // alembic ставится ЧЕРЕЗ единый манифест (requirements.txt), а не
        // отдельным pip-вызовом: манифест обязан содержать alembic, а
        // py_pip_install — единственный pip install рецепта.
        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        let pip_args = cmd_args(pip);
        assert!(
            pip_args.iter().any(|a| a == "-r"),
            "pip ставит РОВНО из манифеста (-r): {pip_args:?}"
        );
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic не ставится отдельно — он в манифесте: {pip_args:?}"
        );
        let requirements = recipe
            .steps
            .iter()
            .find(|s| s.id() == "requirements_txt")
            .expect("requirements_txt должен быть в плане");
        match requirements {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("alembic"),
                    "манифест обязан содержать alembic: {content}"
                );
            }
            _ => panic!("requirements_txt — WriteFile"),
        }
    }

    #[test]
    fn alembic_venv_lives_inside_backend_segment_in_monorepo() {
        // Моно-репозиторий (python backend + typescript frontend): python-код
        // и requirements.txt лежат в backend/, поэтому venv создаётся ВНУТРИ
        // backend/, а не в корне проекта. Путь к бинарю формируется как
        // backend/venv/Scripts/alembic.exe (Win) или backend/venv/bin/alembic.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["alembic".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let venv_create = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create должен быть в плане");
        let venv_path = venv_path_of(venv_create);
        assert!(
            venv_path.ends_with("backend/venv") || venv_path.ends_with("backend\\venv"),
            "venv создаётся внутри backend/: {venv_path}"
        );

        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        match pip {
            Step::Command { command, args, .. } => {
                assert!(
                    command.contains("backend"),
                    "pip вызывается из venv внутри backend/: {command}"
                );
                assert!(
                    args.iter().any(|a| a.ends_with("backend/requirements.txt")
                        || a.ends_with("backend\\requirements.txt")),
                    "pip читает requirements.txt из backend/: {args:?}"
                );
            }
            _ => panic!("py_pip_install — Command"),
        }

        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command {
                command, on_error, ..
            } => {
                assert!(
                    command.contains("backend") && command.contains("venv"),
                    "alembic вызывается из backend/venv: {command}"
                );
                assert_eq!(
                    on_error,
                    &ErrorMode::Abort,
                    "alembic_init не должен проваливаться молча (Skip)"
                );
            }
            _ => panic!("alembic_init — Command"),
        }
    }

    #[test]
    fn python_venv_bin_resolves_inside_segment() {
        // Корень проекта: <project>\venv\Scripts\alembic.exe (Win) /
        // <project>/venv/bin/alembic.
        let root_bin = python_venv_bin("C:\\dev\\myapp", ".", "alembic");
        assert!(
            root_bin.contains("venv") && !root_bin.contains("backend"),
            "корневой venv без сегмента: {root_bin}"
        );
        assert!(
            root_bin.ends_with("alembic.exe") || root_bin.ends_with("/alembic"),
            "имя бинаря на конце: {root_bin}"
        );
        assert!(
            root_bin.starts_with("C:\\dev\\myapp"),
            "путь АБСОЛЮТНЫЙ (не зависит от рабочего каталога): {root_bin}"
        );
        // Моно-репозиторий: <project>\backend\venv\Scripts\alembic.exe (Win)
        // / <project>/backend/venv/bin/alembic.
        let seg_bin = python_venv_bin("C:\\dev\\myapp", "backend", "alembic");
        assert!(
            seg_bin.contains("backend") && seg_bin.contains("venv"),
            "бинарь внутри backend/venv: {seg_bin}"
        );
        assert!(
            seg_bin.starts_with("C:\\dev\\myapp"),
            "путь АБСОЛЮТНЫЙ: {seg_bin}"
        );
        assert!(
            seg_bin.ends_with("alembic.exe") || seg_bin.ends_with("/alembic"),
            "имя бинаря на конце: {seg_bin}"
        );
    }

    #[test]
    fn python_dir_natural_key_orders_numerically() {
        // Python313 > Python311 > Python310 (числовое, а не лексикографическое).
        assert!(python_dir_natural_key("Python313") > python_dir_natural_key("Python311"));
        assert!(python_dir_natural_key("Python311") > python_dir_natural_key("Python310"));
        assert_eq!(python_dir_natural_key("Python313"), python_dir_natural_key("Python313"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn python_from_path_skips_windows_store_aliases() {
        // PATH, состоящий ТОЛЬКО из каталога Store-заглушек, не считается
        // установкой Python: python.exe там — заглушка (код 9009), а не
        // интерпретатор. Резолвер обязан вернуть None, а не путь к заглушке.
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let apps = std::path::Path::new(&local).join("Microsoft").join("WindowsApps");
            if apps.is_dir() {
                let path = format!("{};C:\\Windows\\System32", apps.to_string_lossy());
                assert!(
                    python_from_path(&path).is_none(),
                    "Store-заглушка не может быть интерпретатором: {path}"
                );
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn python_from_path_prefers_real_interpreter_over_earlier_alias() {
        // Реальный python.exe в PATH ПОСЛЕ каталога WindowsApps обязан
        // выигрывать: заглушка пропускается, поиск продолжается.
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_py_resolve_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("python.exe"), b"stub").unwrap();
        let path = format!("{};C:\\Windows\\System32", dir.to_string_lossy());
        let found = python_from_path(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            found.as_deref() == Some(&format!("{}\\python.exe", dir.to_string_lossy())),
            "реальный python из PATH: {found:?}"
        );
    }

    #[test]
    fn venv_steps_are_only_created_for_python_with_alembic() {
        // Каждый Python-проект получает изолированное окружение (venv
        // создаётся даже без alembic — иначе fastapi-проекты ставили
        // зависимости в глобальный Python). Без Python venv-шагов нет.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["sqlalchemy".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().any(|s| s.id() == "py_venv_create"),
            "Python-проект без alembic всё равно получает venv"
        );
        // Без alembic pip НЕ ставит alembic явно (только -r requirements.txt)
        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        let pip_args = cmd_args(pip);
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic не ставится, если инструмент не выбран: {pip_args:?}"
        );

        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().all(|s| s.id() != "py_venv_create"),
            "venv не нужен без Python"
        );
    }

    #[test]
    fn qt_webengine_stack_generates_webengine_files() {
        // qt-webengine + react: react (side=frontend) + cpp (backend) — обе
        // стороны, поэтому qt (side=either, язык cpp → backend/) живёт в
        // backend/. main.cpp обязан содержать QWebEngineView-бойлерплейт,
        // CMakeLists.txt — WebEngineWidgets (не заглушки).
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "react".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let main_cpp = recipe
            .steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "backend/src/main.cpp"))
            .unwrap_or_else(|| panic!("qt должен писать backend/src/main.cpp"));
        match main_cpp {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("#include <QWebEngineView>"), "{content}");
                assert!(content.contains("QWebEngineView view;"), "{content}");
                assert!(content.contains("qrc:/web/index.html"), "{content}");
                assert!(
                    content.contains("QApplication app(argc, argv)"),
                    "нужен <QApplication> бойлерплейт: {content}"
                );
            }
            _ => panic!("qt_main — WriteFile"),
        }

        let cmake = recipe
            .steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "backend/CMakeLists.txt"))
            .unwrap_or_else(|| panic!("qt должен писать backend/CMakeLists.txt"));
        match cmake {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("find_package(Qt6 REQUIRED COMPONENTS WebEngineWidgets)"),
                    "{content}"
                );
                assert!(
                    content.contains("target_link_libraries(myapp Qt6::WebEngineWidgets)"),
                    "{content}"
                );
                assert!(content.contains("qt_add_resources"), "{content}");
            }
            _ => panic!("qt_cmake — WriteFile"),
        }
    }

    #[test]
    fn laravel_and_symfony_use_composer_not_npm() {
        // P-баг: @laravel/installer падал с «npm error 404 Not Found», а
        // бинарь symfony не установлен. PHP-фреймворки создаются через
        // `composer create-project ... --no-interaction --prefer-dist`:
        // запускается глобальный `composer` из PATH ИЛИ `php <АБСОЛЮТНЫЙ
        // путь к composer.phar>` (composer_launch — Toolchain store
        // %LOCALAPPDATA%\StackPilot\tools\php, %APPDATA%\Composer...).
        // Относительный `composer.phar` запрещён: php ищет его в рабочем
        // каталоге CLI и падает с «Could not open input file: composer.phar».
        for (fw_id, step_id, package) in [
            ("laravel", "laravel_new", "laravel/laravel"),
            ("symfony", "symfony_new", "symfony/skeleton"),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["php".into()];
            ctx.frameworks = vec![fw_id.into()];

            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            assert!(
                !recipe
                    .steps
                    .iter()
                    .any(|s| s.id().contains("installer") || s.id().contains("@laravel")),
                "npm-путь @laravel/installer не должен использоваться"
            );

            let step = recipe
                .steps
                .iter()
                .find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::Generate {
                    generator_id,
                    generator_config,
                    on_error,
                    ..
                } => {
                    assert_eq!(generator_id, "scaffold");
                    // Composer-скаффолд — обязательный шаг: провал
                    // инициализации фреймворка не маскируется скипом.
                    assert_eq!(on_error, &ErrorMode::Abort);
                    let command = generator_config
                        .get("command")
                        .and_then(|v| v.as_str())
                        .expect("command обязан быть");
                    assert!(
                        command == "composer" || command == "php",
                        "composer запускается как composer или php, а не npm-клиент: {generator_config}"
                    );
                    let args = generator_config
                        .get("args")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let name_arg = args
                        .iter()
                        .position(|a| a.as_str() == Some("__TARGET__"))
                        .expect("__TARGET__ обязан быть в args")
                        as usize;

                    // create-project идёт сразу после префикса (путь к phar
                    // в режиме php, ничего в режиме composer), за ним —
                    // пакет, а плейсхолдер target стоит на name_arg.
                    let cp_idx = args
                        .iter()
                        .position(|a| a.as_str() == Some("create-project"))
                        .expect("create-project обязан быть в args");
                    if command == "php" {
                        let phar_idx = args
                            .iter()
                            .position(|a| a.as_str().map_or(false, |s| s.ends_with("composer.phar")))
                            .expect("php-режим: в args обязан быть абсолютный путь к composer.phar: {args:?}");
                        let phar = args[phar_idx].as_str().unwrap_or_default();
                        assert!(
                            phar.ends_with("composer.phar"),
                            "php-режим: первым аргументом — АБСОЛЮТНЫЙ путь к composer.phar: {args:?}"
                        );
                        assert_ne!(
                            phar, "composer.phar",
                            "относительный composer.phar запрещён (рабочий каталог CLI ≠ каталог phar): {args:?}"
                        );
                        assert_eq!(
                            cp_idx,
                            phar_idx + 1,
                            "php [flags] <phar> create-project ...: {args:?}"
                        );
                    } else {
                        assert_eq!(cp_idx, 0, "composer create-project ...: {args:?}");
                    }
                    assert_eq!(cp_idx + 2, name_arg,
                        "имя проекта — аргумент сразу после пакета create-project: {generator_config}");
                    assert_eq!(
                        args.get(cp_idx + 1).and_then(|v| v.as_str()),
                        Some(package),
                        "пакет сразу после create-project: {generator_config}"
                    );
                    assert!(args.iter().any(|a| a == "--no-interaction"), "{args:?}");
                    assert!(args.iter().any(|a| a == "--prefer-source"), "{args:?}");
                    assert_eq!(
                        generator_config.get("target_dir").and_then(|v| v.as_str()),
                        Some("."),
                        "в монолите PHP-фреймворк живёт в корне"
                    );
                    // Способность: composer create-project создаёт именованную
                    // папку; temp+move по умолчанию (composer не принимает "."
                    // в непустом каталоге). Пост-условие — НАСТОЯЩИЙ composer.json
                    // (generic-фолбэк не считается успешным каркасом).
                    assert_eq!(
                        generator_config.get("capability").and_then(|v| v.as_str()),
                        Some("creates_named_directory"),
                        "{generator_config}"
                    );
                    let expected = generator_config
                        .get("expected_outputs")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    assert_eq!(
                        expected,
                        vec!["composer.json"],
                        "composer обязан создать composer.json (не package.json): {generator_config}"
                    );
                }
                _ => panic!("{step_id} — Generate"),
            }
        }
    }

    // ==================== runtime conditions (пост-условия скаффолда) ======

    #[test]
    fn runtime_condition_checks_actual_filesystem_state() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_cond_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), "{}").unwrap();

        let exists = |path: &str| Some(StepCondition::FileExists { path: path.into() });
        let not_exists = |path: &str| Some(StepCondition::FileNotExists { path: path.into() });

        assert!(runtime_condition(exists("package.json").as_ref(), &dir));
        assert!(!runtime_condition(exists("missing.txt").as_ref(), &dir));
        assert!(!runtime_condition(
            not_exists("package.json").as_ref(),
            &dir
        ));
        assert!(runtime_condition(not_exists("missing.txt").as_ref(), &dir));
        assert!(
            !runtime_condition(exists("frontend/package.json").as_ref(), &dir),
            "вложенные пути проверяются тоже"
        );

        // контекстные условия на рантайме не фильтруются
        assert!(runtime_condition(Some(&StepCondition::Always), &dir));
        assert!(runtime_condition(None, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============ безопасные пути и политики идемпотентности ==============

    #[test]
    fn runtime_condition_backslashes_and_unsafe_paths() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_cond2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::write(dir.join("backend").join("package.json"), "{}").unwrap();

        let exists = |path: &str| Some(StepCondition::FileExists { path: path.into() });
        let not_exists = |path: &str| Some(StepCondition::FileNotExists { path: path.into() });

        // Обратные слеши нормализуются; путь условия root-relative.
        assert!(runtime_condition(
            exists("backend\\package.json").as_ref(),
            &dir
        ));
        assert!(runtime_condition(
            exists("backend/package.json").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            not_exists("backend/package.json").as_ref(),
            &dir
        ));

        // Выход за корень / абсолютные пути: условие НЕ выполнено (шаг
        // пропускается, а не пишет мимо проекта).
        assert!(!runtime_condition(exists("../outside.txt").as_ref(), &dir));
        assert!(!runtime_condition(
            not_exists("../outside.txt").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            exists("C:\\Windows\\win.ini").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            not_exists("C:\\Windows\\win.ini").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(exists("/etc/hosts").as_ref(), &dir));
        assert!(!runtime_condition(not_exists("/etc/hosts").as_ref(), &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plan_with_write_file(
        path: &str,
        content: &str,
        policy: Option<FilePolicy>,
        dir_name: &str,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_policy_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::WriteFile {
                id: "w".into(),
                label: "Write".into(),
                description: String::new(),
                path: path.into(),
                content: content.into(),
                overwrite: false,
                policy,
                condition: None,
                on_error: ErrorMode::Skip,
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn execute_skip_if_exists_keeps_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::SkipIfExists), "skip");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_overwrite_policy_replaces_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::Overwrite), "over");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(matches!(
            result.step_results[0].status,
            StepStatus::Success { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_create_only_skips_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::CreateOnly),
            "create_only",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_merge_json_keeps_existing_keys_and_merges_deeply() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "conf.json",
            r#"{"b": 2, "nested": {"y": 2}}"#,
            Some(FilePolicy::MergeJson),
            "merge",
        );
        std::fs::write(dir.join("conf.json"), r#"{"a": 1, "nested": {"x": 1}}"#).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(matches!(
            result.step_results[0].status,
            StepStatus::Success { .. }
        ));
        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("conf.json")).unwrap()).unwrap();
        assert_eq!(merged["a"], 1, "существующий ключ сохраняется");
        assert_eq!(merged["b"], 2, "недостающий ключ добавляется");
        assert_eq!(
            merged["nested"]["x"], 1,
            "вложенный существующий ключ сохраняется"
        );
        assert_eq!(
            merged["nested"]["y"], 2,
            "вложенный недостающий ключ добавляется"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_merge_json_fails_on_non_json_existing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("conf.json", "{}", Some(FilePolicy::MergeJson), "merge_bad");
        std::fs::write(dir.join("conf.json"), "not json at all").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("not valid JSON")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("conf.json")).unwrap(),
            "not json at all"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_fail_on_mismatch_noop_when_identical() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "same",
            Some(FilePolicy::FailOnMismatch),
            "fom_same",
        );
        std::fs::write(dir.join("app.json"), "same").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Success { message } if message.contains("unchanged")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "same"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_fail_on_mismatch_fails_when_different() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::FailOnMismatch),
            "fom_diff",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("fail_on_mismatch")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_write_escaping_path_fails_without_creating() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("../escape.txt", "x", Some(FilePolicy::Overwrite), "escape");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("escapes")),
            "{:?}",
            result.step_results[0].status
        );
        assert!(
            !dir.parent().unwrap().join("escape.txt").exists(),
            "файл не пишется мимо корня"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_write_absolute_path_fails() {
        let engine = DefaultRecipeEngine::new();
        let abs = std::env::temp_dir().join("stackpilot_abs_outside.txt");
        let _ = std::fs::remove_file(&abs);
        let (plan, dir) = plan_with_write_file(
            &abs.to_string_lossy(),
            "x",
            Some(FilePolicy::Overwrite),
            "abs",
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("escapes")),
            "{:?}",
            result.step_results[0].status
        );
        assert!(!abs.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_rerun_second_run_skips_write_step() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::SkipIfExists), "rerun");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx1, _rx1) = tokio::sync::mpsc::channel(16);
        let first = engine.execute(plan.clone(), tx1, no_cancel()).await;
        assert!(matches!(
            first.step_results[0].status,
            StepStatus::Skipped { .. }
        ));
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        let second = engine.execute(plan.clone(), tx2, no_cancel()).await;
        assert!(
            matches!(second.step_results[0].status, StepStatus::Skipped { .. }),
            "повторный запуск снова пропускает существующий файл"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_preview_marks_existing_target_as_not_executing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::SkipIfExists),
            "prev_existing",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let preview = engine.preview(&plan);
        assert_eq!(preview.total_steps, 1);
        let p = &preview.step_previews[0];
        assert!(p.existing_file, "fs-статус: файл существует");
        assert_eq!(p.file_policy, Some(FilePolicy::SkipIfExists));
        assert!(
            !p.will_execute,
            "skip_if_exists над существующим файлом — шаг не выполнится"
        );
        assert!(
            p.skip_reason.is_some(),
            "причина пропуска показывается в превью"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_preview_writes_when_target_missing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::SkipIfExists),
            "prev_missing",
        );
        let preview = engine.preview(&plan);
        assert!(!preview.step_previews[0].existing_file);
        assert!(preview.step_previews[0].will_execute);
        assert_eq!(preview.will_execute_count, 1);
        assert_eq!(preview.will_skip_count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scaffold_generate_step(extra_config: serde_json::Value) -> Step {
        // CLI создаёт временную папку (плейсхолдер → temp_name) и падает
        // с exit 1 — сценарий «CLI умер после создания temp-каталога».
        let args = if cfg!(target_os = "windows") {
            vec![
                "/c".to_string(),
                "mkdir".to_string(),
                SCAFFOLD_TARGET.to_string(),
                "&&".to_string(),
                "exit".to_string(),
                "1".to_string(),
            ]
        } else {
            vec![
                "-c".to_string(),
                "mkdir".to_string(),
                "-p".to_string(),
                SCAFFOLD_TARGET.to_string(),
                "&&".to_string(),
                "exit".to_string(),
                "1".to_string(),
            ]
        };
        let mut config = serde_json::json!({
            "command": if cfg!(target_os = "windows") { "cmd" } else { "sh" },
            "args": args,
            "capability": "creates_named_directory",
            "target_dir": ".",
            "temp_dir_allowed": true,
            "expected_outputs": vec!["package.json"],
        });
        if let serde_json::Value::Object(map) = &mut config {
            if let serde_json::Value::Object(extra) = extra_config {
                for (k, v) in extra {
                    map.insert(k, v);
                }
            }
        }
        Step::Generate {
            id: "scaffold_test".into(),
            label: "Scaffold".into(),
            description: String::new(),
            generator_id: "scaffold".into(),
            generator_config: config,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    }

    fn plan_with_generate(step: Step, dir_name: &str) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_scaffold_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![step],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn execute_scaffold_skip_if_exists_when_outputs_present() {
        let engine = DefaultRecipeEngine::new();
        let mut step = scaffold_generate_step(serde_json::json!({}));
        match &mut step {
            Step::Generate { policy, .. } => *policy = Some(FilePolicy::SkipIfExists),
            _ => unreachable!(),
        }
        let (plan, dir) = plan_with_generate(step, "skip_done");
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        // CLI (exit 1) не запускался: expected_outputs уже на месте.
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Success { message } if message.contains("skipped by policy")),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_scaffold_runs_cli_when_outputs_missing() {
        let engine = DefaultRecipeEngine::new();
        let step = scaffold_generate_step(serde_json::json!({}));
        let (plan, dir) = plan_with_generate(step, "run_cli");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Failed { .. }),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_failed_scaffold_leaves_no_temp_dir() {
        let engine = DefaultRecipeEngine::new();
        let step = scaffold_generate_step(serde_json::json!({}));
        let (plan, dir) = plan_with_generate(step, "temp_cleanup");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Failed { .. }),
            "{:?}",
            result.step_results[0].status
        );
        // CLI создал временную папку и упал — temp+move обязан её удалить.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("temp_"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "после провала не должно оставаться temp_-папок: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_skips_finalize_steps_after_skip_mode_failure() {
        // Провал Skip-режима не останавливает пайплайн, но git add/commit
        // и README после него не выполняются: коммитить сломанный проект
        // (и перезаписывать его README) вредно.
        let engine = DefaultRecipeEngine::new();
        let failing = scaffold_generate_step(serde_json::json!({}));
        let git_add = Step::Command {
            id: "git_add".into(),
            label: "Stage all files".into(),
            description: String::new(),
            command: "git".into(),
            args: vec!["add".into(), ".".into()],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let readme = Step::WriteFile {
            id: "readme".into(),
            label: "Create README.md".into(),
            description: String::new(),
            path: "README.md".into(),
            content: "# test".into(),
            overwrite: true,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        };
        let (mut plan, dir) = plan_with_generate(failing, "finalize_skip");
        plan.steps.push(git_add);
        plan.steps.push(readme);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(matches!(
            result.step_results[0].status,
            StepStatus::Failed { .. }
        ));
        for r in &result.step_results[1..] {
            assert!(
                matches!(&r.status, StepStatus::Skipped { reason } if reason.contains("Previous step failed")),
                "финализационный шаг обязан быть пропущен с причиной: {:?}",
                r.status
            );
        }
        assert!(
            !std::fs::read_dir(&dir).unwrap().any(|e| {
                e.ok()
                    .is_some_and(|e| e.file_name().to_string_lossy() == "README.md")
            }),
            "README не перезаписывается после провала"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Мок-шина процессов: записывает все запуски, отвечает успехом или
    /// Exit-ошибкой по заданным командам. Позволяет тестировать исполнение
    /// (порядок шагов, семантику провалов) без реальных CLI.
    #[derive(Default)]
    struct MockCommandRunner {
        calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
        fail_commands: Vec<String>,
        /// Обычный провал (пустые хвосты — без сетевых маркеров).
        network_fail_commands: Vec<String>,
        /// Сколько раз сетевая команда падает до успешного ответа
        /// (0/не задано — падает всегда).
        fail_times: std::sync::Mutex<std::collections::HashMap<String, u32>>,
    }

    #[async_trait::async_trait]
    impl CommandRunner for MockCommandRunner {
        async fn run(
            &self,
            spec: ProcessSpec,
            _sink: Option<&ExecutionEventSink>,
        ) -> Result<ProcessOutput, ProcessExecutionError> {
            self.calls
                .lock()
                .unwrap()
                .push((spec.command.clone(), spec.args.clone()));
            let is_net = self.network_fail_commands.contains(&spec.command);
            let is_plain = self.fail_commands.contains(&spec.command);
            let fail_now = if is_plain {
                true
            } else if is_net {
                match self.fail_times.lock().unwrap().get_mut(&spec.command) {
                    Some(remaining) if *remaining > 0 => {
                        *remaining -= 1;
                        true
                    }
                    Some(_) => false,
                    None => true,
                }
            } else {
                false
            };
            if fail_now {
                let stderr_tail = if is_net {
                    "Error: Failed to query available provider packages\n\
                     could not connect to registry.terraform.io: failed to request discovery document:\n\
                     GET https://registry.terraform.io/.well-known/terraform.json giving up after 4 attempts: context deadline exceeded"
                        .to_string()
                } else {
                    String::new()
                };
                return Err(ProcessExecutionError {
                    kind: ProcessErrorKind::Exit {
                        code: "1".to_string(),
                    },
                    command: spec.command.clone(),
                    args: spec.args.clone(),
                    working_dir: spec.working_dir.clone().unwrap_or_default(),
                    stdout_tail: "mocked".to_string(),
                    stderr_tail,
                });
            }
            Ok(ProcessOutput {
                stdout_tail: "mocked".to_string(),
                stderr_tail: String::new(),
                duration_ms: 0,
            })
        }
    }

    #[tokio::test]
    async fn execute_routes_commands_through_command_runner() {
        // CLI-шаги выполняются через CommandRunner: мок записывает вызовы и
        // отвечает по правилам — порядок и семантика провалов проверяются без
        // реальных процессов.
        let mock = Arc::new(MockCommandRunner {
            fail_commands: vec!["boom".to_string()],
            ..Default::default()
        });
        let engine = DefaultRecipeEngine::with_executor(Arc::new(
            executor::StepExecutor::with_command_runner(mock.clone()),
        ));
        let cmd = |id: &str, command: &str| Step::Command {
            id: id.to_string(),
            label: id.to_string(),
            description: String::new(),
            command: command.to_string(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: None,
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dir = std::env::temp_dir().join(format!("stackpilot_mock_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![cmd("first", "echo"), cmd("second", "boom")],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "custom".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        let statuses: Vec<&str> = result
            .step_results
            .iter()
            .map(|r| match &r.status {
                StepStatus::Success { .. } => "ok",
                StepStatus::Failed { .. } => "failed",
                _ => "other",
            })
            .collect();
        assert_eq!(statuses, vec!["ok", "failed"], "{:?}", result.step_results);
        assert!(
            matches!(result.overall, OverallStatus::PartialFailure { .. }),
            "{:?}",
            result.overall
        );
        let calls = mock.calls.lock().unwrap();
        let commands: Vec<&str> = calls.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(commands, vec!["echo", "boom"], "все CLI идут через мок");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Единичный Command-шаг с заданной командой/аргументами и режимом ошибки.
    fn plan_with_command(
        dir_name: &str,
        command: &str,
        arg: &str,
        on_error: ErrorMode,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_net_{dir_name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::Command {
                id: "net_step".into(),
                label: "Network step".into(),
                description: String::new(),
                command: command.into(),
                args: vec![arg.into()],
                working_dir: None,
                env: None,
                timeout_secs: Some(10),
                condition: None,
                on_error,
                interactive: vec![],
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "custom".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn execute_retries_network_command_then_succeeds() {
        // Сетевой сбой (недоступный registry.terraform.io) транзиентен по
        // своему характеру: движок повторяет команду, а не верит первому
        // провалу. Первые 2 попытки падают, третья успешна.
        let mock = Arc::new(MockCommandRunner {
            network_fail_commands: vec!["terraform".to_string()],
            fail_times: std::sync::Mutex::new(
                [("terraform".to_string(), 2u32)].into_iter().collect(),
            ),
            ..Default::default()
        });
        let engine = DefaultRecipeEngine::with_executor(Arc::new(
            executor::StepExecutor::with_command_runner(mock.clone()),
        ));
        let (plan, dir) = plan_with_command("retry_ok", "terraform", "init", ErrorMode::Abort);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Success { .. }),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            mock.calls.lock().unwrap().len(),
            network::NETWORK_MAX_ATTEMPTS as usize,
            "сетевая команда повторяется до успеха"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_network_failure_does_not_abort_and_gives_advice() {
        // Устойчивый сетевой сбой (все повторы исчерпаны) НЕ останавливает
        // генерацию даже для Abort-шага: это проблема окружения (реестр
        // недоступен), а не проекта. Итог — PartialFailure + понятный совет.
        let mock = Arc::new(MockCommandRunner {
            network_fail_commands: vec!["terraform".to_string()],
            ..Default::default()
        });
        let engine = DefaultRecipeEngine::with_executor(Arc::new(
            executor::StepExecutor::with_command_runner(mock.clone()),
        ));
        let (plan, dir) =
            plan_with_command("net_persistent", "terraform", "init", ErrorMode::Abort);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.overall, OverallStatus::PartialFailure { .. }),
            "{:?}",
            result.overall
        );
        let error = match &result.step_results[0].status {
            StepStatus::Failed { error } => error,
            other => panic!("шаг обязан провалиться: {:?}", other),
        };
        assert!(
            error.contains("registry.terraform.io") && error.contains("VPN"),
            "в ошибке обязан быть совет: {error}"
        );
        assert_eq!(
            mock.calls.lock().unwrap().len(),
            network::NETWORK_MAX_ATTEMPTS as usize,
            "повторы исчерпаны до совета"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_aborts_on_non_network_failure_even_for_network_command() {
        // Та же сетевая команда (npm), но сбой НЕ сетевой (пустые хвосты,
        // без маркеров): повтор не нужен, Abort-семантика рецепта сохраняется.
        let mock = Arc::new(MockCommandRunner {
            fail_commands: vec!["npm".to_string()],
            ..Default::default()
        });
        let engine = DefaultRecipeEngine::with_executor(Arc::new(
            executor::StepExecutor::with_command_runner(mock.clone()),
        ));
        let (plan, dir) = plan_with_command("net_code_err", "npm", "install", ErrorMode::Abort);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.overall, OverallStatus::Aborted { .. }),
            "{:?}",
            result.overall
        );
        assert_eq!(
            mock.calls.lock().unwrap().len(),
            1,
            "не-сетевой сбой не повторяется"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_aborts_on_non_network_command_failure() {
        // Не сетевая команда (echo) в Abort-режиме по-прежнему останавливает
        // пайплайн — «генерация продолжается после сетевого сбоя» не должна
        // маскировать реальные ошибки рецепта.
        let mock = Arc::new(MockCommandRunner {
            fail_commands: vec!["boom".to_string()],
            ..Default::default()
        });
        let engine = DefaultRecipeEngine::with_executor(Arc::new(
            executor::StepExecutor::with_command_runner(mock.clone()),
        ));
        let (plan, dir) = plan_with_command("abort_plain", "boom", "x", ErrorMode::Abort);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.overall, OverallStatus::Aborted { .. }),
            "{:?}",
            result.overall
        );
        assert_eq!(
            mock.calls.lock().unwrap().len(),
            1,
            "не-сетевая команда не повторяется"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_marks_scaffold_with_existing_outputs_as_not_executing() {
        let engine = DefaultRecipeEngine::new();
        let mut step = scaffold_generate_step(serde_json::json!({}));
        match &mut step {
            Step::Generate { policy, .. } => *policy = Some(FilePolicy::SkipIfExists),
            _ => unreachable!(),
        }
        let (plan, dir) = plan_with_generate(step, "prev_scaffold");
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let preview = engine.preview(&plan);
        let p = &preview.step_previews[0];
        assert!(
            p.existing_file,
            "post-условия на месте — existing_file true"
        );
        assert!(
            !p.will_execute,
            "scaffold с готовыми выходами не выполняется"
        );
        assert_eq!(preview.will_execute_count, 0);
        assert_eq!(preview.will_skip_count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============ идемпотентность git-шагов и политики скаффолдов =========

    #[test]
    fn git_cleanup_nested_works_on_current_platform() {
        // Windows — историческая PowerShell-команда; Unix — find, который
        // НЕ трогает корневой .git (mindepth 2) и удаляет вложенные
        // репозитории на любой глубине (прежний PowerShell-шаг на Linux
        // молча скипался, и git add . падал на вложенных .git).
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_git_init(&ctx, "C:\\dev\\myapp");
        let cleanup = steps
            .iter()
            .find(|s| s.id() == "git_cleanup_nested")
            .expect("git_cleanup_nested в плане");
        match cleanup {
            Step::Command {
                command,
                args,
                on_error,
                ..
            } => {
                if cfg!(target_os = "windows") {
                    assert!(command.starts_with("Get-ChildItem"), "{command}");
                } else {
                    assert_eq!(command, "find");
                    assert!(
                        args.windows(2).any(|w| w[0] == "-mindepth" && w[1] == "2"),
                        "корневой .git исключается mindepth: {args:?}"
                    );
                    assert!(args.iter().any(|a| a == ".git"), "{args:?}");
                    assert!(args.iter().any(|a| a == "-prune"), "{args:?}");
                    assert!(
                        args.windows(3).any(|w| w[0] == "rm" && w[1] == "-rf" && w[2] == "{}"),
                        "{args:?}"
                    );
                }
                assert_eq!(
                    on_error,
                    &ErrorMode::Skip,
                    "отсутствие git/find не валит генерацию"
                );
            }
            other => panic!("git_cleanup_nested — Command: {:?}", other.id()),
        }
    }

    /// Вложенные .git удаляются, корневой репозиторий проекта не трогается.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn git_cleanup_nested_removes_only_inner_repositories() {
        let engine = DefaultRecipeEngine::new();
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_git_cleanup_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("frontend/.git")).unwrap();
        std::fs::create_dir_all(dir.join("backend/nested/.git")).unwrap();

        let dir_str = dir.to_string_lossy().into_owned();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![git_cleanup_nested_step(&dir_str)],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "custom".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Success { .. }),
            "шаг очистки обязан выполниться: {:?}",
            result.step_results[0].status
        );
        assert!(dir.join(".git").is_dir(), "корневой репозиторий не трогаем");
        assert!(
            !dir.join("frontend/.git").exists(),
            "вложенный .git во frontend/ удалён"
        );
        assert!(
            !dir.join("backend/nested/.git").exists(),
            "вложенный .git на глубине 3 удалён"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_init_gated_on_head_file() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_git_init(&ctx, "C:\\dev\\myapp");
        let init = steps
            .iter()
            .find(|s| s.id() == "git_init")
            .expect("git_init в плане");
        assert_eq!(
            init.condition(),
            Some(&StepCondition::FileNotExists {
                path: ".git/HEAD".into()
            }),
            "повторный запуск не переинициализирует репозиторий"
        );
    }

    #[test]
    fn git_commit_is_idempotent_noop_when_nothing_staged() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_finalize(&ctx, &[], "C:\\dev\\myapp", "myapp");
        let commit = steps
            .iter()
            .find(|s| s.id() == "git_commit")
            .expect("git_commit в плане");
        match commit {
            Step::Command { command, .. } => {
                assert!(
                    command.contains("git diff --cached --quiet"),
                    "коммит только при изменениях: {command}"
                );
                assert!(
                    command.contains("exit 0"),
                    "нечего коммитить — не ошибка: {command}"
                );
            }
            other => panic!("ожидался Command: {:?}", other.id()),
        }
    }

    #[test]
    fn safe_scaffolds_carry_skip_if_exists_policy() {
        let ctx = context();
        let layout = ProjectLayout::compute(&ctx);
        let zig = steps_for_language("zig", "myapp", "C:\\dev\\myapp", &ctx);
        let zig_step = zig
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init в плане");
        assert_eq!(zig_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let flutter = steps_for_framework("flutter", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let flutter_step = flutter
            .iter()
            .find(|s| s.id() == "flutter_create")
            .expect("flutter_create в плане");
        assert_eq!(flutter_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let laravel = steps_for_framework("laravel", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let laravel_step = laravel
            .iter()
            .find(|s| s.id() == "laravel_new")
            .expect("laravel_new в плане");
        assert_eq!(laravel_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let symfony = steps_for_framework("symfony", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let symfony_step = symfony
            .iter()
            .find(|s| s.id() == "symfony_new")
            .expect("symfony_new в плане");
        assert_eq!(symfony_step.file_policy(), Some(FilePolicy::SkipIfExists));
    }

    #[test]
    fn npm_install_gated_on_node_modules() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_finalize(&ctx, &["frontend".to_string()], "C:\\dev\\myapp", "myapp");
        let install = steps
            .iter()
            .find(|s| s.id() == "npm_install_0")
            .expect("npm_install_0 в плане");
        assert_eq!(
            install.condition(),
            Some(&StepCondition::FileNotExists {
                path: "frontend/node_modules".into()
            }),
            "повторный запуск не переустанавливает зависимости"
        );
    }

    #[test]
    fn language_inits_carry_rerun_guards() {
        let ctx = context();
        let cases: &[(&str, &str)] = &[
            ("rust", "Cargo.toml"),
            ("go", "go.mod"),
            ("java", "pom.xml"),
            ("elixir", "mix.exs"),
            ("gleam", "gleam.toml"),
            ("typescript", "tsconfig.json"),
        ];
        for (lang, marker) in cases {
            let steps = steps_for_language(lang, "myapp", "C:\\dev\\myapp", &ctx);
            assert!(
                steps.iter().any(|s| s.condition()
                    == Some(&StepCondition::FileNotExists {
                        path: marker.to_string()
                    })),
                "{lang}: шаг обязан иметь FileNotExists {marker}"
            );
        }
    }

    #[test]
    fn dart_create_gated_on_subdir_pubspec() {
        let ctx = context();
        let steps = steps_for_language("dart", "my-app", "C:\\dev\\myapp", &ctx);
        assert!(
            steps.iter().any(|s| s.condition()
                == Some(&StepCondition::FileNotExists {
                    path: "my_app/pubspec.yaml".into()
                })),
            "dart create кладёт пакет в подпапку — гейт на {}/pubspec.yaml",
            "my_app"
        );
    }

    #[test]
    fn django_venv_gated_on_marker() {
        // Django (как и любой Python-фреймворк) использует ЕДИНСТВЕННЫЙ
        // канонический venv проекта: отдельного django-venv не существует.
        // Windows сохраняет гейт по маркеру venv/pyvenv.cfg; на Unix создание
        // идемпотентно внутри генератора python-venv (гейт по маркеру
        // недопустим: после падения `python -m venv` маркер уже существует).
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let create = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create в плане");
        if cfg!(target_os = "windows") {
            assert_eq!(
                create.condition(),
                Some(&StepCondition::FileNotExists {
                    path: "venv/pyvenv.cfg".into()
                }),
                "повторный запуск не пересоздаёт venv"
            );
        } else {
            match create {
                Step::Generate {
                    generator_id,
                    condition,
                    ..
                } => {
                    assert_eq!(generator_id, "python-venv");
                    assert!(condition.is_none());
                }
                other => panic!("ожидался Generate python-venv: {:?}", other.id()),
            }
        }
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "django_venv_create"),
            "отдельного django-venv быть не должно — venv единственный"
        );
    }

    fn plan_with_single_command(
        condition: Option<StepCondition>,
        dir_name: &str,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_engine_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (command, args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec!["/d".into(), "/c".into(), "exit 0".into()],
            )
        } else {
            ("true".to_string(), vec![])
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::Command {
                id: "dependent".into(),
                label: "Dependent step".into(),
                description: "Depends on scaffold output".into(),
                command,
                args,
                working_dir: None,
                env: None,
                timeout_secs: Some(10),
                condition,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn engine_skips_dependent_step_when_postcondition_missing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_single_command(
            Some(StepCondition::FileExists {
                path: "package.json".into(),
            }),
            "skip",
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert_eq!(result.step_results.len(), 1);
        assert!(
            matches!(
                &result.step_results[0].status,
                StepStatus::Skipped { reason } if reason.contains("package.json")
            ),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn engine_runs_step_when_postcondition_satisfied() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_single_command(
            Some(StepCondition::FileExists {
                path: "package.json".into(),
            }),
            "run",
        );
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        drop(rx);
        assert_eq!(result.step_results.len(), 1);
        assert!(
            !matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Explicit step dependencies ====================

    /// План из двух шагов: [prereq] + [dependent]; prereq выполняет команду
    /// из `exit_code` (0 = успех, 1 = провал). Оба шага без условий
    /// (runtime-условия не вмешиваются в зависимостные сценарии).
    fn plan_with_dependency(
        dir_name: &str,
        prereq_exit: &str,
        prereq_condition: Option<StepCondition>,
        dependent_condition: Option<StepCondition>,
        dep: StepDependency,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_deps_{dir_name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (cmd, ok_args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec!["/d".into(), "/c".into(), "exit 0".into()],
            )
        } else {
            ("true".to_string(), vec![])
        };
        // Команда провала — отдельно от успешной: на Unix `true <arg>`
        // завершается с кодом 0, поэтому провалом должен быть вызов `false`.
        let (fail_cmd, fail_args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec!["/d".into(), "/c".into(), "exit 1".into()],
            )
        } else {
            ("false".to_string(), vec![])
        };
        let prereq = Step::Command {
            id: "prereq".into(),
            label: "Prerequisite step".into(),
            description: String::new(),
            command: if prereq_exit == "0" {
                cmd.clone()
            } else {
                fail_cmd
            },
            args: if prereq_exit == "0" {
                ok_args.clone()
            } else {
                fail_args
            },
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: prereq_condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dependent = Step::Command {
            id: "dependent".into(),
            label: "Dependent step".into(),
            description: String::new(),
            command: cmd,
            args: ok_args,
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: dependent_condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![prereq, dependent],
            dependencies: vec![dep],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn dependency_skips_dependent_when_prereq_failed() {
        // Провал предшественника (например, nest_new / py_venv_create /
        // go_mod_init / vite_create) → зависимый шаг пропускается С ТОЧНОЙ
        // причиной, а не выполняет команду и не падает с вторичной ошибкой.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "fail",
            "1",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert_eq!(result.step_results.len(), 2);
        assert!(matches!(
            &result.step_results[0].status,
            StepStatus::Failed { .. }
        ));
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("prerequisite 'prereq'")
                    && reason.contains("failed")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_skips_dependent_when_prereq_skipped() {
        // Предшественник пропущен по runtime-условию и файлового
        // пост-условия нет → зависимый шаг пропускается.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "skip_no_file",
            "0",
            Some(StepCondition::FileExists {
                path: "never_created.txt".into(),
            }),
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(matches!(
            &result.step_results[0].status,
            StepStatus::Skipped { .. }
        ));
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("prerequisite 'prereq' was skipped")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_proceeds_when_skipped_prereq_left_postcondition() {
        // Django-ранний venv: py_venv_create пропущен (маркер
        // venv/pyvenv.cfg уже создан django_venv_create), но py_pip_upgrade
        // ОБЯЗАН выполниться — пост-условие предшественника на месте.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "skip_has_file",
            "0",
            Some(StepCondition::FileNotExists {
                path: "venv/pyvenv.cfg".into(),
            }),
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: "venv/pyvenv.cfg".into(),
            },
        );
        std::fs::create_dir_all(dir.join("venv")).unwrap();
        std::fs::write(dir.join("venv/pyvenv.cfg"), "").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Skipped { .. }),
            "prereq: {:?}",
            result.step_results[0].status
        );
        assert!(
            matches!(&result.step_results[1].status, StepStatus::Success { .. }),
            "dependent обязан выполниться: {:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_marks_prereq_failed_when_postcondition_missing() {
        // Предшественник «успешно» завершился, но обещанного файла нет —
        // он ретроактивно помечается Failed (путь + рабочая директория),
        // зависимый шаг пропускается.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "missing_post",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: "dist/index.html".into(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(
                &result.step_results[0].status,
                StepStatus::Failed { error } if error.contains("dist/index.html")
                    && error.contains(&dir.to_string_lossy().into_owned())
            ),
            "prereq обязан стать Failed с путём и cwd: {:?}",
            result.step_results[0].status
        );
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("expected output 'dist/index.html'")
                    && reason.contains("'prereq'")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_skips_step_when_required_file_does_not_exist() {
        // Чистое файловое предусловие (без предшественника): файл обязан
        // быть на момент запуска — отсутствует → пропуск с причиной.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "pure_file",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: String::new(),
                expects_file: "frontend/package.json".into(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("required file 'frontend/package.json' does not exist")
            ),
            "{:?}",
            result.step_results[1].status
        );
        // файл появляется → шаг выполняется
        let (plan2, dir2) = plan_with_dependency(
            "pure_file_ok",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: String::new(),
                expects_file: "frontend/package.json".into(),
            },
        );
        std::fs::create_dir_all(dir2.join("frontend")).unwrap();
        std::fs::write(dir2.join("frontend/package.json"), "{}").unwrap();
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        let result2 = engine.execute(plan2, tx2, no_cancel()).await;
        assert!(
            matches!(&result2.step_results[1].status, StepStatus::Success { .. }),
            "{:?}",
            result2.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn topo_order_sorts_reverse_declared_dependency_chain() {
        // Рецепт декларирует шаги в ОБРАТНОМ порядке: сборка (C) → установка
        // (B) → каркас (A). Топосортировка выстраивает цепочку правильно.
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("C"), cmd("B"), cmd("A")];
        let ordered = topo_order_steps(&steps, &[dep("C", "B"), dep("B", "A")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B", "C"]);
    }

    #[test]
    fn topo_order_keeps_declaration_order_for_independent_steps() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        // Без зависимостей — порядок декларации сохраняется полностью.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
        // Y зависит от X — независимый Z остаётся на своём месте.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[dep("Y", "X")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
        // Z зависит от X — Y (независимый) не переставляется.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[dep("Z", "X")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
    }

    #[test]
    fn topo_order_rejects_missing_prerequisite() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("B", "missing")]).unwrap_err();
        assert!(err.contains("'missing' of 'B' is not in the plan"), "{err}");
        // Чисто файловое предусловие: dependent обязан существовать,
        // предшественник не нужен — порядок не меняется.
        let ordered = topo_order_steps(&steps, &[dep("A", "")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B"]);
        let err = topo_order_steps(&steps, &[dep("ghost", "")]).unwrap_err();
        assert!(err.contains("'ghost' is not in the plan"), "{err}");
    }

    #[test]
    fn topo_order_rejects_self_dependency() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("A", "A")]).unwrap_err();
        assert!(err.contains("'A' cannot depend on itself"), "{err}");
    }

    #[test]
    fn topo_order_rejects_duplicate_step_ids() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![cmd("A"), cmd("A")];
        let err = topo_order_steps(&steps, &[]).unwrap_err();
        assert!(err.contains("duplicate step id 'A'"), "{err}");
    }

    #[test]
    fn topo_order_reports_two_node_cycle_path() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("A", "B"), dep("B", "A")]).unwrap_err();
        assert!(err.contains("Dependency cycle detected"), "{err}");
        assert!(err.contains("A -> B -> A"), "{err}");
    }

    #[test]
    fn topo_order_reports_three_node_cycle_path() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B"), cmd("C")];
        let err =
            topo_order_steps(&steps, &[dep("A", "B"), dep("B", "C"), dep("C", "A")]).unwrap_err();
        assert!(err.contains("Dependency cycle detected"), "{err}");
        assert!(err.contains("A -> B -> C -> A"), "{err}");
    }

    #[test]
    fn topo_order_reorders_parallel_flattening_dependencies() {
        // Parallel раскрывается в порядке следования шагов [B, A]; зависимость
        // B→A нормализуется перестановкой в [A, B] — план НЕ отклоняется.
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![Step::Parallel {
            id: "par".into(),
            label: "Parallel".into(),
            description: String::new(),
            steps: vec![cmd("B"), cmd("A")],
            condition: None,
            on_error: ErrorMode::Skip,
        }];
        let flat = flatten_steps(&steps, &WizardContext::default());
        assert_eq!(
            flat.iter().map(|s| s.id()).collect::<Vec<_>>(),
            vec!["B", "A"]
        );
        let dep = StepDependency {
            step_id: "B".into(),
            prereq_id: "A".into(),
            expects_file: String::new(),
        };
        let ordered = topo_order_steps(&flat, &[dep]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn execute_aborts_on_cyclic_dependencies() {
        // План с настоящим циклом не выполняется вовсе: ранний Aborted
        // с полным путём цикла, без единого шага.
        let engine = DefaultRecipeEngine::new();
        let (mut plan, dir) = plan_with_dependency(
            "cycle",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        plan.dependencies.push(StepDependency {
            step_id: "prereq".into(),
            prereq_id: "dependent".into(),
            expects_file: String::new(),
        });
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(
                &result.overall,
                OverallStatus::Aborted { reason, .. }
                    if reason.to_lowercase().contains("dependency cycle detected")
            ),
            "{:?}",
            result.overall
        );
        assert!(result.step_results.is_empty(), "ни один шаг не выполняется");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_normalizes_out_of_order_declared_steps() {
        // Прямо построенный план с зависимым шагом ДО предшественника:
        // execute() нормализует порядок (стабильная топосортировка) и
        // выполняет оба шага — предшественник первым.
        let engine = DefaultRecipeEngine::new();
        let (mut plan, dir) = plan_with_dependency(
            "reorder",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        plan.steps = vec![plan.steps[1].clone(), plan.steps[0].clone()];
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.overall, OverallStatus::Success),
            "{:?}",
            result.overall
        );
        let ids: Vec<String> = result
            .step_results
            .iter()
            .map(|r| r.step_id.clone())
            .collect();
        assert_eq!(ids, vec!["prereq", "dependent"]);
        assert!(
            result
                .step_results
                .iter()
                .all(|r| matches!(r.status, StepStatus::Success { .. })),
            "{:?}",
            result.step_results
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_dependencies_filters_absent_steps_and_derives_expects_file() {
        let cmd = |id: &str, condition: Option<StepCondition>| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![
            cmd("scaffold", None),
            // ровно ОДНА выжившая декларация → expects_file выводится из
            // FileExists-условия зависимого шага
            cmd(
                "patch",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
            cmd("other", None),
            // явное пост-условие переопределяет вывод из условия
            cmd(
                "patch2",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
        ];
        let declared = vec![
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
            // висячий предшественник — отбрасывается
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "ghost".into(),
                expects_file: String::new(),
            },
            // висячий dependent — отбрасывается
            StepDependency {
                step_id: "ghost".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
            StepDependency {
                step_id: "patch2".into(),
                prereq_id: "other".into(),
                expects_file: "explicit.txt".into(),
            },
            // дубликат схлопывается
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
        ];
        let built = build_dependencies(&steps, &declared);
        assert_eq!(built.len(), 2, "{built:?}");
        let derived = built
            .iter()
            .find(|d| d.prereq_id == "scaffold")
            .expect("выжившая декларация");
        assert_eq!(derived.expects_file, "frontend/package.json");
        let explicit = built
            .iter()
            .find(|d| d.prereq_id == "other")
            .expect("выжившая декларация");
        assert_eq!(explicit.expects_file, "explicit.txt");
    }

    #[test]
    fn build_dependencies_skips_derivation_for_multi_prereq_dependents() {
        // qt_cmake_build зависит и от qt_web_build, и от qt_cmake_configure:
        // его FileExists-условие (frontend/dist/index.html) — пост-условие
        // ТОЛЬКО первого, авто-вывод отключён, expects_file задан явно.
        let cmd = |id: &str, condition: Option<StepCondition>| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![
            cmd(
                "qt_web_build",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
            cmd(
                "qt_cmake_configure",
                Some(StepCondition::FileExists {
                    path: "CMakeLists.txt".into(),
                }),
            ),
            cmd(
                "qt_cmake_build",
                Some(StepCondition::FileExists {
                    path: "frontend/dist/index.html".into(),
                }),
            ),
        ];
        let declared = vec![
            StepDependency {
                step_id: "qt_cmake_build".into(),
                prereq_id: "qt_web_build".into(),
                expects_file: "frontend/dist/index.html".into(),
            },
            StepDependency {
                step_id: "qt_cmake_build".into(),
                prereq_id: "qt_cmake_configure".into(),
                expects_file: String::new(),
            },
        ];
        let built = build_dependencies(&steps, &declared);
        assert_eq!(built.len(), 2);
        let web = built
            .iter()
            .find(|d| d.prereq_id == "qt_web_build")
            .unwrap();
        assert_eq!(web.expects_file, "frontend/dist/index.html");
        let cmake = built
            .iter()
            .find(|d| d.prereq_id == "qt_cmake_configure")
            .unwrap();
        assert_eq!(
            cmake.expects_file, "",
            "без авто-вывода для множественных dep"
        );
    }

    #[test]
    fn recipe_declares_dependency_pairs() {
        // Nest + telegraf: патчи package.json — после nest_new.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "telegraf".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            find("nest_pkg_name", "nest_new"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("telegraf_pkg_patch", "nest_new"),
            "{:?}",
            recipe.dependencies
        );

        // Gin + Cobra: go-команды — после go mod init.
        let mut ctx = context();
        ctx.languages = vec!["go".into()];
        ctx.frameworks = vec!["gin".into(), "cobra".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(recipe
            .dependencies
            .iter()
            .any(|d| d.step_id == "get_gin" && d.prereq_id == "go_mod_init"));
        assert!(recipe
            .dependencies
            .iter()
            .any(|d| d.step_id == "get_cobra" && d.prereq_id == "go_mod_init"));
    }

    #[test]
    fn recipe_declares_python_dependencies() {
        // python + alembic: pip/alembic — строго после venv и pip-установки.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(find("py_pip_upgrade", "py_venv_create"));
        assert!(find("py_pip_check", "py_venv_create"));
        assert!(find("py_pip_install", "py_pip_upgrade"));
        assert!(find("py_pip_install", "py_pip_check"));
        assert!(find("alembic_init", "py_pip_install"));
        assert!(find("py_requirements_check", "py_pip_install"));
        // маркер venv — пост-условие venv-шагов (единый канонический venv)
        let dep = recipe
            .dependencies
            .iter()
            .find(|d| d.step_id == "py_pip_upgrade" && d.prereq_id == "py_venv_create")
            .unwrap();
        assert_eq!(
            dep.expects_file, "venv/pyvenv.cfg",
            "{:?}",
            recipe.dependencies
        );
    }

    #[test]
    fn recipe_declares_django_start_dependency() {
        // django-admin (django_start) — строго ПОСЛЕ установки манифеста
        // единого venv: отдельного django-venv не существует, пакеты ставятся
        // ровно один раз через py_pip_install.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == "django_start" && d.prereq_id == "py_pip_install"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "django_venv_create")
                && !recipe.steps.iter().any(|s| s.id() == "django_pip_install"),
            "django использует единый venv, отдельного django-venv нет"
        );
    }

    #[test]
    fn recipe_declares_qt_webengine_dependencies() {
        // qt webengine: cmake-сборка — после веб-сборки (с явным
        // пост-условием frontend/dist/index.html) и после cmake-конфигурации.
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            find("qt_web_build", "vite_create"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_configure", "qt_cmake"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_build", "qt_web_build"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_build", "qt_cmake_configure"),
            "{:?}",
            recipe.dependencies
        );
        let dep = recipe
            .dependencies
            .iter()
            .find(|d| d.step_id == "qt_cmake_build" && d.prereq_id == "qt_web_build")
            .unwrap();
        assert_eq!(dep.expects_file, "frontend/dist/index.html");
    }

    #[test]
    fn plan_keeps_only_surviving_dependencies() {
        // tauri + react (компаньон): фронтенд скаффолдит vite_create, а
        // НЕ tauri_web_scaffold — декларация на отсутствующий шаг исчезает,
        // выжившая получает expects_file из условия зависимого шага.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "rust".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];
        let engine = DefaultRecipeEngine::new();
        let plan = engine
            .plan(&ctx, std::path::Path::new("C:\\dev\\myapp"))
            .expect("plan должен собраться");
        let ids: Vec<String> = plan.steps.iter().map(|s| s.id()).collect();
        assert!(ids.iter().any(|i| i == "vite_create"), "{ids:?}");
        assert!(!ids.iter().any(|i| i == "tauri_web_scaffold"), "{ids:?}");
        let has_pair = |plan: &ExecutionPlan, step: &str, prereq: &str| {
            plan.dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            has_pair(&plan, "tauri_pkg_name", "vite_create"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            !has_pair(&plan, "tauri_pkg_name", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            has_pair(&plan, "tauri_config_patch", "tauri_init"),
            "{:?}",
            plan.dependencies
        );
        let dep = plan
            .dependencies
            .iter()
            .find(|d| d.step_id == "tauri_pkg_name" && d.prereq_id == "vite_create")
            .unwrap();
        assert_eq!(dep.expects_file, "frontend/package.json");
        // без компаньона выживает tauri_web_scaffold
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "rust".into()];
        ctx.frameworks = vec!["tauri".into()];
        let plan = engine
            .plan(&ctx, std::path::Path::new("C:\\dev\\myapp"))
            .unwrap();
        assert!(
            has_pair(&plan, "tauri_pkg_name", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            !has_pair(&plan, "tauri_pkg_name", "vite_create"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            has_pair(&plan, "tauri_web_install", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
    }

    #[test]
    fn preview_shows_dependencies_and_skip_reasons() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "preview",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let preview = engine.preview(&plan);
        let dependent = preview
            .step_previews
            .iter()
            .find(|p| p.id == "dependent")
            .expect("dependent в превью");
        assert_eq!(dependent.prerequisites, vec!["prereq"]);
        assert!(
            dependent
                .possible_skip_reasons
                .iter()
                .any(|r| r.contains("'prereq'")),
            "{:?}",
            dependent.possible_skip_reasons
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recipe_without_dependencies_field_parses() {
        // Обратная совместимость: старые сериализованные рецепты (без
        // поля dependencies) десериализуются в пустой список.
        let json = r#"{
            "id": "recipe_old",
            "name": "Old recipe",
            "description": "legacy",
            "tags": ["typescript"],
            "steps": []
        }"#;
        let recipe: Recipe = serde_json::from_str(json).expect("старый Recipe парсится");
        assert!(recipe.dependencies.is_empty());
        // ExecutionPlan/StepPreview — те же гарантии
        let preview_json = r#"{"id":"s1","label":"L","description":"D","action":"$ x","will_execute":true,"skip_reason":null}"#;
        let sp: StepPreview =
            serde_json::from_str(preview_json).expect("старый StepPreview парсится");
        assert!(sp.prerequisites.is_empty());
        assert!(sp.possible_skip_reasons.is_empty());
    }

    // ==================== Scenario A: Nest + Telegraf ====================

    #[test]
    fn nest_uses_yes_flag_to_skip_npx_prompt() {
        // Без --yes npx спрашивает «Ok to proceed?» и падает в не-TTY
        // сессии; --package-manager фиксирует ответ промпта флагом.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let step = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        let args = cmd_args(step);
        assert_eq!(
            args.get(0).map(String::as_str),
            Some("--yes"),
            "--yes сразу после npx (иначе prompt 'Ok to proceed?'): {args:?}"
        );
        assert!(args.iter().any(|a| a == "--package-manager"), "{args:?}");
        assert!(args.iter().any(|a| a == "--skip-install"), "{args:?}");
        assert!(args.iter().any(|a| a == "--skip-git"), "{args:?}");
    }

    #[test]
    fn telegraf_patches_nest_package_json_without_clobbering() {
        // nest + telegraf: telegraf — side-фреймворк (kind="side"), его шаги
        // выполняются ПОСЛЕ nest и НЕ перезаписывают package.json nest.
        // В split (react + nest + telegraf) оба живут в backend/: dep-патч
        // работает в backend/ с условием FileExists backend/package.json.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.backend_languages = vec!["typescript".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "react".into(), "telegraf".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "telegraf_package"),
            "при nest telegraf не пишет собственный package.json (затирал бы nest)"
        );
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "telegraf_pkg_patch")
            .expect("dep-патч обязан быть при nest");
        match patch {
            Step::Command {
                working_dir,
                condition,
                args,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "патч работает в каталоге nest-каркаса"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "backend/package.json", "условие seg-префиксовано")
                    }
                    other => panic!("ожидали FileExists backend/package.json: {other:?}"),
                }
                let script = args
                    .iter()
                    .find(|a| a.starts_with("const fs="))
                    .expect("node -e скрипт");
                assert!(script.contains("telegraf"), "{script}");
                assert!(script.contains("4.16.3"), "{script}");
            }
            _ => panic!("telegraf_pkg_patch — Command"),
        }
        // Без nest telegraf пишет собственный package.json
        let mut solo = context();
        solo.languages = vec!["typescript".into()];
        solo.frameworks = vec!["telegraf".into()];
        let solo_recipe = recipe_for(&solo, "myapp").expect("recipe must build");
        assert!(solo_recipe
            .steps
            .iter()
            .any(|s| s.id() == "telegraf_package"));
        assert!(!solo_recipe
            .steps
            .iter()
            .any(|s| s.id() == "telegraf_pkg_patch"));
    }

    #[test]
    fn express_fastify_telegraf_declare_real_deps_and_validate_manifest() {
        // Express/Fastify/Telegraf: зависимости — НАСТОЯЩИЕ записи в
        // package.json (финальный npm install ставит их), а не упоминание в
        // entry-файле. Каждый каркас получает пост-валидацию manifest-check
        // (Abort), требующую свой пакет.
        for (fw, pkg, check_id, package_id) in [
            ("express", "express", "express_pkg_check", "express_package"),
            ("fastify", "fastify", "fastify_pkg_check", "fastify_package"),
            (
                "telegraf",
                "telegraf",
                "telegraf_pkg_check",
                "telegraf_package",
            ),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["javascript".into()];
            ctx.frameworks = vec![fw.into()];
            let layout = ProjectLayout::compute(&ctx);
            let pkg_path = match layout.framework_dir(fw) {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            match find_step(&recipe, check_id) {
                Step::Generate {
                    generator_id,
                    generator_config,
                    on_error,
                    ..
                } => {
                    assert_eq!(generator_id, "manifest-check", "{check_id}");
                    assert_eq!(
                        generator_config.get("path").and_then(|v| v.as_str()),
                        Some(pkg_path.as_str()),
                        "{check_id}"
                    );
                    assert!(
                        gen_strs(generator_config, "required_dependencies")
                            .contains(&pkg.to_string()),
                        "{check_id}"
                    );
                    assert_eq!(
                        on_error,
                        &ErrorMode::Abort,
                        "отсутствие зависимости фреймворка останавливает пайплайн: {check_id}"
                    );
                }
                _ => panic!("{check_id} — Generate"),
            }
            match find_step(&recipe, package_id) {
                Step::WriteFile { content, .. } => {
                    assert!(
                        content.contains(&format!("\"{pkg}\"")),
                        "{package_id} обязан декларировать {pkg}: {content}"
                    );
                }
                _ => panic!("{package_id} — WriteFile"),
            }
        }
    }

    #[test]
    fn side_frameworks_run_after_main_frameworks() {
        // kind="side" (telegraf, aiogram) выполняется ПОСЛЕ главных
        // фреймворков (nest, django) независимо от порядка карточек в
        // мастере: их шаги пишут поверх/патчат каркас главного.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["aiogram".into(), "django".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("django_start") < idx("aiogram_bot"),
            "django (app) обязан скаффолдиться до aiogram (side)"
        );
    }

    // ==================== Scenario B: Laravel / Symfony / PHP ====================

    #[test]
    fn laravel_symfony_php_preflight_precedes_composer_without_platform_req() {
        // composer create-project: PHP-префлайт (ext-fileinfo) идёт ДО
        // composer-скаффолда, а --ignore-platform-req=ext-fileinfo удалён —
        // он маскировал отсутствие расширения и Laravel/Symfony падали
        // в рантайме с невнятными ошибками.
        for (fw_id, check_id, new_id) in [
            ("laravel", "laravel_php_check", "laravel_new"),
            ("symfony", "symfony_php_check", "symfony_new"),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["php".into()];
            ctx.frameworks = vec![fw_id.into()];
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            let idx = |id: &str| {
                recipe
                    .steps
                    .iter()
                    .position(|s| s.id() == id)
                    .unwrap_or_else(|| panic!("{fw_id}: {id} должен быть в плане"))
            };
            assert!(
                idx(check_id) < idx(new_id),
                "{fw_id}: php-префлайт обязан идти ДО composer create-project"
            );
            let check = &recipe.steps[idx(check_id)];
            let cargs = cmd_args(check);
            assert_eq!(
                cargs[0], "-d",
                "префлайт — php -d extension=fileinfo -r скрипт"
            );
            assert_eq!(cargs[1], "extension=fileinfo");
            assert_eq!(
                cargs[2], "-r",
                "префлайт — php -d extension=fileinfo -r скрипт"
            );
            let scaffold = recipe.steps.iter().find(|s| s.id() == new_id).unwrap();
            if let Step::Generate {
                generator_config, ..
            } = scaffold
            {
                let args = gen_args(generator_config);
                assert!(
                    !args.iter().any(|a| a.contains("ignore-platform-req")),
                    "{fw_id}: --ignore-platform-req удалён (маскировал отсутствие ext-fileinfo): {args:?}"
                );
            } else {
                panic!("{new_id} — Generate");
            }
        }
    }

    // ==================== Scenario C: Django / FastAPI / Python ====================

    #[test]
    fn python_preflight_runs_before_venv_and_django_steps() {
        // python_preflight (версия + путь интерпретатора) выполняется
        // РАНЬШЕ любых venv/pip/django-admin шагов: ошибка интерпретатора
        // видна сразу, а не в середине пайплайна.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(idx("python_preflight") < idx("py_venv_create"));
        assert!(idx("python_preflight") < idx("py_pip_install"));
        // django_start декларируется в фазе 1 (root-скаффолд), но его
        // предусловие py_pip_install (топологически) ставит django-admin
        // ПОСЛЕ установки манифеста — venv/pip гарантированно готовы.
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == "django_start" && d.prereq_id == "py_pip_install"),
            "{:?}",
            recipe.dependencies
        );
        let preflight = &recipe.steps[idx("python_preflight")];
        match preflight {
            Step::Command {
                command, on_error, ..
            } => {
                assert_eq!(
                    command,
                    &python_command(),
                    "интерпретатор обязан совпадать с python_command() (реальный бинарь, а не Store-заглушка): {command}"
                );
                assert_eq!(on_error, &ErrorMode::Abort);
            }
            _ => panic!("python_preflight — Command"),
        }
    }

    #[test]
    fn py_venv_create_skips_when_django_created_venv_early() {
        // Ранний django-venv (django_venv_create) больше не существует, но
        // шаг обязан оставаться идемпотентным: на Windows это гейт
        // FileNotExists по маркеру (seg-префикс в split), на Unix —
        // самопроверка здоровья внутри генератора python-venv.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let venv = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create должен быть в плане");
        if cfg!(target_os = "windows") {
            match venv {
                Step::Command {
                    condition, command, ..
                } => {
                    match condition {
                        Some(StepCondition::FileNotExists { path }) => {
                            assert_eq!(path, "backend/venv/pyvenv.cfg", "маркер в сегменте")
                        }
                        other => panic!("ожидали FileNotExists: {other:?}"),
                    }
                    assert_eq!(
                        command,
                        &python_command(),
                        "интерпретатор обязан совпадать с python_command() (реальный бинарь, а не Store-заглушка): {command}"
                    );
                }
                other => panic!("на Windows py_venv_create — Command: {:?}", other.id()),
            }
        } else {
            match venv {
                Step::Generate {
                    generator_id,
                    generator_config,
                    ..
                } => {
                    assert_eq!(generator_id, "python-venv");
                    assert_eq!(
                        generator_config.get("python").and_then(|v| v.as_str()),
                        Some(python_command().as_str())
                    );
                    assert!(
                        venv_path_of(venv).ends_with("backend/venv"),
                        "venv в сегменте"
                    );
                }
                other => panic!("на Unix py_venv_create — Generate: {:?}", other.id()),
            }
        }
    }

    #[test]
    fn py_pip_upgrade_precedes_pip_install() {
        // Bootstrap pip (python -m pip install --upgrade pip) идёт ДО
        // py_pip_install: старые окружения несут устаревший pip, который
        // ломает установку requirements.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(idx("py_pip_upgrade") < idx("py_pip_install"));
        let upgrade = &recipe.steps[idx("py_pip_upgrade")];
        match upgrade {
            Step::Command { command, args, .. } => {
                assert!(command.contains("venv"), "pip из venv: {command}");
                let js = args.join(" ");
                assert!(js.contains("--upgrade") && js.ends_with("pip"), "{js}");
            }
            _ => panic!("py_pip_upgrade — Command"),
        }
    }

    #[test]
    fn alembic_env_patch_wires_database_url_in_segment() {
        // alembic init генерирует env.py со статическим sqlalchemy.url —
        // README/.env обещают DATABASE_URL из окружения. Патч-шаг обязан
        // идти после init, работать в каталоге python-сегмента, применяться
        // только к существующему migrations/env.py и не валить проект (Skip).
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["alembic".into(), "sqlalchemy".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let patch = find_step(&recipe, "alembic_env_patch");
        match patch {
            Step::Command {
                command,
                args,
                working_dir,
                condition,
                on_error,
                ..
            } => {
                assert!(
                    command.contains("backend") && command.contains("venv"),
                    "патч запускается интерпретатором venv сегмента: {command}"
                );
                assert_eq!(args[0], "-c");
                let script = &args[1];
                assert!(script.contains("DATABASE_URL"), "{script}");
                assert!(script.contains("set_main_option"), "{script}");
                assert!(
                    script.contains("replace('%', '%%')"),
                    "configparser интерполирует % — URL экранируется: {script}"
                );
                assert_eq!(
                    working_dir.as_deref(),
                    Some("./backend"),
                    "патч работает там же, где alembic init"
                );
                assert!(
                    matches!(
                        condition,
                        Some(StepCondition::FileExists { path })
                            if path == "backend/migrations/env.py"
                    ),
                    "{condition:?}"
                );
                assert_eq!(
                    on_error,
                    &ErrorMode::Skip,
                    "провал патча не должен валить сгенерированный проект"
                );
            }
            other => panic!("alembic_env_patch — Command: {:?}", other.id()),
        }
        assert_dep(&recipe, "alembic_env_patch", "alembic_init");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(idx("alembic_init") < idx("alembic_env_patch"));
    }

    #[test]
    fn alembic_env_patch_script_rewrites_env_py_idempotently() {
        // Реальное исполнение скрипта патча (без сети): fake env.py получает
        // set_main_option из os.environ, повторный прогон — no-op.
        let python = if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        };
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return; // интерпретатор недоступен — скрипт проверяется по конфигу выше
        }
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_alembic_patch_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("migrations")).unwrap();
        std::fs::write(dir.join("migrations/env.py"), "config = context.config\n").unwrap();

        let run = || {
            std::process::Command::new(python)
                .args(["-c", ALEMBIC_DATABASE_URL_PATCH])
                .current_dir(&dir)
                .output()
                .expect("скрипт патча обязан запускаться")
        };

        let first = run();
        assert!(
            first.status.success(),
            "патч упал: {}",
            String::from_utf8_lossy(&first.stderr)
        );
        let text = std::fs::read_to_string(dir.join("migrations/env.py")).unwrap();
        assert!(text.contains("STACKPILOT_DATABASE_URL"), "{text}");
        assert!(text.contains("set_main_option"), "{text}");
        assert!(text.contains("config = context.config"), "{text}");

        // Идемпотентность: второй прогон ничего не добавляет.
        let second = run();
        assert!(second.status.success());
        let stdout = String::from_utf8_lossy(&second.stdout);
        assert!(stdout.contains("already reads DATABASE_URL"), "{stdout}");
        let text_after = std::fs::read_to_string(dir.join("migrations/env.py")).unwrap();
        assert_eq!(text, text_after, "повторный запуск не меняет файл");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pytest_tool_creates_smoke_test_in_python_segment() {
        // Без smoke-теста `pytest` на свежем проекте возвращает код 5
        // («no tests collected») и CI красный сразу после генерации.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["pytest".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let smoke = find_step(&recipe, "pytest_smoke_test");
        assert_eq!(
            write_path_of(smoke),
            "backend/tests/test_smoke.py",
            "smoke-тест живёт в python-сегменте рядом с pytest.ini"
        );
        match smoke {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("def test_smoke"), "{content}");
                assert!(content.contains("assert True"), "{content}");
            }
            other => panic!("pytest_smoke_test — WriteFile: {:?}", other.id()),
        }

        // Корневой проект: tests/test_smoke.py.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["pytest".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_eq!(
            write_path_of(find_step(&recipe, "pytest_smoke_test")),
            "tests/test_smoke.py"
        );
    }

    #[test]
    fn alembic_init_runs_in_python_segment_dir() {
        // alembic init создаёт migrations/ В КАТАЛОГЕ python-сегмента
        // (backend/), рядом с venv и requirements.txt, а не в корне проекта.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("./backend"),
                    "alembic init работает в каталоге python-сегмента"
                );
            }
            _ => panic!("alembic_init — Command"),
        }
    }

    #[test]
    fn python_root_project_uses_canonical_venv_and_manifest_at_root() {
        // Корневой Python-проект (backend-only, fastapi): единый канонический
        // venv лежит в <root>/venv, манифест — <root>/requirements.txt,
        // пост-валидация требует fastapi И ASGI-сервер (uvicorn).
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_py_venv_create(&recipe, "venv/pyvenv.cfg", "venv");
        assert_eq!(
            write_path_of(find_step(&recipe, "requirements_txt")),
            "requirements.txt"
        );
        match find_step(&recipe, "requirements_txt") {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("fastapi[standard]"), "{content}");
                assert!(
                    content.contains("uvicorn"),
                    "ASGI-сервер обязателен: {content}"
                );
            }
            _ => unreachable!(),
        }
        match find_step(&recipe, "py_requirements_check") {
            Step::Generate {
                generator_id,
                generator_config,
                ..
            } => {
                assert_eq!(generator_id, "manifest-check");
                assert_eq!(
                    generator_config.get("path").and_then(|v| v.as_str()),
                    Some("requirements.txt")
                );
                let deps = gen_strs(generator_config, "required_dependencies");
                assert!(deps.contains(&"fastapi".to_string()), "{deps:?}");
                assert!(deps.contains(&"uvicorn".to_string()), "{deps:?}");
            }
            _ => panic!("py_requirements_check — Generate"),
        }
    }

    #[test]
    fn python_backend_segment_places_venv_and_manifest_in_backend() {
        // Split-проект (python backend + typescript frontend): venv и
        // requirements.txt живут ВНУТРИ backend/, маркер — root-relative
        // backend/venv/pyvenv.cfg, django задекларирован в манифесте.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_py_venv_create(&recipe, "backend/venv/pyvenv.cfg", "backend/venv");
        assert_eq!(
            write_path_of(find_step(&recipe, "requirements_txt")),
            "backend/requirements.txt"
        );
        match find_step(&recipe, "requirements_txt") {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("django"), "{content}");
            }
            _ => unreachable!(),
        }
        match find_step(&recipe, "py_requirements_check") {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("path").and_then(|v| v.as_str()),
                    Some("backend/requirements.txt")
                );
                let deps = gen_strs(generator_config, "required_dependencies");
                assert!(deps.contains(&"django".to_string()), "{deps:?}");
            }
            _ => panic!("py_requirements_check — Generate"),
        }
    }

    #[test]
    fn vscode_settings_point_at_canonical_python_venv() {
        // VS Code обязан видеть интерпретатор канонического venv (venv/ в
        // корне, backend/venv в split), а не исторический `.venv`.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let config = gen_config_of_step(find_step(&recipe, "vscode_merge"));
        let interpreter = config
            .get("python_interpreter")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if cfg!(target_os = "windows") {
            assert_eq!(
                interpreter,
                "${workspaceFolder}\\venv\\Scripts\\python.exe",
                "корневой venv"
            );
        } else {
            assert_eq!(
                interpreter,
                "${workspaceFolder}/venv/bin/python",
                "корневой venv"
            );
        }

        // Split: python-код и venv — в backend/.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let config = gen_config_of_step(find_step(&recipe, "vscode_merge"));
        let interpreter = config
            .get("python_interpreter")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            interpreter.contains("backend")
                && interpreter.contains("venv")
                && interpreter.starts_with("${workspaceFolder}"),
            "интерпретатор из backend/venv: {interpreter}"
        );

        // Без python интерпретатор не передаётся вовсе.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let config = gen_config_of_step(find_step(&recipe, "vscode_merge"));
        assert!(
            config.get("python_interpreter").map(|v| v.is_null()).unwrap_or(true),
            "без Python ключа быть не должно: {config}"
        );
    }

    /// CI: рабочий каталог workflow (или None — корень проекта).
    fn ci_workdir_of(
        languages: &[&str],
        backend_langs: &[&str],
        frontend_langs: &[&str],
        frameworks: &[&str],
    ) -> Option<String> {
        let mut ctx = context();
        ctx.languages = languages.iter().map(|s| s.to_string()).collect();
        ctx.backend_languages = backend_langs.iter().map(|s| s.to_string()).collect();
        ctx.frontend_languages = frontend_langs.iter().map(|s| s.to_string()).collect();
        ctx.frameworks = frameworks.iter().map(|s| s.to_string()).collect();
        ctx.ci = true;
        let layout = ProjectLayout::compute(&ctx);
        let steps = steps_for_ci(&layout, &ctx, "C:\\dev\\myapp", "myapp");
        let ci = steps
            .iter()
            .find(|s| s.id() == "ci_workflow")
            .expect("ci_workflow должен быть в плане");
        match ci {
            Step::WriteFile { content, .. } => content
                .lines()
                .find_map(|line| line.trim().strip_prefix("working-directory: "))
                .map(String::from),
            other => panic!("ci_workflow — WriteFile: {:?}", other.id()),
        }
    }

    #[test]
    fn ci_workdir_follows_layout() {
        // Split nest+react: CI главного фреймворка (nest) — в backend/;
        // без working-directory `npm ci` из корня не находит package.json.
        assert_eq!(
            ci_workdir_of(&["typescript"], &[], &[], &["nest", "react"]).as_deref(),
            Some("backend"),
            "nest+react — backend/"
        );
        // Frontend-only React: vite-каркас лежит в frontend/.
        assert_eq!(
            ci_workdir_of(&["typescript"], &[], &["typescript"], &["react"]).as_deref(),
            Some("frontend"),
            "react-only — frontend/"
        );
        // Backend-only python: всё в корне, defaults-блока нет.
        assert_eq!(
            ci_workdir_of(&["python"], &["python"], &[], &["fastapi"]),
            None,
            "fastapi backend-only — корень"
        );
        // Split python+react: django-код и venv — в backend/.
        assert_eq!(
            ci_workdir_of(
                &["python", "typescript"],
                &["python"],
                &["typescript"],
                &["django", "react"],
            )
            .as_deref(),
            Some("backend"),
            "django+react — backend/"
        );
        // Tauri: rust живёт в src-tauri/, веб-часть — в frontend/, корнем
        // владеет оболочка.
        assert_eq!(
            ci_workdir_of(&["rust", "typescript"], &["rust"], &["typescript"], &["tauri", "react"])
                .as_deref(),
            Some("src-tauri"),
            "tauri+rust — src-tauri/"
        );
        assert_eq!(
            ci_workdir_of(&["typescript", "rust"], &["rust"], &["typescript"], &["tauri", "react"])
                .as_deref(),
            Some("frontend"),
            "tauri+typescript — frontend/"
        );
        // Порядок карточек не должен ломать выбор: react первым в списке
        // фреймворков не уводит CI python-стека в frontend/ — фреймворк
        // обязан использовать главный язык (react не использует python).
        assert_eq!(
            ci_workdir_of(
                &["python", "typescript"],
                &["python"],
                &["typescript"],
                &["react", "django"],
            )
            .as_deref(),
            Some("backend"),
            "python + [react, django] — backend/"
        );
    }

    // ==================== Scenario E: Zig / Flutter ====================

    #[test]
    fn zig_init_uses_generates_root_shell_capability() {
        // `zig init` раскладывает shell в текущем каталоге (способность
        // generates_root_shell), пост-условия — build.zig + build.zig.zon.
        // В split (zig + flutter) каталогом становится backend/.
        let mut ctx = context();
        ctx.languages = vec!["zig".into()];
        ctx.frameworks = vec![];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let zig = recipe
            .steps
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init должен быть в плане");
        match zig {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("zig")
                );
                assert_eq!(gen_args(generator_config), vec!["init".to_string()]);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell")
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["build.zig".to_string(), "build.zig.zon".to_string()]
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some(".")
                );
                assert_eq!(on_error, &ErrorMode::Skip);
            }
            _ => panic!("zig_init — Generate"),
        }

        // Split: zig + flutter → zig init работает в backend/
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let zig = recipe
            .steps
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init должен быть в плане (zig-cli не подавляет zig-скаффолд)");
        match zig {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("backend"),
                    "в split zig init работает в backend/"
                );
            }
            _ => panic!("zig_init — Generate"),
        }
    }

    #[test]
    fn flutter_create_uses_project_name_flag_in_current_directory() {
        // `flutter create --project-name <safe> .` работает ВНУТРИ frontend/
        // (creates_in_current_directory) — без вложенной матрёшки
        // frontend/<name>/; пост-условия — pubspec.yaml + lib/.
        let mut ctx = context();
        ctx.project_name = Some("my-app".into());
        ctx.languages = vec!["dart".into()];
        ctx.frameworks = vec!["flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let flutter = recipe
            .steps
            .iter()
            .find(|s| s.id() == "flutter_create")
            .expect("flutter_create должен быть в плане");
        match flutter {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("flutter")
                );
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("create"));
                assert_eq!(args.get(1).map(String::as_str), Some("--project-name"));
                assert_eq!(
                    args.get(2).map(String::as_str),
                    Some("my_app"),
                    "дефис в имени → валидный Dart-пакет"
                );
                assert_eq!(
                    args.get(3).map(String::as_str),
                    Some("__TARGET__"),
                    "CLI работает в каталоге назначения, без вложенной папки: {args:?}"
                );
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_in_current_directory")
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["pubspec.yaml".to_string(), "lib".to_string()]
                );
            }
            _ => panic!("flutter_create — Generate"),
        }
    }

    #[test]
    fn flutter_preflight_aborts_when_sdk_missing() {
        // Отсутствующий flutter обязан ОСТАНОВИТЬ пайплайн (Abort) с
        // понятной причиной, а не маскироваться скипом каркаса: префлайт
        // проверяет SDK строго ДО flutter create.
        let mut ctx = context();
        ctx.languages = vec!["dart".into()];
        ctx.frameworks = vec!["flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let pre = recipe
            .steps
            .iter()
            .find(|s| s.id() == "flutter_preflight")
            .expect("flutter_preflight должен быть в плане");
        match pre {
            Step::Command {
                command,
                args,
                on_error,
                ..
            } => {
                assert_eq!(command, "flutter");
                assert_eq!(args, &vec!["--version".to_string()]);
                assert_eq!(
                    on_error,
                    &ErrorMode::Abort,
                    "SDK отсутствует — пайплайн останавливается"
                );
            }
            _ => panic!("flutter_preflight — Command"),
        }

        let pre_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "flutter_preflight")
            .unwrap();
        let create_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "flutter_create")
            .unwrap();
        assert!(pre_idx < create_idx, "префлайт ДО flutter create");
    }

    #[test]
    fn scaffold_ownership_table_describes_known_frameworks() {
        // Единственная таблица владения: поведение каждого скаффолдера
        // описано здесь, движок не делает выводов по id фреймворка.
        let tauri = ScaffoldOwnership::for_framework("tauri");
        assert!(tauri.creates_app_shell);
        assert!(!tauri.creates_frontend, "tauri — обёртка, фронтенд чужой");
        assert!(
            tauri.wraps_existing_project,
            "tauri init требует готовый фронтенд"
        );
        assert!(tauri.may_run_in_existing_dir);
        assert!(!tauri.supports_staging_dir);
        assert_eq!(
            tauri.expected_outputs,
            &["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"]
        );

        let electron = ScaffoldOwnership::for_framework("electron");
        assert!(
            electron.creates_frontend_shell(),
            "electron: main + renderer"
        );
        assert!(!electron.is_ui_companion);
        assert!(
            electron.supports_staging_dir,
            "Forge init — только в пустой папке (temp+move)"
        );
        assert!(!electron.may_run_in_existing_dir);
        assert_eq!(electron.expected_outputs, &["package.json"]);

        let flutter = ScaffoldOwnership::for_framework("flutter");
        assert!(flutter.creates_frontend_shell(), "flutter: dart-ui");
        assert!(!flutter.is_ui_companion);
        assert!(
            flutter.may_run_in_existing_dir,
            "flutter create работает ВНУТРИ каталога"
        );
        assert!(!flutter.supports_staging_dir);
        assert_eq!(flutter.expected_outputs, &["pubspec.yaml", "lib"]);

        let qt = ScaffoldOwnership::for_framework("qt");
        assert!(qt.creates_app_shell);
        assert!(
            !qt.creates_frontend,
            "qt: собственный UI-стек, веб-часть — компаньон"
        );
        assert!(
            !qt.wraps_existing_project,
            "qt собирается в своём каталоге, не поверх проекта"
        );

        // UI-компаньоны — встраиваемые библиотеки под оболочку.
        for companion in ["react", "vue", "svelte"] {
            let o = ScaffoldOwnership::for_framework(companion);
            assert!(o.is_ui_companion, "{companion} — UI-компаньон");
            assert!(
                o.creates_frontend_shell(),
                "{companion} — полный фронтенд-каркас"
            );
            assert!(o.requires_empty_dir);
        }
        // Веб-фреймворки и прочие владельцы компаньонами не являются.
        for owner in ["nextjs", "sveltekit", "nuxt", "expo", "solidjs"] {
            assert!(
                !ScaffoldOwnership::for_framework(owner).is_ui_companion,
                "{owner}"
            );
        }

        // Inplace-фреймворки (express, fastapi, axum...) каркас не создают.
        assert_eq!(
            ScaffoldOwnership::for_framework("express"),
            ScaffoldOwnership::none()
        );
        assert_eq!(
            ScaffoldOwnership::for_framework("fastapi"),
            ScaffoldOwnership::none()
        );
        assert_eq!(
            ScaffoldOwnership::for_framework("unknown-fw"),
            ScaffoldOwnership::none()
        );
    }

    #[test]
    fn electron_scaffolds_own_frontend_and_suppresses_companion() {
        // electron + react: у electron СОБСТВЕННЫЙ renderer (create-electron-app
        // собирает main + renderer), поэтому react — UI-компаньон — НЕ
        // скаффолдится отдельно: два несвязанных фронтенда в frontend/
        // запрещены. electron_init — scaffold-генератор (temp+move: Forge init
        // в непустом каталоге назначения падает), шаблон renderer'а
        // фиксируется флагом --template.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["electron".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        assert!(
            !recipe.steps.iter().any(|s| s.id() == "vite_create"),
            "react не скаффолдится: electron владеет фронтендом"
        );

        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "electron_init")
            .expect("electron_init должен быть в плане");
        match init {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_project_and_may_prompt")
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "electron живёт в frontend/ (собственный renderer)"
                );
                let args = gen_args(generator_config);
                assert!(args.contains(&"__TARGET__".to_string()), "{args:?}");
                assert!(
                    args.iter().any(|a| a == "--template"),
                    "шаблон renderer'а фиксируется флагом: {args:?}"
                );
                assert!(
                    args.iter().any(|a| a == "vite"),
                    "TS-стек → vite-шаблон (create-electron-app): {args:?}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["package.json".to_string()]
                );
                assert!(generator_config
                    .get("interactive")
                    .and_then(|v| v.as_array())
                    .is_some_and(|entries| entries.iter().any(|e| e
                        .get("trigger")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t.contains("git repository")))));
            }
            _ => panic!("electron_init — Generate scaffold"),
        }

        // npm install ровно один раз, в frontend/ (node_modules не плодятся).
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            1,
            "один npm install на каталог с package.json"
        );
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("./frontend"));
            }
            _ => panic!("npm_install — Command"),
        }
    }

    #[test]
    fn four_target_combos_keep_legal_step_order() {
        // Четыре целевых стека: итоговый ПОРЯДОК шагов плана для каждого —
        // легальная последовательность (см. отчёт по сценариям).
        // Порядок проверяется на уровне plan() (topo_order_steps): связи,
        // объявленные в рецепте (qt_web_build после vite_create), в
        // compose_recipe гарантируются именно топологической сортировкой.
        let engine = DefaultRecipeEngine::new();
        let plan_for = |ctx: &WizardContext| -> ExecutionPlan {
            engine
                .plan(ctx, std::path::Path::new("C:\\dev\\myapp"))
                .expect("plan должен собраться")
        };
        let idx = |plan: &ExecutionPlan, id: &str| -> usize {
            plan.steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("шаг {id} отсутствует в плане"))
        };
        let installs = |plan: &ExecutionPlan| -> usize {
            plan.steps
                .iter()
                .filter(|s| s.id().starts_with("npm_install"))
                .count()
        };

        // (a) Tauri + Svelte + TypeScript: фронтенд FIRST → tauri init.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let plan_a = plan_for(&ctx);
        assert!(
            idx(&plan_a, "vite_create") < idx(&plan_a, "tauri_init"),
            "(a) svelte-фронтенд ДО tauri init"
        );
        assert!(
            idx(&plan_a, "tauri_init") < idx(&plan_a, "tauri_config_patch"),
            "(a) config-патч после init"
        );
        assert!(
            !plan_a.steps.iter().any(|s| s.id() == "cargo_init"),
            "(a) rust-языковой скаффолд подавлен tauri"
        );
        assert_eq!(installs(&plan_a), 1, "(a) один npm install");

        // (b) Electron + React + TypeScript: react подавлен (собственный
        // renderer electron), один npm install в frontend/.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["electron".into(), "react".into()];
        let plan_b = plan_for(&ctx);
        assert!(
            !plan_b.steps.iter().any(|s| s.id() == "vite_create"),
            "(b) react подавлен electron"
        );
        assert_eq!(installs(&plan_b), 1, "(b) один npm install в frontend/");

        // (c) Qt WebEngine + Vue + TypeScript: веб-сборка после vite_create,
        // cmake — после qt_cmake; сборка ждёт собранный фронтенд.
        let mut ctx = context();
        ctx.languages = vec!["cpp".into(), "typescript".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "vue".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let plan_c = plan_for(&ctx);
        assert!(
            idx(&plan_c, "vite_create") < idx(&plan_c, "qt_web_build"),
            "(c) vue-фронтенд ДО веб-сборки qt"
        );
        assert!(
            idx(&plan_c, "qt_cmake") < idx(&plan_c, "qt_cmake_configure")
                && idx(&plan_c, "qt_cmake_configure") < idx(&plan_c, "qt_cmake_build"),
            "(c) cmake: configure после CMakeLists, build после configure"
        );
        assert!(
            idx(&plan_c, "qt_web_build") < idx(&plan_c, "qt_cmake_build"),
            "(c) сборка после собранного фронтенда"
        );

        // (d) Zig CLI + Zap + Flutter: zig-shell в backend/, flutter в
        // frontend/; префлайт SDK ДО каркаса.
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "zap".into(), "flutter".into()];
        let plan_d = plan_for(&ctx);
        assert!(
            idx(&plan_d, "zig_init") < idx(&plan_d, "flutter_create"),
            "(d) zig-shell в backend/ до flutter-каркаса"
        );
        assert!(
            idx(&plan_d, "flutter_preflight") < idx(&plan_d, "flutter_create"),
            "(d) префлайт SDK ДО каркаса"
        );
        assert!(
            idx(&plan_d, "zap_zon") < idx(&plan_d, "zap_fetch")
                && idx(&plan_d, "zap_fetch") < idx(&plan_d, "zap_main"),
            "(d) zon → fetch → entry"
        );

        // Сводка порядков для отчёта (cargo test -- --nocapture).
        for (name, plan) in [
            ("(a) tauri+svelte+ts", &plan_a),
            ("(b) electron+react+ts", &plan_b),
            ("(c) qt-webengine+vue+ts", &plan_c),
            ("(d) zig-cli+zap+flutter", &plan_d),
        ] {
            println!(
                "[{name}] {}",
                plan.steps
                    .iter()
                    .map(|s| s.id())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
    }

    // ==================== Scenario G: Qt WebEngine ====================

    #[test]
    fn qt_webengine_build_and_cmake_steps_use_segment_dirs() {
        // qt-webengine + react: веб-сборка работает в frontend/ (npm run
        // build), cmake-шаги — в каталоге qt-сегмента (backend/); условия
        // FileExists seg-префиксованы (into_segment не трогает condition).
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "react".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let web_build = recipe
            .steps
            .iter()
            .find(|s| s.id() == "qt_web_build")
            .expect("qt_web_build должен быть в плане");
        match web_build {
            Step::Command {
                working_dir,
                condition,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "веб-сборка в frontend/ (рядом с qt-сегментом)"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "frontend/package.json")
                    }
                    other => panic!("ожидали FileExists frontend/package.json: {other:?}"),
                }
            }
            _ => panic!("qt_web_build — Command"),
        }

        let cmake = recipe
            .steps
            .iter()
            .find(|s| s.id() == "qt_cmake_configure")
            .expect("qt_cmake_configure должен быть в плане");
        match cmake {
            Step::Command {
                working_dir,
                condition,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "cmake работает в каталоге qt-сегмента (относительно корня проекта)"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "backend/CMakeLists.txt")
                    }
                    other => panic!("ожидали FileExists backend/CMakeLists.txt: {other:?}"),
                }
            }
            _ => panic!("qt_cmake_configure — Command"),
        }
    }

    // ==================== Scenario H: Go / Gin / Cobra / SolidStart ====================

    #[test]
    fn gin_go_get_uses_at_latest_and_is_gated_on_gomod() {
        // go get pkg@latest — современная форма (обновляет go.mod); условие
        // FileExists go.mod (seg-префикс backend/) скипает шаг без go.mod
        // вместо создания модуля в неверном каталоге.
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["gin".into(), "solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let get = recipe
            .steps
            .iter()
            .find(|s| s.id() == "get_gin")
            .expect("get_gin должен быть в плане");
        match get {
            Step::Command {
                args, condition, ..
            } => {
                assert_eq!(
                    args,
                    &vec![
                        "get".to_string(),
                        "github.com/gin-gonic/gin@latest".to_string()
                    ],
                    "go get pkg@latest: {args:?}"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => assert_eq!(path, "backend/go.mod"),
                    other => panic!("ожидали FileExists backend/go.mod: {other:?}"),
                }
            }
            _ => panic!("get_gin — Command"),
        }
    }

    #[test]
    fn cobra_steps_gated_on_gomod_in_backend_segment() {
        // go get github.com/spf13/cobra падает «go.mod file not found» без
        // модуля: шаг имеет условие FileExists go.mod (seg-префикс) и Abort —
        // каркас без реальной зависимости не проходит молча.
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["cobra".into(), "solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let step = recipe
            .steps
            .iter()
            .find(|s| s.id() == "get_cobra")
            .expect("get_cobra должен быть в плане");
        match step {
            Step::Command {
                args, condition, ..
            } => {
                assert_eq!(
                    args,
                    &vec![
                        "get".to_string(),
                        "github.com/spf13/cobra@latest".to_string()
                    ]
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(
                            path, "backend/go.mod",
                            "get_cobra: условие seg-префиксовано"
                        )
                    }
                    other => panic!("get_cobra: ожидали FileExists backend/go.mod: {other:?}"),
                }
            }
            _ => panic!("get_cobra — Command"),
        }
    }

    #[test]
    fn solidstart_noninteractive_flags() {
        // create-solid: позиционные projectName+template, --solidstart --v2
        // (без --v2 CLI спрашивает версию SolidStart), --ts (язык) —
        // полный неинтерактивный набор; шаблон "basic" валиден для
        // SolidStart (в отличие от "ts").
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let solid = recipe
            .steps
            .iter()
            .find(|s| s.id() == "solid_init")
            .expect("solid_init должен быть в плане");
        match solid {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("--yes"), "{args:?}");
                assert_eq!(
                    args.get(1).map(String::as_str),
                    Some("create-solid"),
                    "{args:?}"
                );
                assert_eq!(
                    args.get(3).map(String::as_str),
                    Some("basic"),
                    "шаблон basic (валиден для SolidStart): {args:?}"
                );
                assert!(args.contains(&"--solidstart".to_string()), "{args:?}");
                assert!(
                    args.contains(&"--v2".to_string()),
                    "без --v2 промпт версии: {args:?}"
                );
                assert!(args.contains(&"--ts".to_string()), "{args:?}");
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
            }
            _ => panic!("solid_init — Generate"),
        }
    }

    #[test]
    fn nuxt_git_init_false_is_single_token() {
        // citty (nuxi) не принимает `--gitInit false` пробелом для boolean —
        // только один токен --gitInit=false, иначе nuxi игнорирует значение.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nuxt".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let nuxt = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nuxt_create")
            .expect("nuxt_create должен быть в плане");
        match nuxt {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                assert!(
                    args.contains(&"--gitInit=false".to_string()),
                    "gitInit=false одним токеном: {args:?}"
                );
                let git_idx = args.iter().position(|a| a == "--gitInit");
                assert!(git_idx.is_none(), "пробельный вариант недопустим: {args:?}");
                assert!(args.contains(&"--no-install".to_string()), "{args:?}");
                assert!(args.contains(&"--packageManager".to_string()), "{args:?}");
            }
            _ => panic!("nuxt_create — Generate"),
        }
    }

    // ==================== Scenario D: Tauri ====================

    #[test]
    fn tauri_init_gated_on_frontend_package_json_and_expects_cargo_toml() {
        // tauri init: --yes (npx prompt), пост-условия tauri.conf.json +
        // Cargo.toml, условие FileExists frontend/package.json (init только
        // когда фронтенд-каркас реально создан).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        match init {
            Step::Generate {
                generator_config,
                condition,
                ..
            } => {
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("--yes"), "{args:?}");
                assert!(args.contains(&"--ci".to_string()), "{args:?}");
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "frontend/package.json")
                    }
                    other => panic!("ожидали FileExists frontend/package.json: {other:?}"),
                }
                let expected = gen_strs(generator_config, "expected_outputs");
                assert!(
                    expected.contains(&"src-tauri/tauri.conf.json".to_string()),
                    "{expected:?}"
                );
                assert!(
                    expected.contains(&"src-tauri/Cargo.toml".to_string()),
                    "Cargo.toml — вторая обязательная часть shell: {expected:?}"
                );
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell")
                );
            }
            _ => panic!("tauri_init — Generate"),
        }
    }

    // ==================== Validation pass: 12-stack scenario matrix ====================
    // Каждый тест проверяет КОНКРЕТНЫЙ стек: класс раскладки, порядок шагов,
    // команды/аргументы, рабочие директории, условия (root-relative), таймауты,
    // зависимости и размещение финального npm install.

    fn ctx_scenario(languages: &[&str], frameworks: &[&str], tools: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            // Полная сессия мастера: docker — осознанный выбор теста, git и
            // vscode включены явно (default() — всё off).
            docker: false,
            git_init: true,
            vscode_config: true,
            ..Default::default()
        }
    }

    /// Сквозное выполнение python-пайплайна на реальном интерпретаторе —
    /// проверяет исправление Debian/Ubuntu (без python3-venv): venv обязан
    /// создаться с рабочим pip через fallback генератора, манифест —
    /// установиться, fastapi — импортироваться, alembic env.py — получить
    /// DATABASE_URL. Требует сеть (pip install fastapi/alembic).
    /// Запускать вручную:
    /// `cargo test --lib execute_python_environment -- --ignored`.
    #[tokio::test]
    #[ignore = "requires a real python3 and network access (pip install)"]
    async fn execute_python_environment_end_to_end() {
        let engine = DefaultRecipeEngine::new();
        // Кириллица в пути — как у реальных пользователей (Загрузки/…):
        // вывод pip содержит этот путь, поэтому окно триггеров обязано
        // обрезаться по границе UTF-8 (регрессия «Pipe to stdout was broken»).
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_python_e2e_Загрузки_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut ctx = ctx_scenario(
            &["python"],
            &["fastapi"],
            &["alembic", "sqlalchemy", "pytest", "ruff"],
        );
        ctx.git_init = false;
        ctx.vscode_config = false;
        ctx.project_path = Some(dir.clone());

        let plan = engine.plan(&ctx, &dir).expect("план обязан построиться");
        // Потребитель событий работает параллельно (как UI): большой вывод
        // pip не должен блокировать ридер навсегда.
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.overall, OverallStatus::Success),
            "pipeline обязан завершиться успешно: {:?}",
            result
                .step_results
                .iter()
                .filter(|r| matches!(r.status, StepStatus::Failed { .. }))
                .map(|r| (r.step_id.clone(), r.status.clone()))
                .collect::<Vec<_>>()
        );

        // pip работает и зависимости установлены внутри venv проекта.
        let venv_bin = dir.join("venv").join(if cfg!(target_os = "windows") {
            "Scripts"
        } else {
            "bin"
        });
        let venv_python = venv_bin.join(if cfg!(target_os = "windows") {
            "python.exe"
        } else {
            "python"
        });
        let probe = std::process::Command::new(&venv_python)
            .args(["-c", "import fastapi, uvicorn; print(fastapi.__version__)"])
            .output()
            .expect("интерпретатор venv обязан запускаться");
        assert!(
            probe.status.success(),
            "fastapi/uvicorn обязаны импортироваться из venv: {}{}",
            String::from_utf8_lossy(&probe.stdout),
            String::from_utf8_lossy(&probe.stderr)
        );

        // alembic init создал env.py, патч-шаг привязал его к DATABASE_URL.
        let env_py = std::fs::read_to_string(dir.join("migrations/env.py"))
            .expect("migrations/env.py обязан существовать после alembic init");
        assert!(
            env_py.contains("STACKPILOT_DATABASE_URL"),
            "env.py обязан читать DATABASE_URL: {env_py}"
        );
        assert!(env_py.contains("set_main_option"), "{env_py}");

        // Функциональная проверка: alembic с DATABASE_URL (sqlite) обязан
        // отработать. Без патча env.py взял бы битый sqlalchemy.url из
        // alembic.ini («driver://user:pass@...») и упал бы на диалекте.
        let alembic = venv_bin.join(if cfg!(target_os = "windows") {
            "alembic.exe"
        } else {
            "alembic"
        });
        let upgrade = std::process::Command::new(&alembic)
            .args(["upgrade", "head"])
            .env("DATABASE_URL", "sqlite:///./alembic_check.db")
            .current_dir(&dir)
            .output()
            .expect("alembic обязан запускаться");
        assert!(
            upgrade.status.success(),
            "alembic upgrade head с DATABASE_URL: {}{}",
            String::from_utf8_lossy(&upgrade.stdout),
            String::from_utf8_lossy(&upgrade.stderr)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plan_ids(recipe: &Recipe) -> Vec<String> {
        recipe.steps.iter().map(|s| s.id()).collect()
    }

    fn find_step<'a>(recipe: &'a Recipe, id: &str) -> &'a Step {
        recipe
            .steps
            .iter()
            .find(|s| s.id() == id)
            .unwrap_or_else(|| panic!("step '{id}' is missing from the plan"))
    }

    /// Каждый следующий id обязан встретиться ПОСЛЕ предыдущего (подпоследовательность).
    fn assert_order(recipe: &Recipe, expected: &[&str]) {
        let ids = plan_ids(recipe);
        let mut pos = 0;
        for want in expected {
            let found = ids[pos..]
                .iter()
                .position(|id| id == want)
                .unwrap_or_else(|| panic!("'{want}' missing after {:?}", &ids[..pos]));
            pos += found + 1;
        }
    }

    fn assert_dep(recipe: &Recipe, step: &str, prereq: &str) {
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq),
            "dependency {step} <- {prereq} is missing"
        );
    }

    fn command_of(step: &Step) -> (String, Vec<String>) {
        match step {
            Step::Command { command, args, .. } => (command.clone(), args.clone()),
            other => panic!("expected Command, got {:?}", other.id()),
        }
    }

    fn wd_of(step: &Step) -> String {
        match step {
            Step::Command { working_dir, .. } => working_dir
                .clone()
                .unwrap_or_else(|| panic!("working_dir unset")),
            other => panic!("expected Command, got {:?}", other.id()),
        }
    }

    fn write_path_of(step: &Step) -> String {
        match step {
            Step::WriteFile { path, .. } => path.clone(),
            other => panic!("expected WriteFile, got {:?}", other.id()),
        }
    }

    /// Все (путь, содержимое) записываемых движком файлов.
    fn written_files(recipe: &Recipe) -> Vec<(String, String)> {
        recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::WriteFile { path, content, .. } => Some((path.clone(), content.clone())),
                _ => None,
            })
            .collect()
    }

    fn assert_file_exists(step: &Step, path: &str) {
        match step.condition() {
            Some(StepCondition::FileExists { path: p }) => assert_eq!(p, path, "FileExists path"),
            other => panic!("expected FileExists({path}), got {other:?}"),
        }
    }

    fn assert_file_not_exists(step: &Step, path: &str) {
        match step.condition() {
            Some(StepCondition::FileNotExists { path: p }) => {
                assert_eq!(p, path, "FileNotExists path")
            }
            other => panic!("expected FileNotExists({path}), got {other:?}"),
        }
    }

    /// py_venv_create: Windows — исторический гейт FileNotExists по маркеру
    /// (повторный запуск не пересоздаёт venv); Unix — генератор python-venv
    /// без условия, который сам проверяет здоровье окружения; путь venv
    /// проверяется суффиксом (`venv` или `backend/venv`).
    fn assert_py_venv_create(recipe: &Recipe, marker: &str, venv_suffix: &str) {
        let step = find_step(recipe, "py_venv_create");
        if cfg!(target_os = "windows") {
            assert_file_not_exists(step, marker);
        } else {
            let venv_path = venv_path_of(step);
            assert!(
                venv_path.ends_with(venv_suffix),
                "venv path '{venv_path}' must end with '{venv_suffix}'"
            );
        }
    }

    fn gen_policy(step: &Step) -> Option<FilePolicy> {
        match step {
            Step::Generate { policy, .. } => *policy,
            other => panic!("expected Generate, got {:?}", other.id()),
        }
    }

    fn assert_no_npm_install(recipe: &Recipe) {
        assert!(
            !plan_ids(recipe)
                .iter()
                .any(|id| id.starts_with("npm_install")),
            "no JS framework — npm install must not be scheduled"
        );
    }

    #[test]
    fn validation_s1_nest_telegraf_nextjs_split_with_docker_services() {
        let mut ctx = ctx_scenario(
            &["typescript"],
            &["nest", "telegraf", "nextjs"],
            &["postgresql", "redis"],
        );
        ctx.docker = true;
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");
        assert!(layout.eager_dirs().contains(&"backend".to_string()));
        assert!(layout.eager_dirs().contains(&"frontend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "nest_new",
                "nest_pkg_name",
                "nextjs_create",
                "nextjs_pkg_name",
                "telegraf_bot",
                "telegraf_pkg_patch",
                "env_example",
                "git_cleanup_nested",
                "git_init",
                "dockerfile",
                "docker_ignore",
                "docker_compose",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "npm_install_1",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "nest_pkg_name", "nest_new");
        assert_dep(&recipe, "telegraf_pkg_patch", "nest_new");

        // nest: npx @nestjs/cli new . внутри backend/, с interactive-ответом на пакетный менеджер.
        let (cmd, args) = command_of(find_step(&recipe, "nest_new"));
        assert_eq!(cmd, "npx");
        assert_eq!(
            args,
            vec![
                "--yes",
                "@nestjs/cli",
                "new",
                ".",
                "--package-manager",
                "npm",
                "--skip-install",
                "--skip-git",
            ]
        );
        assert_eq!(wd_of(find_step(&recipe, "nest_new")), "backend");
        let nest_pkg = find_step(&recipe, "nest_pkg_name");
        assert_file_exists(nest_pkg, "backend/package.json");
        assert_eq!(
            wd_of(nest_pkg),
            "backend",
            "package_name_patch работает в относительной backend/"
        );

        // telegraf: бот в backend/src/bot.js, патч package.json строго после nest.
        assert_eq!(
            write_path_of(find_step(&recipe, "telegraf_bot")),
            "backend/src/bot.js"
        );
        let patch = find_step(&recipe, "telegraf_pkg_patch");
        assert_file_exists(patch, "backend/package.json");
        assert_eq!(wd_of(patch), "backend");
        assert_eq!(command_of(patch).0, "node");
        assert!(command_of(patch).1[0] == "-e");

        // nextjs: scaffold-генератор в frontend/ с --skip-install.
        let nextjs = find_step(&recipe, "nextjs_create");
        let args = gen_args(gen_config_of_step(nextjs));
        assert!(
            args.contains(&"create-next-app@latest".to_string()),
            "{args:?}"
        );
        assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
        assert!(args.contains(&"--typescript".to_string()), "{args:?}");
        assert_eq!(
            gen_config_of_step(nextjs)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(gen_strs(gen_config_of_step(nextjs), "expected_outputs")
            .contains(&"package.json".to_string()));
        let nextjs_pkg = find_step(&recipe, "nextjs_pkg_name");
        assert_file_exists(nextjs_pkg, "frontend/package.json");
        assert_eq!(wd_of(nextjs_pkg), "frontend");

        // Инфра: .env.example + docker-compose с postgres и redis.
        let env = find_step(&recipe, "env_example");
        assert_eq!(write_path_of(env), ".env.example");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("POSTGRES_DB"), "{content}");
                assert!(content.contains("REDIS_URL"), "{content}");
            }
            _ => unreachable!(),
        }
        assert_eq!(
            write_path_of(find_step(&recipe, "dockerfile")),
            "backend/Dockerfile"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "docker_ignore")),
            "backend/.dockerignore"
        );
        let compose = find_step(&recipe, "docker_compose");
        assert_eq!(write_path_of(compose), "docker-compose.yaml");
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("postgres"), "{content}");
                assert!(content.contains("redis"), "{content}");
            }
            _ => unreachable!(),
        }

        // Один npm install на backend (nest) и один на frontend (nextjs).
        let (cmd0, args0) = command_of(find_step(&recipe, "npm_install_0"));
        assert_eq!(cmd0, "npm");
        assert_eq!(args0, vec!["install"]);
        assert_eq!(wd_of(find_step(&recipe, "npm_install_0")), "./backend");
        assert_file_not_exists(find_step(&recipe, "npm_install_0"), "backend/node_modules");
        assert_eq!(wd_of(find_step(&recipe, "npm_install_1")), "./frontend");
        assert_file_not_exists(find_step(&recipe, "npm_install_1"), "frontend/node_modules");
    }

    #[test]
    fn validation_s2_laravel_react_composer_scaffold() {
        let ctx = ctx_scenario(&["php", "typescript"], &["laravel", "react"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "laravel_php_check",
                "laravel_new",
                "vite_create",
                "react_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        // Языковые scaffold'ы подавлены: laravel сам создаёт PHP-каркас, react — TS.
        assert!(
            !plan_ids(&recipe).contains(&"composer_json".to_string()),
            "php scaffold suppressed by laravel"
        );
        assert!(!plan_ids(&recipe).contains(&"tsc_init".to_string()));

        // PHP-префлайт: fileinfo обязателен до composer create-project.
        let preflight = find_step(&recipe, "laravel_php_check");
        let (cmd, args) = command_of(preflight);
        assert_eq!(cmd, "php");
        assert_eq!(args[0], "-d");
        assert_eq!(args[1], "extension=fileinfo");
        assert_eq!(args[2], "-r");
        assert!(args[3].contains("fileinfo"));
        assert!(matches!(error_mode_of(preflight), ErrorMode::Abort));

        // Composer: create-project laravel/laravel в backend/ (temp+move, SkipIfExists).
        let laravel = find_step(&recipe, "laravel_new");
        let gen_args_v = gen_args(gen_config_of_step(laravel));
        assert!(
            gen_args_v.contains(&"create-project".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"laravel/laravel".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"--no-interaction".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"--prefer-source".to_string()),
            "{gen_args_v:?}"
        );
        assert_eq!(
            gen_config_of_step(laravel)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        // Composer-скаффолд обязан оставить НАСТОЯЩИЙ composer.json
        // (не generic-фолбэк), иначе каркас не считается успешным.
        assert!(
            gen_strs(gen_config_of_step(laravel), "expected_outputs")
                .contains(&"composer.json".to_string()),
            "laravel_new обязан ожидать composer.json, а не package.json"
        );
        assert!(
            !gen_strs(gen_config_of_step(laravel), "expected_outputs")
                .contains(&"package.json".to_string()),
            "laravel — PHP-каркас, package.json ему не нужен"
        );
        assert!(matches!(
            gen_policy(laravel),
            Some(FilePolicy::SkipIfExists)
        ));

        // React в frontend/ через create-vite (react-ts).
        let vite = find_step(&recipe, "vite_create");
        let vargs = gen_args(gen_config_of_step(vite));
        assert!(vargs.contains(&"react-ts".to_string()), "{vargs:?}");
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert_file_exists(
            find_step(&recipe, "react_pkg_name"),
            "frontend/package.json",
        );
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s3_django_vue_python_venv_chain() {
        let ctx = ctx_scenario(
            &["python", "typescript"],
            &["django", "vue"],
            &["alembic", "sqlalchemy"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "node_preflight",
                "npm_preflight",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "django_start",
                "vite_create",
                "vue_pkg_name",
                "alembic_init",
                "sqlalchemy_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "py_venv_verify", "py_venv_create");
        assert_dep(&recipe, "py_pip_upgrade", "py_venv_create");
        assert_dep(&recipe, "py_pip_install", "py_pip_upgrade");
        assert_dep(&recipe, "py_pip_install", "py_pip_check");
        assert_dep(&recipe, "py_requirements_check", "py_pip_install");
        assert_dep(&recipe, "alembic_init", "py_pip_install");
        assert_dep(&recipe, "django_start", "py_pip_install");

        // Python-каркас сегментирован в backend/.
        match find_step(&recipe, "create_src") {
            Step::CreateDirectory { path, .. } => assert_eq!(path, "backend/src"),
            _ => panic!("create_src — CreateDirectory"),
        }
        let reqs = find_step(&recipe, "requirements_txt");
        assert_eq!(write_path_of(reqs), "backend/requirements.txt");
        match reqs {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("django"), "{content}");
                assert!(content.contains("sqlalchemy"), "{content}");
                assert!(content.contains("alembic"), "{content}");
            }
            _ => unreachable!(),
        }

        // Ранний venv: python -m venv backend/venv (Windows) или генератор
        // python-venv (Unix), маркер root-relative. Никакого django-venv.
        let venv_create = find_step(&recipe, "py_venv_create");
        let venv_path = venv_path_of(venv_create);
        assert!(
            venv_path.ends_with("backend/venv") || venv_path.ends_with("backend\\venv"),
            "venv создаётся внутри backend/: {venv_path}"
        );
        if cfg!(target_os = "windows") {
            assert_file_not_exists(venv_create, "backend/venv/pyvenv.cfg");
        } else {
            match venv_create {
                Step::Generate { generator_id, .. } => assert_eq!(generator_id, "python-venv"),
                other => panic!("на Unix py_venv_create — Generate: {:?}", other.id()),
            }
        }
        assert!(matches!(error_mode_of(venv_create), ErrorMode::Abort));

        // django-admin строго из venv, в рабочей директории backend/.
        let start = find_step(&recipe, "django_start");
        assert!(
            command_of(start).0.contains("django-admin"),
            "{}",
            command_of(start).0
        );
        assert_eq!(command_of(start).1, vec!["startproject", "myapp", "."]);
        assert_eq!(wd_of(start), "backend");

        // tools-фаза: единый venv, pip ставит РОВНО из манифеста (-r),
        // alembic задекларирован в манифесте, а не отдельным pip-вызовом.
        let pip = find_step(&recipe, "py_pip_install");
        let (_, pip_args) = command_of(pip);
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        assert!(pip_args.iter().any(|a| a == "-r"), "{pip_args:?}");
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic ставится из манифеста: {pip_args:?}"
        );
        let requirements = find_step(&recipe, "requirements_txt");
        match requirements {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("alembic"), "{content}");
            }
            _ => unreachable!(),
        }
        let alembic = find_step(&recipe, "alembic_init");
        assert!(command_of(alembic).0.contains("alembic"));
        assert_eq!(command_of(alembic).1, vec!["init", "migrations"]);
        assert_eq!(wd_of(alembic), "./backend");

        // ИСПРАВЛЕНО в аудите 11.2: sqlalchemy_config сегментируется вместе
        // с python-частью — database.py лежит в backend/ рядом с
        // requirements.txt и venv (каталог python-сегмента).
        assert_eq!(
            write_path_of(find_step(&recipe, "sqlalchemy_config")),
            "backend/src/database.py"
        );

        assert_eq!(wd_of(find_step(&recipe, "npm_install_0")), "./frontend");
    }

    #[test]
    fn validation_s4_tauri_svelte_integrated() {
        let ctx = ctx_scenario(&["rust", "typescript"], &["tauri", "svelte"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "connected");
        assert!(
            layout.eager_dirs().is_empty(),
            "integrated — никаких eager-директорий"
        );

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "vite_create",
                "svelte_pkg_name",
                "tauri_init",
                "tauri_config_patch",
                "tauri_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "tauri_config_patch", "tauri_init");
        assert_dep(&recipe, "tauri_pkg_name", "vite_create");

        // Компаньон svelte: НЕ tauri_web_scaffold/install (только vite).
        assert!(
            !plan_ids(&recipe).contains(&"tauri_web_scaffold".to_string()),
            "companion suppresses tauri_web_scaffold"
        );
        assert!(!plan_ids(&recipe).contains(&"tauri_web_install".to_string()));

        // svelte-ts через create-vite в frontend/.
        let vite = find_step(&recipe, "vite_create");
        assert!(gen_args(gen_config_of_step(vite)).contains(&"svelte-ts".to_string()));
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );

        // tauri init в корне, гейт на frontend/package.json, пост-условия shell.
        let init = find_step(&recipe, "tauri_init");
        assert_file_exists(init, "frontend/package.json");
        assert!(matches!(gen_policy(init), Some(FilePolicy::SkipIfExists)));
        let iargs = gen_args(gen_config_of_step(init));
        assert!(iargs.contains(&"--ci".to_string()), "{iargs:?}");
        assert!(iargs.contains(&"--app-name".to_string()), "{iargs:?}");
        assert_eq!(
            gen_config_of_step(init)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some(".")
        );
        let expected = gen_strs(gen_config_of_step(init), "expected_outputs");
        assert!(
            expected.contains(&"src-tauri/tauri.conf.json".to_string()),
            "{expected:?}"
        );
        assert!(
            expected.contains(&"src-tauri/Cargo.toml".to_string()),
            "{expected:?}"
        );

        assert_file_exists(
            find_step(&recipe, "tauri_config_patch"),
            "src-tauri/tauri.conf.json",
        );
        assert_file_exists(
            find_step(&recipe, "tauri_pkg_name"),
            "frontend/package.json",
        );
        assert_eq!(wd_of(find_step(&recipe, "tauri_pkg_name")), "frontend");
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s5_fastapi_backend_only_with_tools() {
        let ctx = ctx_scenario(
            &["python"],
            &["fastapi"],
            &["alembic", "ruff", "sqlalchemy"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "backend-only");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "fastapi_main",
                "alembic_init",
                "ruff_config",
                "sqlalchemy_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);

        // Всё в корне: venv = ./venv, требования = ./requirements.txt.
        match find_step(&recipe, "create_src") {
            Step::CreateDirectory { path, .. } => assert_eq!(path, "src"),
            _ => panic!("create_src — CreateDirectory"),
        }
        assert_eq!(
            write_path_of(find_step(&recipe, "fastapi_main")),
            "src/main.py"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "sqlalchemy_config")),
            "src/database.py"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "ruff_config")),
            "ruff.toml"
        );
        assert_py_venv_create(&recipe, "venv/pyvenv.cfg", "venv");
        let (_, pip_args) = command_of(find_step(&recipe, "py_pip_install"));
        assert!(pip_args.iter().any(|a| a == "-r"), "{pip_args:?}");
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic ставится из манифеста: {pip_args:?}"
        );
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        let alembic_wd = wd_of(find_step(&recipe, "alembic_init"));
        assert_eq!(
            alembic_wd, ".",
            "alembic работает в корне проекта (backend-only): {alembic_wd}"
        );
        let reqs = find_step(&recipe, "requirements_txt");
        assert_eq!(write_path_of(reqs), "requirements.txt");
        match reqs {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("fastapi[standard]"), "{content}");
                assert!(content.contains("ruff"), "{content}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn validation_s6_zig_zap_flutter_prisma_drizzle() {
        let ctx = ctx_scenario(
            &["zig", "dart"],
            &["zig-cli", "zap", "flutter"],
            &["prisma", "drizzle"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "zig_init",
                "zig_cli_module",
                "zap_zon",
                "zap_build",
                "zap_fetch",
                "zap_main",
                "flutter_create",
                "prisma_init",
                "prisma_cleanup",
                "drizzle_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);
        assert!(
            !plan_ids(&recipe).contains(&"dart_create".to_string()),
            "flutter suppresses the dart scaffold"
        );

        // zig init: shell в backend/ (сегмент), SkipIfExists, пост-условия build.zig + zon.
        let zig_init = find_step(&recipe, "zig_init");
        assert!(matches!(
            gen_policy(zig_init),
            Some(FilePolicy::SkipIfExists)
        ));
        assert_eq!(
            gen_config_of_step(zig_init)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        let zexpected = gen_strs(gen_config_of_step(zig_init), "expected_outputs");
        assert!(
            zexpected.contains(&"build.zig".to_string()),
            "{zexpected:?}"
        );
        assert!(
            zexpected.contains(&"build.zig.zon".to_string()),
            "{zexpected:?}"
        );

        // zap (inplace-фреймворк): перезаписывает build.zig/zon в backend/.
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_zon")),
            "backend/build.zig.zon"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_build")),
            "backend/build.zig"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_main")),
            "backend/src/main.zig"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zig_cli_module")),
            "backend/src/cli.zig"
        );
        let (zcmd, zargs) = command_of(find_step(&recipe, "zap_fetch"));
        assert_eq!(zcmd, "zig");
        assert!(zargs.contains(&"fetch".to_string()), "{zargs:?}");
        assert!(zargs.contains(&"--save".to_string()), "{zargs:?}");
        assert_eq!(wd_of(find_step(&recipe, "zap_fetch")), "backend");

        // flutter create в frontend/ (creates_in_current_directory, SkipIfExists).
        let fl = find_step(&recipe, "flutter_create");
        let fargs = gen_args(gen_config_of_step(fl));
        assert!(fargs.contains(&"create".to_string()), "{fargs:?}");
        assert!(fargs.contains(&"--project-name".to_string()), "{fargs:?}");
        assert!(fargs.contains(&"myapp".to_string()), "{fargs:?}");
        assert_eq!(
            gen_config_of_step(fl)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(matches!(gen_policy(fl), Some(FilePolicy::SkipIfExists)));
        let fexpected = gen_strs(gen_config_of_step(fl), "expected_outputs");
        assert!(
            fexpected.contains(&"pubspec.yaml".to_string()),
            "{fexpected:?}"
        );
        assert!(fexpected.contains(&"lib".to_string()), "{fexpected:?}");

        // prisma init в корне (ИЗВЕСТНОЕ ОГРАНИЧЕНИЕ: без JS/TS-каркаса и package.json
        // в корне шаг упадёт на рантайме и будет молча пропущен через ErrorMode::Skip).
        let prisma = find_step(&recipe, "prisma_init");
        let (pcmd, pargs) = command_of(prisma);
        assert_eq!(pcmd, "npx");
        assert!(pargs.contains(&"prisma@6".to_string()), "{pargs:?}");
        assert!(pargs.contains(&"init".to_string()), "{pargs:?}");
        assert!(
            pargs.contains(&"--datasource-provider".to_string()),
            "{pargs:?}"
        );
        assert!(pargs.contains(&"sqlite".to_string()), "{pargs:?}");
        let prisma_wd = wd_of(prisma);
        assert_eq!(
            prisma_wd, ".",
            "prisma init работает в корне проекта: {prisma_wd}"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "drizzle_config")),
            "drizzle.config.ts"
        );
    }

    #[test]
    fn validation_s7_symfony_nuxt() {
        let ctx = ctx_scenario(&["php", "typescript"], &["symfony", "nuxt"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "symfony_php_check",
                "symfony_new",
                "nuxt_create",
                "nuxt_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert!(!plan_ids(&recipe).contains(&"composer_json".to_string()));

        let symfony = find_step(&recipe, "symfony_new");
        let sargs = gen_args(gen_config_of_step(symfony));
        assert!(sargs.contains(&"create-project".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"symfony/skeleton".to_string()), "{sargs:?}");
        assert_eq!(
            gen_config_of_step(symfony)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        assert!(matches!(
            gen_policy(symfony),
            Some(FilePolicy::SkipIfExists)
        ));

        // nuxi init: неинтерактивен только с полным набором флагов.
        let nuxt = find_step(&recipe, "nuxt_create");
        let nargs = gen_args(gen_config_of_step(nuxt));
        assert!(nargs.contains(&"nuxi@latest".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"init".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--template".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--packageManager".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--gitInit=false".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--no-install".to_string()), "{nargs:?}");
        assert_eq!(
            gen_config_of_step(nuxt)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert_file_exists(find_step(&recipe, "nuxt_pkg_name"), "frontend/package.json");
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s8_qt_webengine_vue() {
        let mut ctx = ctx_scenario(&["cpp", "typescript"], &["qt", "vue"], &[]);
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");
        assert!(layout.eager_dirs().contains(&"backend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "qt_main",
                "qt_cmake",
                "qt_web_build",
                "qt_cmake_configure",
                "qt_cmake_build",
                "vite_create",
                "vue_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "qt_web_build", "vite_create");
        assert_dep(&recipe, "qt_cmake_configure", "qt_cmake");
        assert_dep(&recipe, "qt_cmake_build", "qt_web_build");
        assert_dep(&recipe, "qt_cmake_build", "qt_cmake_configure");
        // qt_cmake_build дополнительно требует собранный frontend.
        assert!(
            recipe.dependencies.iter().any(|d| {
                d.step_id == "qt_cmake_build"
                    && d.prereq_id == "qt_web_build"
                    && d.expects_file == "frontend/dist/index.html"
            }),
            "qt_cmake_build <- qt_web_build должен нести expects_file"
        );

        // Qt (inplace): main.cpp + CMakeLists в backend/, перезапись поверх zig/c++ каркаса.
        assert_eq!(
            write_path_of(find_step(&recipe, "qt_main")),
            "backend/src/main.cpp"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "qt_cmake")),
            "backend/CMakeLists.txt"
        );

        // Цепочка сборки WebEngine: веб-часть в frontend/, cmake в backend/.
        let web_build = find_step(&recipe, "qt_web_build");
        let (wcmd, wargs) = command_of(web_build);
        assert_eq!(wcmd, "npm");
        assert_eq!(wargs, vec!["run", "build"]);
        assert_eq!(wd_of(web_build), "frontend");
        assert_file_exists(web_build, "frontend/package.json");
        assert!(matches!(error_mode_of(web_build), ErrorMode::Skip));

        let configure = find_step(&recipe, "qt_cmake_configure");
        assert_eq!(command_of(configure).0, "cmake");
        assert_eq!(command_of(configure).1, vec!["-S", ".", "-B", "build"]);
        assert_eq!(wd_of(configure), "backend", "cmake работает в qt-сегменте");
        assert_file_exists(configure, "backend/CMakeLists.txt");
        assert!(matches!(error_mode_of(configure), ErrorMode::Abort));

        let build = find_step(&recipe, "qt_cmake_build");
        assert_eq!(command_of(build).0, "cmake");
        assert_eq!(command_of(build).1, vec!["--build", "build"]);
        assert_eq!(wd_of(build), "backend");
        // Root-relative пост-условие: собранный фронтенд, а НЕ backend/frontend/...
        assert_file_exists(build, "frontend/dist/index.html");
        assert!(matches!(error_mode_of(build), ErrorMode::Abort));

        // vue в frontend/ (vue-ts), финальный npm install.
        let vite = find_step(&recipe, "vite_create");
        assert!(gen_args(gen_config_of_step(vite)).contains(&"vue-ts".to_string()));
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s9_go_gin_cobra_solidjs() {
        let ctx = ctx_scenario(
            &["go", "typescript"],
            &["gin", "cobra", "solidjs"],
            &["mongodb"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "go_mod_init",
                "create_cmd",
                "create_internal",
                "main_go",
                "gin_main",
                "get_gin",
                "get_cobra",
                "cobra_cli",
                "solid_init",
                "solidjs_pkg_name",
                "env_example",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "get_gin", "go_mod_init");
        assert_dep(&recipe, "get_cobra", "go_mod_init");

        // go mod init живёт в backend/ вместе со своим маркером (rerun-guard сегментирован).
        let go_mod = find_step(&recipe, "go_mod_init");
        assert_eq!(command_of(go_mod).0, "go");
        assert_eq!(command_of(go_mod).1, vec!["mod", "init", "myapp"]);
        assert_eq!(wd_of(go_mod), "backend");
        assert_file_not_exists(go_mod, "backend/go.mod");

        assert_eq!(
            write_path_of(find_step(&recipe, "main_go")),
            "backend/cmd/main.go"
        );
        let gin_main = find_step(&recipe, "gin_main");
        assert_eq!(write_path_of(gin_main), "backend/cmd/main.go");
        assert!(matches!(
            step_file_policy_of(gin_main),
            FilePolicy::Overwrite
        ));
        let get_gin = find_step(&recipe, "get_gin");
        assert_file_exists(get_gin, "backend/go.mod");
        assert!(matches!(error_mode_of(get_gin), ErrorMode::Abort));

        // cobra: реальная зависимость через go get @latest (go.mod обновляет
        // сам go get, WriteFile-заглушки go.mod больше нет) — FileExists
        // go.mod + Abort.
        assert_file_exists(find_step(&recipe, "get_cobra"), "backend/go.mod");
        assert_eq!(
            write_path_of(find_step(&recipe, "cobra_cli")),
            "backend/cmd/cli/main.go"
        );

        // solidjs: create-solid в frontend/, полный набор флагов для неинтерактивности.
        let solid = find_step(&recipe, "solid_init");
        let sargs = gen_args(gen_config_of_step(solid));
        assert!(sargs.contains(&"--yes".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"create-solid".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"basic".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--solidstart".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--v2".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--ts".to_string()), "{sargs:?}");
        assert_eq!(
            gen_config_of_step(solid)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );

        let env = find_step(&recipe, "env_example");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("MONGODB_URI"), "{content}");
            }
            _ => panic!("env_example — WriteFile"),
        }
        assert_eq!(wd_of(find_step(&recipe, "npm_install_0")), "./frontend");
    }

    #[test]
    fn validation_s10_frontend_only_nextjs() {
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "separated");
        assert!(
            layout.eager_dirs().contains(&"backend".to_string()),
            "nextjs-раскладка держит backend/"
        );
        assert!(
            layout.eager_dirs().contains(&"frontend".to_string()),
            "nextjs-раскладка держит frontend/"
        );

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "nextjs_create",
                "nextjs_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        let nextjs = find_step(&recipe, "nextjs_create");
        assert_eq!(
            gen_config_of_step(nextjs)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend"),
            "special case: nextjs в frontend-only раскладке — frontend/"
        );
        assert_file_exists(
            find_step(&recipe, "nextjs_pkg_name"),
            "frontend/package.json",
        );
        assert_eq!(wd_of(find_step(&recipe, "nextjs_pkg_name")), "frontend");
        assert_eq!(wd_of(find_step(&recipe, "npm_install_0")), "./frontend");
    }

    #[test]
    fn validation_s11_backend_only_fastapi_plain() {
        let ctx = ctx_scenario(&["python"], &["fastapi"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "backend-only");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "fastapi_main",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);
        // Без инструмента alembic в pip-установке только -r requirements.txt.
        let (_, pip_args) = command_of(find_step(&recipe, "py_pip_install"));
        assert!(!pip_args.contains(&"alembic".to_string()), "{pip_args:?}");
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        assert_file_not_exists(find_step(&recipe, "git_init"), ".git/HEAD");
        // git_commit идемпотентен при повторном запуске.
        let (commit_cmd, _) = command_of(find_step(&recipe, "git_commit"));
        assert!(
            commit_cmd.contains("git diff --cached --quiet"),
            "{commit_cmd}"
        );
    }

    #[test]
    fn validation_s12_tauri_only_rust_vite_vanilla() {
        let ctx = ctx_scenario(&["rust"], &["tauri"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "connected");
        assert!(!layout.eager_dirs().contains(&"backend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "tauri_web_scaffold",
                "tauri_web_install",
                "tauri_init",
                "tauri_config_patch",
                "tauri_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "tauri_web_install", "tauri_web_scaffold");
        assert_dep(&recipe, "tauri_pkg_name", "tauri_web_scaffold");
        // tauri — не JS-фреймворк (langs=[rust]): отдельного npm install НЕТ.
        assert_no_npm_install(&recipe);

        // Без компаньона фронтенд создаёт vite (vanilla), SkipIfExists.
        let web = find_step(&recipe, "tauri_web_scaffold");
        let wargs = gen_args(gen_config_of_step(web));
        assert!(
            wargs.contains(&"create-vite@latest".to_string()),
            "{wargs:?}"
        );
        assert!(wargs.contains(&"--template".to_string()), "{wargs:?}");
        assert!(wargs.contains(&"vanilla".to_string()), "{wargs:?}");
        assert_eq!(
            gen_config_of_step(web)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(matches!(gen_policy(web), Some(FilePolicy::SkipIfExists)));

        // npm install строго после появления frontend/package.json.
        let install = find_step(&recipe, "tauri_web_install");
        assert_eq!(command_of(install).0, "npm");
        assert_eq!(wd_of(install), "frontend");
        assert_file_exists(install, "frontend/package.json");

        assert_file_exists(find_step(&recipe, "tauri_init"), "frontend/package.json");
        assert_file_exists(
            find_step(&recipe, "tauri_config_patch"),
            "src-tauri/tauri.conf.json",
        );
        assert_file_exists(
            find_step(&recipe, "tauri_pkg_name"),
            "frontend/package.json",
        );
    }

    #[test]
    fn validation_segment_markers_follow_working_dir() {
        // Rerun-guard маркеры языковых init-шагов относительны рабочей директории:
        // в Split-раскладке они обязаны уехать в сегмент вместе с командой.
        let ctx = ctx_scenario(&["rust", "typescript"], &["axum"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(
            layout.to_summary(&ctx).class,
            "backend-only",
            "rust+ts+axum: бэкенд-фреймворк без клиентского SPA — монолит в корне"
        );
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "cargo_init"), "Cargo.toml");
        assert_eq!(wd_of(find_step(&recipe, "cargo_init")), ".");
        assert_file_not_exists(find_step(&recipe, "tsc_init"), "tsconfig.json");
        assert_eq!(wd_of(find_step(&recipe, "tsc_init")), ".");

        let ctx = ctx_scenario(&["go", "typescript"], &["solidjs"], &[]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "go_mod_init"), "backend/go.mod");

        let ctx = ctx_scenario(&["dart", "typescript"], &["vue"], &[]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(
            find_step(&recipe, "dart_create"),
            "backend/myapp/pubspec.yaml",
        );

        // Монолит: маркер остаётся в корне (поведение без изменений).
        let mono = ctx_scenario(&["go"], &[], &[]);
        let recipe = recipe_for(&mono, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "go_mod_init"), "go.mod");
        let mono_wd = wd_of(find_step(&recipe, "go_mod_init"));
        assert_eq!(
            mono_wd, ".",
            "монолит: go mod init работает в корне проекта: {mono_wd}"
        );
    }

    #[test]
    fn validation_all_scenario_commands_have_timeouts() {
        // Ни один Command в рецептах не должен остаться без таймаута (защита от зависаний).
        let qt_ctx = ctx_scenario(&["cpp", "typescript"], &["qt", "vue"], &[]);
        let scenarios: Vec<WizardContext> = vec![
            ctx_scenario(
                &["typescript"],
                &["nest", "telegraf", "nextjs"],
                &["postgresql", "redis"],
            ),
            ctx_scenario(&["php", "typescript"], &["laravel", "react"], &[]),
            ctx_scenario(
                &["python", "typescript"],
                &["django", "vue"],
                &["alembic", "sqlalchemy"],
            ),
            ctx_scenario(&["rust", "typescript"], &["tauri", "svelte"], &[]),
            ctx_scenario(
                &["python"],
                &["fastapi"],
                &["alembic", "ruff", "sqlalchemy"],
            ),
            ctx_scenario(
                &["zig", "dart"],
                &["zig-cli", "zap", "flutter"],
                &["prisma", "drizzle"],
            ),
            ctx_scenario(&["php", "typescript"], &["symfony", "nuxt"], &[]),
            qt_ctx,
            ctx_scenario(
                &["go", "typescript"],
                &["gin", "cobra", "solidjs"],
                &["mongodb"],
            ),
            ctx_scenario(&["typescript"], &["nextjs"], &[]),
            ctx_scenario(&["python"], &["fastapi"], &[]),
            ctx_scenario(&["rust"], &["tauri"], &[]),
        ];
        for ctx in scenarios {
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            for step in &recipe.steps {
                match step {
                    Step::Command {
                        id, timeout_secs, ..
                    } => {
                        assert!(
                            timeout_secs.is_some(),
                            "Command '{id}' has no timeout in a recipe"
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn gen_config_of_step(step: &Step) -> &serde_json::Value {
        match step {
            Step::Generate {
                generator_config, ..
            } => generator_config,
            other => panic!("expected Generate, got {:?}", other.id()),
        }
    }

    fn error_mode_of(step: &Step) -> ErrorMode {
        match step {
            Step::Command { on_error, .. } => on_error.clone(),
            Step::Generate { on_error, .. } => on_error.clone(),
            other => panic!("expected Command/Generate, got {:?}", other.id()),
        }
    }

    fn step_file_policy_of(step: &Step) -> FilePolicy {
        step.file_policy().expect("file step must have a policy")
    }

    #[tokio::test]
    async fn engine_failed_step_reports_stdout_and_stderr_tails() {
        let engine = DefaultRecipeEngine::new();
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_err_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (command, args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec![
                    "/d".into(),
                    "/c".into(),
                    "echo out-line && echo err-line 1>&2 && exit 1".into(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec![
                    "-c".into(),
                    "echo out-line; echo err-line 1>&2; exit 1".into(),
                ],
            )
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::Command {
                id: "failing".into(),
                label: "Failing step".into(),
                description: String::new(),
                command,
                args,
                working_dir: None,
                env: None,
                timeout_secs: Some(10),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        match &result.step_results[0].status {
            StepStatus::Failed { error } => {
                assert!(error.contains("out-line"), "stdout tail missing: {error}");
                assert!(error.contains("err-line"), "stderr tail missing: {error}");
                assert!(error.contains("command: "), "command line missing: {error}");
                assert!(error.contains("working directory"), "{error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_create_root_step_with_dot_path_succeeds() {
        // Регрессия: первый шаг каждого рецепта — CreateDirectory path="."
        // («Create project root»). paths::resolve_in_root(".") раньше
        // возвращал None, и шаг падал с «escapes the project root» на
        // любой генерации — проект даже не начинал создаваться.
        let engine = DefaultRecipeEngine::new();
        let dir =
            std::env::temp_dir().join(format!("stackpilot_create_root_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::CreateDirectory {
                id: "create_root".into(),
                label: "Create project root".into(),
                description: String::new(),
                path: ".".into(),
                condition: None,
                on_error: ErrorMode::Abort,
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx, no_cancel()).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Success { .. }),
            "CreateDirectory path='.' обязан успешно создать корень: {:?}",
            result.step_results[0].status
        );
        assert!(dir.is_dir(), "корень проекта должен существовать");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============================================================
    // Audit 11.2: каждый выбираемый в мастере фреймворк/инструмент
    // обязан иметь реальную реализацию — ни echo-заглушек, ни
    // config-hint .md, ни хардкод-секретов, ни «тихих» скипов.
    // ============================================================

    fn wizard_framework_ids() -> Vec<(String, String)> {
        let tree: serde_json::Value =
            serde_json::from_str(include_str!("../knowledge/wizard_tree.json")).unwrap();
        tree["frameworks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|fw| {
                let id = fw["id"].as_str().unwrap().to_string();
                let lang = fw["languages"][0]
                    .as_str()
                    .unwrap_or("typescript")
                    .to_string();
                (id, lang)
            })
            .collect()
    }

    fn wizard_tool_ids() -> Vec<String> {
        let tree: serde_json::Value =
            serde_json::from_str(include_str!("../knowledge/wizard_tree.json")).unwrap();
        tree["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn audit_every_selectable_framework_builds_a_real_recipe() {
        for (fw, lang) in wizard_framework_ids() {
            let ctx = ctx_scenario(&[lang.as_str()], &[fw.as_str()], &[]);
            let recipe =
                recipe_for(&ctx, "myapp").unwrap_or_else(|e| panic!("framework '{fw}': {e}"));
            let ids = plan_ids(&recipe);
            assert!(
                !ids.contains(&"fw_unknown".to_string()),
                "framework '{fw}' hits the echo fallback: {ids:?}"
            );
            assert!(
                ids.iter()
                    .any(|id| id.contains("readme") || id != "fw_unknown"),
                "framework '{fw}' produces an empty plan: {ids:?}"
            );
        }
    }

    #[test]
    fn audit_no_echo_or_config_hint_placeholders_anywhere() {
        for (fw, lang) in wizard_framework_ids() {
            let ctx = ctx_scenario(&[lang.as_str()], &[fw.as_str()], &[]);
            let recipe = recipe_for(&ctx, "myapp").unwrap();
            for (path, content) in written_files(&recipe) {
                assert!(
                    !content.contains("See documentation for setup details"),
                    "framework '{fw}': config-hint placeholder in {path}"
                );
                assert!(
                    !content.contains("No automated setup available"),
                    "framework '{fw}': echo placeholder leaked into {path}"
                );
            }
        }
        for tool in wizard_tool_ids() {
            let ctx = ctx_scenario(&["typescript"], &["nextjs"], &[tool.as_str()]);
            let recipe = recipe_for(&ctx, "myapp").unwrap();
            let ids = plan_ids(&recipe);
            assert!(
                !ids.iter().any(|id| id.starts_with("config_dir_")),
                "tool '{tool}' falls into the defensive config/ fallback: {ids:?}"
            );
            for (path, content) in written_files(&recipe) {
                assert!(
                    !content.contains("See documentation for setup details"),
                    "tool '{tool}': config-hint placeholder in {path}"
                );
            }
        }
    }

    #[test]
    fn audit_cobra_gets_real_go_dependency() {
        let ctx = ctx_scenario(&["go"], &["cobra"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"get_cobra".to_string()), "{ids:?}");
        assert!(!ids.contains(&"cobra_init".to_string()), "{ids:?}");
        let (cmd, args) = command_of(find_step(&recipe, "get_cobra"));
        assert_eq!(cmd, "go");
        assert!(
            args.iter().any(|a| a == "github.com/spf13/cobra@latest"),
            "{args:?}"
        );
        assert_dep(&recipe, "get_cobra", "go_mod_init");
    }

    #[test]
    fn audit_ktor_has_gradle_build_system() {
        let ctx = ctx_scenario(&["kotlin"], &["ktor"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"ktor_gradle_wrapper".to_string()), "{ids:?}");
        assert!(ids.contains(&"ktor_deps_check".to_string()), "{ids:?}");
        let (cmd, _) = command_of(find_step(&recipe, "ktor_gradle_wrapper"));
        assert_eq!(cmd, "gradle");
        assert_eq!(
            error_mode_of(find_step(&recipe, "ktor_gradle_wrapper")),
            ErrorMode::Abort
        );
        let files = written_files(&recipe);
        assert!(
            files
                .iter()
                .any(|(p, _)| p.ends_with("settings.gradle.kts")),
            "ktor must write settings.gradle.kts, got {:?}",
            files.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
        assert!(
            files.iter().any(|(p, c)| {
                p.ends_with("build.gradle.kts") && c.contains("io.ktor:ktor-server-core")
            }),
            "build.gradle.kts must declare ktor-server-core"
        );
        assert_dep(&recipe, "ktor_deps_check", "ktor_gradle_wrapper");
    }

    #[test]
    fn audit_swiftui_is_real_swiftpm_package() {
        let ctx = ctx_scenario(&["swift"], &["swiftui"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let files = written_files(&recipe);
        assert!(
            files
                .iter()
                .any(|(p, c)| p.ends_with("Package.swift") && c.contains("swift-tools-version")),
            "swiftui must write a real Package.swift"
        );
        assert!(
            files.iter().any(
                |(p, _)| p.ends_with("/Sources/") && p.ends_with("App.swift")
                    || p.ends_with("/App.swift")
            ),
            "swiftui must write Sources/<name>/App.swift"
        );
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"swiftui_build".to_string()), "{ids:?}");
        let (cmd, args) = command_of(find_step(&recipe, "swiftui_build"));
        assert_eq!(cmd, "swift");
        assert_eq!(args, vec!["build"]);
        assert_eq!(
            error_mode_of(find_step(&recipe, "swiftui_build")),
            ErrorMode::Abort
        );
        // пакет живёт в frontend/ (swiftui — frontend-сторона) — сборка там
        // же, сегмент применяется РОВНО ОДИН раз (без frontend/frontend).
        let wd = wd_of(find_step(&recipe, "swiftui_build"));
        assert_eq!(wd, "frontend", "{wd}");
    }

    #[test]
    fn audit_scaffold_frameworks_validate_expected_outputs() {
        // vapor: mix/vapor new + пост-валидация Package.swift в каталоге
        // назначения (имя каркаса = имя папки, не temp_*).
        let ctx = ctx_scenario(&["swift"], &["vapor"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let vapor = find_step(&recipe, "vapor_new");
        let cfg = gen_config_of_step(vapor);
        let expects = cfg["expected_outputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            expects.iter().any(|v| v == "Package.swift"),
            "vapor scaffold must expect Package.swift: {cfg}"
        );
        assert!(
            cfg["temp_dir_allowed"] == false,
            "vapor must use the destination dir name: {cfg}"
        );

        // phoenix: mix phx.new + пост-валидация mix.exs.
        let ctx = ctx_scenario(&["elixir"], &["phoenix"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let phoenix = find_step(&recipe, "phoenix_new");
        let cfg = gen_config_of_step(phoenix);
        let expects = cfg["expected_outputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            expects.iter().any(|v| v == "mix.exs"),
            "phoenix scaffold must expect mix.exs: {cfg}"
        );

        // react-native / plasmo: CLI-каркасы с пост-валидацией package.json.
        let ctx = ctx_scenario(&["typescript"], &["react-native"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let rn = find_step(&recipe, "rn_init");
        let cfg = gen_config_of_step(rn);
        let expects = cfg["expected_outputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            expects.iter().any(|v| v == "package.json"),
            "react-native scaffold must expect package.json: {cfg}"
        );
        let ids = plan_ids(&recipe);
        assert!(
            ids.iter().any(|id| id.starts_with("npm_install")),
            "react-native needs a final npm install: {ids:?}"
        );

        let ctx = ctx_scenario(&["typescript"], &["plasmo"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let plasmo = find_step(&recipe, "plasmo_init");
        let cfg = gen_config_of_step(plasmo);
        let expects = cfg["expected_outputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            expects.iter().any(|v| v == "package.json"),
            "plasmo scaffold must expect package.json: {cfg}"
        );
    }

    #[test]
    fn audit_telegram_bots_read_token_from_env() {
        let ctx = ctx_scenario(&["python"], &["aiogram"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let (path, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "src/bot.py")
            .expect("aiogram must write src/bot.py");
        assert!(
            content.contains("TELEGRAM_BOT_TOKEN") && content.contains("os.getenv"),
            "aiogram bot.py must read TELEGRAM_BOT_TOKEN from env: {path}"
        );
        assert!(
            !content.contains("YOUR_BOT_TOKEN"),
            "aiogram bot.py must not hardcode the token"
        );
        let env = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == ".env.example")
            .expect(".env.example must exist for aiogram");
        assert!(env.1.contains("TELEGRAM_BOT_TOKEN"), "{:?}", env.1);

        let ctx = ctx_scenario(&["typescript"], &["telegraf"], &["nest"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let (_, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "src/bot.js")
            .expect("telegraf must write src/bot.js");
        assert!(
            content.contains("process.env.TELEGRAM_BOT_TOKEN"),
            "telegraf bot.js must read TELEGRAM_BOT_TOKEN from env"
        );
        assert!(
            content.contains("dotenv"),
            "telegraf bot.js must load dotenv"
        );
        let env = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == ".env.example")
            .expect(".env.example must exist for telegraf");
        assert!(env.1.contains("TELEGRAM_BOT_TOKEN"), "{:?}", env.1);
    }

    #[test]
    fn audit_tools_write_real_configs() {
        // grafana: provisioning-конфиг вместо .md-хинта.
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &["grafana", "postgresql"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(
            written_files(&recipe).iter().any(|(p, c)| p
                == "config/grafana/provisioning/datasources/datasources.yaml"
                && c.contains("type: postgres")),
            "grafana must provision a postgres datasource"
        );

        // opentelemetry: реальный конфиг коллектора, монтируемый в compose.
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &["opentelemetry"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let (path, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "config/otel-collector.yaml")
            .expect("opentelemetry must write config/otel-collector.yaml");
        assert!(
            content.contains("receivers:") && content.contains("otlp"),
            "{path}"
        );
        let compose = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "docker-compose.yaml" || p == "docker-compose.yml");
        assert!(
            compose.is_some(),
            "opentelemetry must trigger docker-compose generation"
        );

        // dbt: реальный dbt-проект + пост-валидация yaml.
        let ctx = ctx_scenario(&["python"], &["fastapi"], &["dbt"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(plan_ids(&recipe).contains(&"dbt_project_check".to_string()));
        let (_, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "dbt_project.yml")
            .expect("dbt must write dbt_project.yml");
        assert!(content.contains("name:"), "{content}");
        assert!(
            written_files(&recipe)
                .iter()
                .any(|(p, _)| p == "models/example.sql"),
            "dbt must write a model"
        );

        // terraform: real main.tf + init + пост-валидация.
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &["terraform"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"terraform_init".to_string()), "{ids:?}");
        assert!(ids.contains(&"terraform_check".to_string()), "{ids:?}");
        let (_, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "terraform/main.tf")
            .expect("terraform must write terraform/main.tf");
        assert!(content.contains("provider \"docker\""), "{content}");

        // firebase: firebase.json + rules + пост-валидация.
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &["firebase"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(plan_ids(&recipe).contains(&"firebase_check".to_string()));
        let files = written_files(&recipe);
        assert!(
            files.iter().any(|(p, _)| p == "firebase.json"),
            "firebase must write firebase.json"
        );
        assert!(
            files.iter().any(|(p, _)| p == "firestore.rules"),
            "firebase must write firestore.rules"
        );
    }

    #[test]
    fn audit_sqlalchemy_uses_env_database_url() {
        let ctx = ctx_scenario(&["python"], &["fastapi"], &["sqlalchemy"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let (path, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == "src/database.py")
            .expect("sqlalchemy must write src/database.py");
        assert!(content.contains("os.getenv(\"DATABASE_URL\")"), "{content}");
        assert!(
            !content.contains("user:password"),
            "sqlalchemy must not hardcode credentials: {path}"
        );
        // без postgresql инструмент обязан добавить DATABASE_URL в .env.example
        let env = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p == ".env.example")
            .expect(".env.example must exist");
        assert!(env.1.contains("DATABASE_URL"), "{:?}", env.1);
    }

    #[test]
    fn audit_no_hardcoded_secrets_in_generated_content() {
        let scenarios: Vec<Vec<&str>> = vec![
            vec!["postgresql"],
            vec!["mysql"],
            vec!["postgresql", "redis", "mongodb"],
        ];
        for tools in scenarios {
            let ctx = ctx_scenario(&["typescript"], &["nextjs"], &tools);
            let recipe = recipe_for(&ctx, "myapp").unwrap();
            for (path, content) in written_files(&recipe) {
                assert!(
                    !content.contains("user:password"),
                    "placeholder credentials leaked into {path} ({tools:?})"
                );
            }
        }
    }

    #[test]
    fn audit_android_validates_build_gradle() {
        let ctx = ctx_scenario(&["kotlin"], &["android"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(
            ids.contains(&"android_gradle_wrapper".to_string()),
            "{ids:?}"
        );
        assert!(ids.contains(&"android_build_check".to_string()), "{ids:?}");

        let ctx = ctx_scenario(&["kotlin"], &["jetpack-compose"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"android_build_check".to_string()), "{ids:?}");
        let (_, content) = written_files(&recipe)
            .into_iter()
            .find(|(p, _)| p.ends_with("app/build.gradle.kts"))
            .expect("compose must write app/build.gradle.kts");
        assert!(content.contains("androidx.compose.ui:ui"), "{content}");
    }

    #[test]
    fn audit_dotnet_frameworks_validate_csproj() {
        let ctx = ctx_scenario(&["csharp"], &["aspnetcore"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"aspnet_new".to_string()), "{ids:?}");
        assert!(ids.contains(&"aspnet_csproj_check".to_string()), "{ids:?}");
        assert_eq!(
            error_mode_of(find_step(&recipe, "aspnet_new")),
            ErrorMode::Abort
        );

        let ctx = ctx_scenario(&["csharp"], &["maui"], &[]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"maui_new".to_string()), "{ids:?}");
        assert!(ids.contains(&"maui_csproj_check".to_string()), "{ids:?}");
        assert_eq!(
            error_mode_of(find_step(&recipe, "maui_new")),
            ErrorMode::Abort
        );
    }

    #[test]
    fn audit_docker_tool_alone_triggers_compose() {
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &["docker"]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let ids = plan_ids(&recipe);
        assert!(ids.contains(&"docker_compose".to_string()), "{ids:?}");
        assert!(ids.contains(&"dockerfile".to_string()), "{ids:?}");
    }

    #[test]
    fn audit_unknown_framework_is_rejected_explicitly() {
        let ctx = ctx_scenario(&["typescript"], &["not-a-real-framework"], &[]);
        let err = recipe_for(&ctx, "myapp").expect_err("unknown framework must be rejected");
        assert!(
            err.contains("not-a-real-framework") && err.contains("not supported"),
            "{err}"
        );
    }
