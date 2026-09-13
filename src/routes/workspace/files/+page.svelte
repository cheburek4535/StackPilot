<script lang="ts">
  import { onDestroy } from "svelte";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import type { IconName } from "$lib/components/ui/icons";
  import { workspaceContext } from "$lib/modules/workspace/context";
  import {
    listDirectory,
    readFile,
    writeFile,
    openInVSCode,
  } from "$lib/modules/workspace/api";
  import type { FileEntry, FileContent } from "$lib/modules/workspace/types";
  import { formatFileSize } from "$lib/modules/workspace/status";
  import {
    toBreadcrumbs,
    parentPath,
  } from "$lib/modules/workspace/paths";
  import CodeEditor from "$lib/components/CodeEditor.svelte";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import { markHelpDid, HELP, HINT_WORKSPACE_FILES } from "$lib/core/help";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";

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
  let saveMsgTimer: ReturnType<typeof setTimeout> | null = null;

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  const projectPath = $derived(project?.project_path ?? null);

  const breadcrumbs = $derived(
    currentDir ? toBreadcrumbs(currentDir) : [],
  );
  const parent = $derived(currentDir ? parentPath(currentDir) : null);

  // Auto-navigate to the project root only when the project itself changes,
  // never when the user browses deeper into the tree.
  let lastProjectPath = $state<string | null>(null);

  $effect(() => {
    if (projectPath && lastProjectPath !== projectPath) {
      lastProjectPath = projectPath;
      navigateToDir(projectPath);
    } else if (!projectPath) {
      lastProjectPath = null;
      currentDir = null;
      entries = [];
      selectedFile = null;
      fileContent = null;
    }
  });

  onDestroy(() => {
    if (saveMsgTimer) {
      clearTimeout(saveMsgTimer);
      saveMsgTimer = null;
    }
  });

  async function navigateToDir(path: string) {
    currentDir = path;
    dirLoading = true;
    dirError = "";
    entries = [];
    selectedFile = null;
    fileContent = null;
    editedContent = "";
    try {
      entries = await listDirectory(path);
    } catch (e) {
      dirError = `${i18n.t("ws.load_failed")}: ${String(e)}`;
    }
    dirLoading = false;
  }

  async function openFile(path: string) {
    markHelpDid(HELP.workspaceFileOpened);
    selectedFile = path;
    fileLoading = true;
    fileError = "";
    fileContent = null;
    try {
      fileContent = await readFile(path);
      editedContent = fileContent.content;
    } catch (e) {
      fileError = `${i18n.t("ws.load_failed")}: ${String(e)}`;
    }
    fileLoading = false;
  }

  async function saveFile() {
    if (!selectedFile) return;
    saving = true;
    try {
      await writeFile(selectedFile, editedContent);
      fileContent = {
        content: editedContent,
        language: fileContent?.language ?? "plaintext",
      };
      flashSaveMsg(i18n.t("ws.saved"));
    } catch (e) {
      flashSaveMsg(i18n.t("ws.save_error", { err: String(e) }));
    }
    saving = false;
  }

  function flashSaveMsg(msg: string) {
    saveMsg = msg;
    if (saveMsgTimer) clearTimeout(saveMsgTimer);
    saveMsgTimer = setTimeout(() => {
      saveMsg = "";
    }, 2500);
  }

  function handleEditorChange(val: string) {
    editedContent = val;
    if (saveMsg === i18n.t("ws.saved")) saveMsg = i18n.t("ws.unsaved");
  }

  async function openSelectedInVSCode(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess(i18n.t("ws.toast_vscode"), i18n.t("ws.toast_file_vscode"));
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_vscode"),
        i18n.t("ws.toast_failed", { err: String(e) }),
      );
    }
  }

  function baseName(path: string): string {
    const parts = path.split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function fileIcon(entry: FileEntry): IconName {
    return entry.is_dir ? "folder" : "file";
  }

  function fileTone(entry: FileEntry): string {
    if (entry.is_dir) return "sp-file-tone-folder";
    const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
    if (["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"].includes(ext)) {
      return "sp-file-tone-yellow";
    }
    if (["py", "pyw"].includes(ext)) return "sp-file-tone-blue";
    if (["rs"].includes(ext)) return "sp-file-tone-orange";
    if (["json", "yaml", "yml", "toml", "ini", "cfg"].includes(ext)) return "sp-file-tone-cyan";
    if (["html", "htm", "xml", "svg"].includes(ext)) return "sp-file-tone-violet";
    if (["css", "scss", "less"].includes(ext)) return "sp-file-tone-blue";
    if (["md", "markdown", "txt"].includes(ext)) return "sp-file-tone-lime";
    if (["sh", "bash", "zsh", "ps1"].includes(ext)) return "sp-file-tone-green";
    return "sp-file-tone-neutral";
  }

  const sortedDirs = $derived(
    entries.filter((e) => e.is_dir).sort((a, b) => a.name.localeCompare(b.name)),
  );
  const sortedFiles = $derived(
    entries.filter((e) => !e.is_dir).sort((a, b) => a.name.localeCompare(b.name)),
  );
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("ws.files_title") as TranslationKey}
    description={i18n.t("ws.files_desc") as TranslationKey}
    icon="folder"
  />

  <HelpHint
    id={HINT_WORKSPACE_FILES.id}
    resolvedBy={HINT_WORKSPACE_FILES.resolvedBy}
    icon="folder"
    title={i18n.t("help.ws_files.title") as TranslationKey}
    text={i18n.t("help.ws_files.body") as TranslationKey}
  />

  {#if wsLoading}
    <LoadingState label={i18n.t("ws.loading_workspace") as TranslationKey} />
  {:else if wsError}
    <ErrorState title={i18n.t("ws.load_failed") as TranslationKey} message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title={i18n.t("ws.no_project_open") as TranslationKey}
      description={i18n.t("ws.no_project_open") as TranslationKey}
    />
  {:else if !projectPath}
    <EmptyState
      icon="external"
      title={i18n.t("ws.no_path") as TranslationKey}
      description={i18n.t("ws.no_path_desc") as TranslationKey}
    />
  {:else}
    <div class="sp-files-layout">
      <Card padding="none" variant="elevated" class="sp-tree-card">
        <div class="sp-tree-head">
          <div class="sp-breadcrumbs">
            {#each breadcrumbs as crumb, i}
              {#if i > 0}
                <span class="sp-bc-sep">/</span>
              {/if}
              <button
                class="sp-bc-link"
                onclick={() => navigateToDir(crumb.path)}
                title={crumb.path}
              >
                {crumb.label}
              </button>
            {/each}
          </div>
          <div class="sp-tree-actions">
            {#if parent}
              <Button
                size="sm"
                variant="ghost"
                icon="chevronLeft"
                label={i18n.t("ws.up_one_level") as TranslationKey}
                onclick={() => navigateToDir(parent!)}
              />
            {/if}
          </div>
        </div>

        {#if dirLoading}
          <div class="sp-tree-state">
            <LoadingState size="sm" label={i18n.t("ws.loading_workspace") as TranslationKey} />
          </div>
        {:else if dirError}
          <div class="sp-tree-state sp-tree-error">{dirError}</div>
        {:else}
          <div class="sp-tree-body">
            <div class="sp-tree-section">
              {#each sortedDirs as entry (entry.path)}
                <button
                  class="sp-entry"
                  onclick={() => navigateToDir(entry.path)}
                  title={entry.path}
                >
                  <span class="sp-entry-icon {fileTone(entry)}" aria-hidden="true">
                    <Icon name={fileIcon(entry)} size={14} />
                  </span>
                  <span class="sp-entry-name">{entry.name}/</span>
                </button>
              {/each}
            </div>
            <div class="sp-tree-section">
              {#each sortedFiles as entry (entry.path)}
                <button
                  class="sp-entry"
                  class:sp-entry-active={selectedFile === entry.path}
                  onclick={() => openFile(entry.path)}
                  title={entry.path}
                >
                  <span class="sp-entry-icon {fileTone(entry)}" aria-hidden="true">
                    <Icon name={fileIcon(entry)} size={14} />
                  </span>
                  <span class="sp-entry-name">{entry.name}</span>
                  <span class="sp-entry-size">{formatFileSize(entry.size)}</span>
                </button>
              {/each}
            </div>
            {#if entries.length === 0}
              <div class="sp-tree-state">{i18n.t("ws.empty_dir") as TranslationKey}</div>
            {/if}
          </div>
        {/if}
      </Card>

      <Card padding="none" variant="elevated" class="sp-editor-card">
        {#if !selectedFile}
          <div class="sp-editor-placeholder">
            <span class="sp-editor-placeholder-icon" aria-hidden="true">
              <Icon name="file" size={22} />
            </span>
            <p>{i18n.t("ws.select_file") as TranslationKey}</p>
          </div>
        {:else if fileLoading}
          <div class="sp-editor-placeholder">
            <LoadingState size="sm" label={i18n.t("ws.loading_workspace") as TranslationKey} />
          </div>
        {:else if fileError}
          <div class="sp-editor-placeholder sp-editor-placeholder-err">
            <p>{fileError}</p>
          </div>
        {:else if fileContent}
          <div class="sp-editor-toolbar">
            <div class="sp-editor-file">
              <Icon name="file" size={14} />
              <span class="sp-editor-filename">{baseName(selectedFile)}</span>
              <span class="sp-editor-path">{selectedFile}</span>
            </div>
            <div class="sp-editor-actions">
              {#if saveMsg}
                <span class="sp-save-msg" class:sp-save-msg-err={saveMsg.startsWith("Error")}>
                  {saveMsg}
                </span>
              {/if}
              <Button
                size="sm"
                variant="secondary"
                icon="external"
                onclick={() => openSelectedInVSCode(selectedFile!)}
              >
                {i18n.t("ws.open_vscode") as TranslationKey}
              </Button>
              <Button
                size="sm"
                variant="primary"
                icon="check"
                loading={saving}
                onclick={saveFile}
              >
                {i18n.t("ws.save") as TranslationKey}
              </Button>
            </div>
          </div>
          <div class="sp-editor-wrapper">
            <CodeEditor
              value={editedContent}
              language={fileContent.language}
              onchange={handleEditorChange}
              onsave={saveFile}
            />
          </div>
        {/if}
      </Card>
    </div>
  {/if}
</PageContainer>

<style>
  .sp-files-layout {
    display: grid;
    grid-template-columns: minmax(16rem, 24rem) 1fr;
    gap: var(--sp-4);
    align-items: start;
  }

  /* ---- tree ---- */

  .sp-tree-card {
    display: flex;
    flex-direction: column;
    max-height: 70vh;
  }

  .sp-tree-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-breadcrumbs {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    min-width: 0;
    overflow-x: auto;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    white-space: nowrap;
  }

  .sp-bc-sep {
    color: var(--sp-text-3);
  }

  .sp-bc-link {
    background: none;
    border: none;
    padding: 0.05rem 0.15rem;
    color: var(--sp-blue);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }

  .sp-bc-link:hover {
    text-decoration: underline;
  }

  .sp-tree-actions {
    flex-shrink: 0;
  }

  .sp-tree-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-tree-section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-entry {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: var(--sp-1) var(--sp-2);
    border: none;
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
    text-align: left;
    cursor: pointer;
    transition: background-color 0.12s ease;
  }

  .sp-entry:hover {
    background: var(--sp-bg-2);
  }

  .sp-entry-active {
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
  }

  .sp-entry-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .sp-file-tone-folder {
    color: var(--sp-violet);
  }

  .sp-file-tone-yellow {
    color: var(--sp-amber);
  }

  .sp-file-tone-blue {
    color: var(--sp-blue);
  }

  .sp-file-tone-orange {
    color: var(--sp-accent);
  }

  .sp-file-tone-cyan {
    color: var(--sp-cyan);
  }

  .sp-file-tone-violet {
    color: var(--sp-violet);
  }

  .sp-file-tone-lime {
    color: var(--sp-lime);
  }

  .sp-file-tone-green {
    color: var(--sp-success);
  }

  .sp-file-tone-neutral {
    color: var(--sp-text-3);
  }

  .sp-entry-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-entry-size {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    flex-shrink: 0;
  }

  .sp-tree-state {
    padding: var(--sp-8) var(--sp-4);
    text-align: center;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .sp-tree-error {
    color: var(--sp-danger);
    font-style: normal;
    word-break: break-all;
  }

  /* ---- editor ---- */

  .sp-editor-card {
    display: flex;
    flex-direction: column;
    min-width: 0;
    max-height: 70vh;
  }

  .sp-editor-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-editor-file {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    color: var(--sp-text-2);
  }

  .sp-editor-filename {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    white-space: nowrap;
  }

  .sp-editor-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-editor-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-shrink: 0;
  }

  .sp-save-msg {
    font-size: var(--sp-fs-xs);
    color: var(--sp-success);
  }

  .sp-save-msg-err {
    color: var(--sp-danger);
  }

  .sp-editor-wrapper {
    flex: 1 1 auto;
    min-height: 0;
    padding: var(--sp-3);
  }

  .sp-editor-wrapper :global(.cm-container) {
    height: 100%;
    min-height: 22rem;
    border-color: var(--sp-border);
  }

  .sp-editor-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: var(--sp-10) var(--sp-4);
    text-align: center;
    color: var(--sp-text-3);
    font-style: italic;
    font-size: var(--sp-fs-sm);
  }

  .sp-editor-placeholder p {
    margin: 0;
  }

  .sp-editor-placeholder-err {
    color: var(--sp-danger);
    font-style: normal;
    word-break: break-all;
  }

  .sp-editor-placeholder-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3rem;
    height: 3rem;
    border-radius: var(--sp-radius-full);
    color: var(--sp-text-3);
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
  }

  @media (max-width: 900px) {
    .sp-files-layout {
      grid-template-columns: 1fr;
    }
  }
</style>
