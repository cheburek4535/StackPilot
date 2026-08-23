const fs = require('fs');
let file = 'src-tauri/src/lib.rs';
let content = fs.readFileSync(file, 'utf8');

if (!content.includes('tcx_uninstall_tool')) {
    content = content.replace(
        'modules::toolchain::commands::tcx_run_health_checks,',
        'modules::toolchain::commands::tcx_run_health_checks,\n            modules::toolchain::commands::tcx_uninstall_tool,'
    );
    fs.writeFileSync(file, content);
}
