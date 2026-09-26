use crate::modules::project_creator::engine::{crc32, zig_package_name};
use crate::modules::project_creator::engine::providers::{cmd, write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct ZapProvider;

impl RecipeProvider for ZapProvider {
    fn id(&self) -> &'static str {
        "zap"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_cli = context.frameworks.iter().any(|f| f == "zig-cli");
        let main_zig = if has_cli {
            format!(
                r#"const std = @import("std");
const zap = @import("zap");
const cli = @import("cli.zig");

fn on_request(r: zap.Request) void {{
    r.sendBody("Hello from {{s}}!", .{{"{}"}}) catch {{}};
}}

pub fn main() !void {{
    var args = std.process.args();
    _ = args.next();
    if (args.next()) |arg| {{
        if (std.mem.eq(u8, arg, "cli")) return cli.run();
    }}
    var listener = zap.HttpListener.init(.{{
        .on_request = on_request,
        .port = 3000,
    }});
    try listener.listen();
    std.debug.print("Listening on http://localhost:3000\n", .{{}});
    zap.start(.{{ .threads = 1, .workers = 1 }});
}}
"#,
                project_name
            )
        } else {
            format!(
                r#"const std = @import("std");
const zap = @import("zap");

fn on_request(r: zap.Request) void {{
    r.sendBody("Hello from {{s}}!", .{{"{}"}}) catch {{}};
}}

pub fn main() !void {{
    var listener = zap.HttpListener.init(.{{
        .on_request = on_request,
        .port = 3000,
    }});
    try listener.listen();
    std.debug.print("Listening on http://localhost:3000\n", .{{}});
    zap.start(.{{ .threads = 1, .workers = 1 }});
}}
"#,
                project_name
            )
        };

        let pkg_name = zig_package_name(project_name);
        let fingerprint = (u64::from(crc32(pkg_name.as_bytes())) << 32) | 0xCAFE_BABE;

        vec![
            write_file(
                "zap_zon",
                "Create build.zig.zon",
                "build.zig.zon",
                &format!(
                    r#".{{
    .name = .{pkg_name},
    .version = "0.1.0",
    .minimum_zig_version = "0.14.0",
    .paths = .{{""}},
    .fingerprint = 0x{fingerprint:016x},
    .dependencies = .{{}},
}}
"#
                ),
            ),
            write_file(
                "zap_build",
                "Create build.zig",
                "build.zig",
                &format!(
                    r#"const std = @import("std");

pub fn build(b: *std.Build) !void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const zap = b.dependency("zap", .{{
        .target = target,
        .optimize = optimize,
    }});

    const exe = b.addExecutable(.{{
        .name = "{}",
        .root_module = b.createModule(.{{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{{
                .{{ .name = "zap", .module = zap.module("zap") }},
            }},
        }}),
    }});

    b.installArtifact(exe);
}}
"#,
                    project_name
                ),
            ),
            cmd(
                "zap_fetch",
                "Add Zap dependency",
                "Fetch zap and save to build.zig.zon",
                "zig",
                vec![
                    "fetch",
                    "--save",
                    "https://github.com/zigzap/zap/archive/refs/tags/v0.10.1.tar.gz",
                ],
                project_path,
            ),
            write_file(
                "zap_main",
                "Create Zap server entry",
                "src/main.zig",
                &main_zig,
            ),
        ]
    }
}
