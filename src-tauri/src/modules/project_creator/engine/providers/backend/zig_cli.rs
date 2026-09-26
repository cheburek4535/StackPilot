use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct ZigCliProvider;

impl RecipeProvider for ZigCliProvider {
    fn id(&self) -> &'static str {
        "zig-cli"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        if context.frameworks.iter().any(|f| f == "zap") {
            vec![write_file(
                "zig_cli_module",
                "Create Zig CLI module",
                "src/cli.zig",
                &format!(
                    r#"const std = @import("std");

pub fn run() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}} CLI!\n", .{{"{}"}});
}}
"#,
                    project_name
                ),
            )]
        } else {
            vec![write_file(
                "zig_main",
                "Create Zig CLI entry",
                "src/main.zig",
                &format!(
                    r#"const std = @import("std");

pub fn main() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}}!\n", .{{"{}"}});
}}
"#,
                    project_name
                ),
            )]
        }
    }
}
