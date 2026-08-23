const fs = require('fs');
let file = 'src/lib/modules/toolchain/components/ToolCard.svelte';
let content = fs.readFileSync(file, 'utf8');

content = content.replace(
    'onrecheck: (toolId: string) => void;',
    'onrecheck: (toolId: string) => void;\n    onuninstall?: (toolId: string) => void;'
);

let uninstallBtn = `
      {#if onuninstall && tool.state.kind !== "missing" && tool.state.kind !== "unsupported_platform"}
        <IconButton icon="trash" title="Удалить" variant="ghost" onclick={() => onuninstall?.(tool.tool_id)} />
      {/if}
`;

content = content.replace(
    '      </Button>\n    {/if}',
    '      </Button>\n    {/if}' + uninstallBtn
);

fs.writeFileSync(file, content);
