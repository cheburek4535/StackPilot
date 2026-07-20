<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentProject, listDirectory, readFile, writeFile, openInVSCode } from "$lib/modules/workspace/api";
  import type { ProjectContext, FileEntry, FileContent } from "$lib/modules/workspace/types";
  import CodeEditor from "$lib/components/CodeEditor.svelte";

  let project = $state<ProjectContext | null>(null);
  let loading = $state(true);

  let currentDir = $state<string | null>(null);
  let entries = $state<FileEntry[]>([]);
  let dirLoading = $state(false);
  let dirError = $state("");

  let selectedFile = $state<string | null>(null);
  let fileContent = $state<FileContent | null>(null);
  let fileLoading = $state(false);
  let fileError = $state("");

  let editedContent = $state<string>("");
  let saving = $state(false);
  let saveMsg = $state("");

  let expandedDirs = $state<Set<string>>(new Set());

  onMount(async () => {
    project = await getCurrentProject();
    if (project?.project_path) {
      await navigateToDir(project.project_path);
    }
    loading = false;
  });

  async function navigateToDir(path: string) {
    currentDir = path;
    dirLoading = true;
    dirError = "";
    entries = [];
    try {
      entries = await listDirectory(path);
    } catch (e) {
      dirError = `Failed to list directory: ${e}`;
    }
    dirLoading = false;
  }

  function toggleDir(path: string) {
    if (expandedDirs.has(path)) {
      expandedDirs.delete(path);
      expandedDirs = new Set(expandedDirs);
    } else {
      expandedDirs.add(path);
      expandedDirs = new Set(expandedDirs);
    }
  }

  async function openFile(path: string) {
    selectedFile = path;
    fileLoading = true;
    fileError = "";
    fileContent = null;
    try {
      fileContent = await readFile(path);
      editedContent = fileContent.content;
    } catch (e) {
      fileError = `Failed to read file: ${e}`;
    }
    fileLoading = false;
  }

  async function saveFile() {
    if (!selectedFile) return;
    saving = true;
    saveMsg = "";
    try {
      await writeFile(selectedFile, editedContent);
      saveMsg = "Saved";
      fileContent = { content: editedContent, language: fileContent?.language ?? "plaintext" };
      setTimeout(() => saveMsg = "", 2000);
    } catch (e) {
      saveMsg = `Error: ${e}`;
    }
    saving = false;
  }

  function handleEditorChange(val: string) {
    editedContent = val;
    if (saveMsg === "Saved") saveMsg = "Unsaved changes";
  }

  function getIcon(entry: FileEntry): string {
    if (entry.is_dir) return expandedDirs.has(entry.path) ? "📂" : "📁";
    const ext = entry.name.split(".").pop()?.toLowerCase();
    if (["js", "jsx", "ts", "tsx"].includes(ext ?? "")) return "🟨";
    if (["py"].includes(ext ?? "")) return "🐍";
    if (["rs"].includes(ext ?? "")) return "🦀";
    if (["json"].includes(ext ?? "")) return "📋";
    if (["html", "htm"].includes(ext ?? "")) return "🌐";
    if (["css", "scss", "less"].includes(ext ?? "")) return "🎨";
    if (["md"].includes(ext ?? "")) return "📝";
    if (["toml", "yaml", "yml", "ini", "cfg"].includes(ext ?? "")) return "⚙️";
    return "📄";
  }

  let pathBreadcrumbs = $derived.by(() => {
    if (!currentDir) return [];
    const parts = currentDir.replace(/\\/g, "/").split("/");
    const crumbs: { label: string; path: string }[] = [];
    let acc = "";
    for (const p of parts) {
      if (!p) continue;
      acc += (acc ? "/" : "") + p;
      crumbs.push({ label: p, path: acc });
    }
    return crumbs;
  });
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs">Logs</a>
      <a href="/workspace/problems">Problems</a>
      <a href="/workspace/info">Info</a>
      <a href="/workspace/files" class="active">Files</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <p class="ws-empty">No project open. <a href="/profiles">Open a profile</a> to browse files.</p>
    {:else if !project.project_path}
      <p class="ws-empty">This project has no path set. Configure a project path in the profile.</p>
    {:else}
      <div class="header-row">
        <div>
          <h1>File Explorer</h1>
          <p class="subtitle">{project.project_path}</p>
        </div>
      </div>

      <div class="explorer-layout">
        <div class="file-tree">
          <div class="tree-header">Files</div>
          {#if dirLoading}
            <p class="tree-empty">Loading...</p>
          {:else if dirError}
            <p class="tree-error">{dirError}</p>
          {:else if !currentDir}
            <p class="tree-empty">No directory selected</p>
          {:else}
            <div class="breadcrumbs">
              {#each pathBreadcrumbs as crumb, i}
                {#if i > 0}<span class="bc-sep">/</span>{/if}
                <button class="bc-link" onclick={() => navigateToDir(crumb.path)}>{crumb.label}</button>
              {/each}
            </div>
            <div class="entries">
              {#each entries.filter((e) => e.is_dir).sort((a, b) => a.name.localeCompare(b.name)) as entry}
                <button class="entry-row" onclick={() => navigateToDir(entry.path)}>
                  <span class="entry-icon">{getIcon(entry)}</span>
                  <span class="entry-name">{entry.name}/</span>
                </button>
              {/each}
              {#each entries.filter((e) => !e.is_dir).sort((a, b) => a.name.localeCompare(b.name)) as entry}
                <button
                  class="entry-row"
                  class:selected={selectedFile === entry.path}
                  onclick={() => openFile(entry.path)}
                >
                  <span class="entry-icon">{getIcon(entry)}</span>
                  <span class="entry-name">{entry.name}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="editor-panel">
          {#if !selectedFile}
            <div class="editor-placeholder">
              <p>Select a file to view and edit</p>
            </div>
          {:else if fileLoading}
            <div class="editor-placeholder">
              <p>Loading file...</p>
            </div>
          {:else if fileError}
            <div class="editor-placeholder error">
              <p>{fileError}</p>
            </div>
          {:else if fileContent}
            <div class="editor-toolbar">
              <span class="editor-filename">{selectedFile.split(/[\\/]/).pop()}</span>
              <div class="editor-actions">
                {#if saveMsg}
                  <span class="save-msg">{saveMsg}</span>
                {/if}
                <button class="toolbar-btn" onclick={saveFile} disabled={saving}>
                  {saving ? "Saving..." : "💾 Save"}
                </button>
                <button class="toolbar-btn" onclick={() => openInVSCode(selectedFile!)}>
                  Open in VSCode
                </button>
              </div>
            </div>
            <div class="editor-wrapper">
              <CodeEditor
                value={editedContent}
                language={fileContent.language}
                onchange={handleEditorChange}
                onsave={saveFile}
              />
            </div>
          {/if}
        </div>
      </div>
    {/if}
  </main>
</div>

<style>
  .workspace-layout { display: flex; min-height: calc(100vh - 49px); }
  .ws-sidebar {
    width: 200px; flex-shrink: 0; background: #fff;
    border-right: 1px solid #e0e0e0; padding: 1.25rem 0;
  }
  .ws-brand { font-weight: 700; font-size: 0.9rem; padding: 0 1.25rem 0.75rem; color: #222; border-bottom: 1px solid #eee; margin-bottom: 0.5rem; }
  .ws-nav { display: flex; flex-direction: column; gap: 0.15rem; }
  .ws-nav a { display: block; padding: 0.4rem 1.25rem; text-decoration: none; color: #555; font-size: 0.85rem; border-left: 3px solid transparent; transition: all 0.1s; }
  .ws-nav a:hover { background: #f5f5f5; color: #222; }
  .ws-nav a.active { background: #e8eaf6; color: #283593; border-left-color: #283593; font-weight: 600; }

  .ws-content { flex: 1; padding: 2rem; display: flex; flex-direction: column; }
  .ws-empty { color: #999; font-style: italic; font-size: 0.85rem; padding: 2rem; text-align: center; background: #fafafa; border-radius: 8px; border: 1px dashed #ddd; }
  .ws-empty a { color: #396cd8; }

  .header-row { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 1rem; }
  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.82rem; margin: 0.15rem 0 0; font-family: monospace; }

  .explorer-layout { display: flex; gap: 1rem; flex: 1; min-height: 0; }

  .file-tree {
    width: 260px; flex-shrink: 0; display: flex; flex-direction: column;
    background: #fff; border: 1px solid #e0e0e0; border-radius: 8px;
    overflow: hidden;
  }
  .tree-header {
    padding: 0.5rem 0.75rem; font-size: 0.78rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.04em; color: #888; background: #fafafa; border-bottom: 1px solid #eee;
  }
  .tree-empty, .tree-error { padding: 1rem; font-size: 0.82rem; color: #888; text-align: center; }
  .tree-error { color: #c62828; }

  .breadcrumbs {
    padding: 0.35rem 0.6rem; font-size: 0.76rem; background: #fafafa;
    border-bottom: 1px solid #eee; white-space: nowrap; overflow-x: auto;
    display: flex; align-items: center; gap: 0.1rem;
  }
  .bc-sep { color: #bbb; }
  .bc-link { background: none; border: none; color: #396cd8; cursor: pointer; font-size: 0.76rem; padding: 0.05rem 0.15rem; white-space: nowrap; }
  .bc-link:hover { text-decoration: underline; }

  .entries { flex: 1; overflow-y: auto; padding: 0.25rem 0; }
  .entry-row {
    display: flex; align-items: center; gap: 0.35rem; width: 100%;
    padding: 0.3rem 0.6rem; border: none; background: none;
    cursor: pointer; font-size: 0.82rem; text-align: left; color: #333;
    transition: background 0.1s;
  }
  .entry-row:hover { background: #f5f5f5; }
  .entry-row.selected { background: #e8eaf6; color: #283593; font-weight: 500; }
  .entry-icon { font-size: 0.9rem; flex-shrink: 0; }
  .entry-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .editor-panel {
    flex: 1; display: flex; flex-direction: column; min-width: 0;
  }
  .editor-placeholder {
    display: flex; align-items: center; justify-content: center;
    height: 100%; color: #999; font-style: italic; font-size: 0.9rem;
    background: #fafafa; border: 1px dashed #ddd; border-radius: 8px;
  }
  .editor-placeholder.error { color: #c62828; }

  .editor-toolbar {
    display: flex; align-items: center; gap: 0.5rem;
    padding: 0.4rem 0.75rem; background: #fafafa;
    border: 1px solid #e0e0e0; border-bottom: none; border-radius: 8px 8px 0 0;
  }
  .editor-filename { font-weight: 600; font-size: 0.85rem; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .editor-actions { display: flex; align-items: center; gap: 0.3rem; flex-shrink: 0; }
  .save-msg { font-size: 0.75rem; color: #2e7d32; }
  .toolbar-btn {
    padding: 0.25rem 0.6rem; border: 1px solid #ccc; border-radius: 5px;
    background: #fff; color: #444; font-size: 0.78rem; cursor: pointer; transition: all 0.15s;
  }
  .toolbar-btn:hover:not(:disabled) { background: #f0f0f0; }
  .toolbar-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .editor-wrapper {
    flex: 1; min-height: 0; overflow: hidden;
    border: 1px solid #e0e0e0; border-top: none; border-radius: 0 0 8px 8px;
  }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }
    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }

    .subtitle { color: #aaa; }
    .file-tree { background: #0f0f0f98; border-color: #444; }
    .tree-header { background: #1e1e1e; color: #aaa; border-bottom-color: #333; }
    .breadcrumbs { background: #1e1e1e; border-bottom-color: #333; }
    .bc-link { color: #5b8def; }
    .entry-row { color: #ccc; }
    .entry-row:hover { background: #333; }
    .entry-row.selected { background: #1a237e; color: #c5cae9; }

    .editor-placeholder { background: #2a2a2a; border-color: #444; color: #888; }
    .editor-toolbar { background: #1e1e1e; border-color: #444; }
    .editor-filename { color: #eee; }
    .toolbar-btn { background: #2a2a2a; color: #ccc; border-color: #555; }
    .toolbar-btn:hover:not(:disabled) { background: #333; }
    .save-msg { color: #81c784; }
    .editor-wrapper { border-color: #444; }
  }
</style>
