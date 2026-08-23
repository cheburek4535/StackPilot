const fs = require('fs');
let file = 'src/lib/modules/toolchain/components/ToolDetailDrawer.svelte';
let content = fs.readFileSync(file, 'utf8');

let uninstallBtn = `
          {#if tool.state.kind !== "missing" && tool.state.kind !== "unsupported_platform"}
            <Button variant="ghost" onclick={() => toolchain.uninstallTool(tool!.tool_id)}>
              Удалить
            </Button>
          {/if}
`;

content = content.replace(
    '        </div>\n      </header>',
    uninstallBtn + '        </div>\n      </header>'
);

fs.writeFileSync(file, content);
