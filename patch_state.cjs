const fs = require('fs');
let file = 'src/lib/modules/toolchain/state.svelte.ts';
let content = fs.readFileSync(file, 'utf8');

if (!content.includes('async uninstallTool')) {
    let uninst = `
  async uninstallTool(toolId: string): Promise<boolean> {
    if (confirm(\`Вы действительно хотите удалить \${toolId}?\n\nВнимание: автоматическое удаление может оставить следы в системе. Для полного удаления используйте системные средства (Установка и удаление программ).\`)) {
      try {
        await window.__TAURI__.invoke("tcx_uninstall_tool", { toolId });
        await this.runHealthChecks([toolId]);
        return true;
      } catch (e: any) {
        notifyError("Удаление не удалось", e.toString());
        return false;
      }
    }
    return false;
  }
`;
    content = content.replace('  // ---- UI-состояние', uninst + '\n  // ---- UI-состояние');
    fs.writeFileSync(file, content);
}
