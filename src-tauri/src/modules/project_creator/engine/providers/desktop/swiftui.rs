use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct SwiftuiProvider;

impl RecipeProvider for SwiftuiProvider {
    fn id(&self) -> &'static str {
        "swiftui"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let safe_name: String = project_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let mut type_name: String = project_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
            .collect();
        if type_name.is_empty() || type_name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            type_name = format!("App{}", type_name);
        }
        vec![
            write_file(
                "swiftui_manifest",
                "Create Swift package manifest",
                "Package.swift",
                &format!(
                    r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "{}",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "{}",
            path: "Sources/{}"
        )
    ]
)
"#,
                    safe_name, safe_name, safe_name
                ),
            ),
            write_file(
                "swiftui_app",
                "Create SwiftUI app entry",
                &format!("Sources/{}/App.swift", safe_name),
                &format!(
                    r#"import SwiftUI

@main
struct {}App: App {{
    var body: some Scene {{
        WindowGroup {{
            ContentView()
        }}
    }}
}}
"#,
                    type_name
                ),
            ),
            write_file(
                "swiftui_view",
                "Create SwiftUI content view",
                &format!("Sources/{}/ContentView.swift", safe_name),
                &format!(
                    r#"import SwiftUI

struct ContentView: View {{
    var body: some View {{
        VStack(spacing: 16) {{
            Text("Hello from {}!")
                .font(.title)
            Text("Built with SwiftUI")
                .foregroundStyle(.secondary)
        }}
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }}
}}

#Preview {{
    ContentView()
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "swiftui_build".into(),
                label: "Build SwiftUI package".into(),
                description: "Verify the package compiles (swift build; requires the Xcode toolchain — SwiftUI targets macOS)".into(),
                command: "swift".into(),
                args: vec!["build".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(600),
                condition: None,
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
        ]
    }
}
