const fs = require('fs');
let file = 'src/lib/modules/toolchain/components/ManageEverythingMode.svelte';
let content = fs.readFileSync(file, 'utf8');

content = content.replace(
    'onrecheck={handleRecheck}',
    'onrecheck={handleRecheck}\n              onuninstall={(id) => toolchain.uninstallTool(id)}'
);

fs.writeFileSync(file, content);
