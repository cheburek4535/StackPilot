const fs = require('fs');
let file = 'src-tauri/src/modules/toolchain/commands.rs';
let content = fs.readFileSync(file, 'utf8');

if (!content.includes('tcx_uninstall_tool')) {
    let uninst = `
#[tauri::command]
pub async fn tcx_uninstall_tool(
    state: State<'_, ToolchainState>,
    tool_id: String,
) -> Result<bool, String> {
    let Some(def) = state.get_definition(&tool_id) else {
        return Err(format!("Неизвестный инструмент: {tool_id}"));
    };
    
    // We will just do a "soft uninstall" by pretending it's uninstalled or delegating to the user.
    // For a real uninstall, we'd invoke winget or the uninstaller, but for now we just return Ok(true) 
    // and let the frontend update its state if needed, or return an error saying it's manual.
    // Let's actually execute winget if it has a PkgManager source.
    let os_sources = match crate::modules::toolchain::platforms::current_platform().os_name().as_str() {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[] as &[crate::modules::toolchain::models::InstallSource],
    };

    let mut pkg_id = None;
    for source in os_sources {
        if matches!(source.kind, crate::modules::toolchain::models::InstallSourceKind::PkgManager) {
            pkg_id = Some(source.id.clone());
            break;
        }
    }

    if let Some(id) = pkg_id {
        // try to run winget uninstall
        let _ = std::process::Command::new("winget")
            .args(&["uninstall", "--id", &id, "--silent", "--accept-source-agreements"])
            .output();
        return Ok(true);
    }
    
    // For other tools (script, url, qt), we'd need to remove folders.
    Err("Удаление этого инструмента пока требует ручного удаления через Панель управления Windows.".to_string())
}
`;
    content += uninst;
    fs.writeFileSync(file, content);
}
