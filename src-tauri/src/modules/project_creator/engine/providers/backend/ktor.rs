use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct KtorProvider;

impl RecipeProvider for KtorProvider {
    fn id(&self) -> &'static str {
        "ktor"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let safe_name = project_name.replace(['-', ' '], "_");
        let build_path = match segment {
            Some(dir) => format!("{}/build.gradle.kts", dir),
            None => "build.gradle.kts".to_string(),
        };

        vec![
            write_file(
                "ktor_settings",
                "Create Gradle settings",
                "settings.gradle.kts",
                &format!(
                    "rootProject.name = \"{}\"\n",
                    safe_name.replace('"', "_")
                ),
            ),
            write_file(
                "ktor_build",
                "Create Gradle build file",
                "build.gradle.kts",
                r#"plugins {
    kotlin("jvm") version "2.0.21"
    application
}

group = "app"
version = "0.1.0"

repositories {
    mavenCentral()
}

val ktorVersion = "3.0.3"

dependencies {
    implementation("io.ktor:ktor-server-core:$ktorVersion")
    implementation("io.ktor:ktor-server-netty:$ktorVersion")
}

application {
    mainClass.set("MainKt")
}

kotlin {
    jvmToolchain(21)
}
"#,
            ),
            write_file(
                "ktor_main",
                "Create Ktor entry",
                "src/main/kotlin/Main.kt",
                &format!(
                    r#"import io.ktor.server.application.*
import io.ktor.server.engine.*
import io.ktor.server.netty.*
import io.ktor.server.response.*
import io.ktor.server.routing.*

fun main() {{
    embeddedServer(Netty, port = 3000) {{
        routing {{
            get("/") {{
                call.respondText("Hello from {}!")
            }}
        }}
    }}.start(wait = true)
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "ktor_gradle_wrapper".into(),
                label: "Generate Gradle wrapper".into(),
                description: "Create gradlew + gradle/wrapper (requires Gradle installed; the project builds with ./gradlew run)".into(),
                command: "gradle".into(),
                args: vec!["wrapper".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: None,
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
            preflight::manifest_check_step(
                "ktor_deps_check",
                "Validate Ktor dependencies",
                &build_path,
                "gradle_kts",
                &["io.ktor:ktor-server-core", "io.ktor:ktor-server-netty"],
            ),
        ]
    }
}
