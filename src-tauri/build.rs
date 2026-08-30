fn main() {
    // Test binaries on Windows link the full Win32 stack, including
    // `TaskDialogIndirect` from comctl32. Without an application manifest
    // the loader binds the legacy comctl32 v5.82 from WinSxS, which does
    // not export v6 functions — every `cargo test` binary then dies at
    // load time with STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139).
    //
    // The shipped application is unaffected: tauri-build embeds the same
    // common-controls v6 manifest into the app binary via tauri-winres.
    // Here we mirror that mechanism (rc.exe -> .res -> linker input) for
    // TEST targets only, so the app build stays exactly as before.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        if let Some(rc) = find_rc() {
            let out_dir =
                std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
            let manifest_path = out_dir.join("stackpilot-common-controls.manifest");
            std::fs::write(&manifest_path, include_str!("app.manifest"))
                .expect("failed to write common-controls manifest");

            let rc_path = out_dir.join("stackpilot-test-manifest.rc");
            let res_path = out_dir.join("stackpilot-test-manifest.res");
            // rc.exe treats `\` as an escape character in string literals —
            // forward slashes are accepted and keep the path intact.
            // Resource type must be the numeric 24 (RT_MANIFEST): the bare
            // identifier `RT_MANIFEST` would be recorded as a *named*
            // string type, which the loader ignores.
            let manifest_display = manifest_path.display().to_string().replace('\\', "/");
            std::fs::write(&rc_path, format!("1 24 \"{}\"\n", manifest_display))
                .expect("failed to write resource script");

            let status = std::process::Command::new(&rc)
                .arg("/fo")
                .arg(&res_path)
                .arg(&rc_path)
                .status()
                .expect("failed to run rc.exe");
            assert!(
                status.success(),
                "rc.exe failed to compile the test manifest resource"
            );

            println!("cargo:rustc-link-arg={}", res_path.display());
        } else {
            println!(
                "cargo:warning=rc.exe not found in Windows Kits; \
                 test binaries may fail to load on Windows"
            );
        }
    }

    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Locate the Windows SDK resource compiler (rc.exe). Mirrors the search
/// used by tauri-winres: the newest `10.0.*` Windows Kits bin folder.
fn find_rc() -> Option<std::path::PathBuf> {
    let kits_root = std::env::var("ProgramFiles(x86)")
        .or_else(|_| std::env::var("ProgramFiles"))
        .ok()
        .map(|pf| {
            std::path::PathBuf::from(pf)
                .join("Windows Kits")
                .join("10")
                .join("bin")
        });

    let mut best: Option<(u64, std::path::PathBuf)> = None;
    if let Some(root) = kits_root {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(version) = name.strip_prefix("10.0.") else {
                    continue;
                };
                // Version directories look like "10.0.22621.0" — the SDK
                // build number is the leading numeric component.
                let Some(build) = version.split('.').next() else {
                    continue;
                };
                let Ok(build) = build.parse::<u64>() else {
                    continue;
                };
                let candidate = entry.path().join("x64").join("rc.exe");
                if candidate.is_file() && best.as_ref().map(|(v, _)| build > *v).unwrap_or(true) {
                    best = Some((build, candidate));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}
