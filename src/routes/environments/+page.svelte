<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import {
    listEnvironments,
    getOrCreateDefaultEnvironment,
    saveEnvironment,
    deleteEnvironment,
    bindProjectToEnvironment,
    unbindProjectFromEnvironment,
    openEnvironmentTerminal,
    configureVsCodeEnvironment,
    exportStandaloneEnvironment,
    calculateDiskUsage,
    cleanupSandbox,
  } from "$lib/modules/project_environment/api";
  import type {
    EnvironmentBinding,
    IsolationMode,
    EnvironmentDiskUsage,
  } from "$lib/modules/project_environment/types";
  import { openProject } from "$lib/core/integration";
  import { selectFolder } from "$lib/modules/project_creator/api";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let environments = $state<EnvironmentBinding[]>([]);
  let loading = $state(true);
  let searchQuery = $state("");

  // Modal State for Create / Edit
  let showModal = $state(false);
  let editingId = $state<string | null>(null);
  let modalName = $state("");
  let modalDescription = $state("");
  let modalIsolationMode = $state<IsolationMode>("isolated");
  let modalEnvVars = $state<{ key: string; value: string }[]>([]);
  let saving = $state(false);

  // Filtered environments
  let filteredEnvironments = $derived(
    environments.filter((env) => {
      const q = searchQuery.toLowerCase().trim();
      if (!q) return true;
      const matchName = (env.name || env.binding_id).toLowerCase().includes(q);
      const matchDesc = (env.description || "").toLowerCase().includes(q);
      const matchTools = Object.keys(env.tool_overrides || {}).some((t) => t.toLowerCase().includes(q));
      const matchProjects = (env.bound_projects || []).some((p) => p.toLowerCase().includes(q));
      return matchName || matchDesc || matchTools || matchProjects;
    })
  );

  let totalIsolated = $derived(environments.filter((e) => e.isolation_mode === "isolated").length);
  let totalProjects = $derived(
    environments.reduce((sum, e) => sum + (e.bound_projects ? e.bound_projects.length : 0), 0)
  );

  let diskUsages = $state<Record<string, EnvironmentDiskUsage>>({});

  let totalDiskUsageBytes = $derived(
    Object.values(diskUsages).reduce((sum, u) => sum + (u?.size_bytes || 0), 0)
  );
  let totalDiskUsageDisplay = $derived(
    totalDiskUsageBytes >= 1024 * 1024 * 1024
      ? `${(totalDiskUsageBytes / (1024 * 1024 * 1024)).toFixed(1)} GB`
      : totalDiskUsageBytes >= 1024 * 1024
      ? `${(totalDiskUsageBytes / (1024 * 1024)).toFixed(1)} MB`
      : totalDiskUsageBytes > 0
      ? `${(totalDiskUsageBytes / 1024).toFixed(1)} KB`
      : "0 B"
  );

  onMount(async () => {
    await loadData();
  });

  async function loadData() {
    loading = true;
    try {
      // Ensure default exists
      await getOrCreateDefaultEnvironment();
      environments = await listEnvironments();

      // Query storage for isolated environments
      for (const env of environments) {
        if (env.isolation_mode === "isolated") {
          calculateDiskUsage(env.binding_id)
            .then((usage) => {
              diskUsages[env.binding_id] = usage;
            })
            .catch(() => {});
        }
      }
    } catch (e) {
      notifyError("Ошибка загрузки сред", String(e));
    } finally {
      loading = false;
    }
  }

  async function handleCleanupSandbox(env: EnvironmentBinding) {
    const confirmed = confirm(
      `Очистить временный кэш песочницы для «${env.name || env.binding_id}»? Исполняемые файлы и шиммы будут пересозданы начисто.`
    );
    if (!confirmed) return;
    try {
      const usage = await cleanupSandbox(env.binding_id);
      diskUsages[env.binding_id] = usage;
      notifySuccess("Кэш очищен", `Размер песочницы теперь ${usage.size_display}`);
    } catch (e) {
      notifyError("Ошибка очистки", String(e));
    }
  }

  function openCreateModal() {
    editingId = null;
    modalName = "";
    modalDescription = "";
    modalIsolationMode = "isolated";
    modalEnvVars = [];
    showModal = true;
  }

  function openEditModal(env: EnvironmentBinding) {
    editingId = env.binding_id;
    modalName = env.name || "";
    modalDescription = env.description || "";
    modalIsolationMode = env.isolation_mode;
    modalEnvVars = Object.entries(env.env_vars || {}).map(([key, value]) => ({ key, value }));
    showModal = true;
  }

  function addEnvVarRow() {
    modalEnvVars = [...modalEnvVars, { key: "", value: "" }];
  }

  function removeEnvVarRow(index: number) {
    modalEnvVars = modalEnvVars.filter((_, i) => i !== index);
  }

  async function handleSave() {
    if (!modalName.trim()) {
      notifyError("Имя обязательно", "Укажите название окружения");
      return;
    }

    saving = true;
    try {
      let bindingToSave: EnvironmentBinding;
      if (editingId) {
        const existing = environments.find((e) => e.binding_id === editingId);
        if (!existing) throw new Error("Окружение не найдено");
        bindingToSave = { ...existing };
      } else {
        const now = new Date().toISOString();
        bindingToSave = {
          schema_version: 1,
          binding_id: `env_${Date.now().toString(16)}`,
          name: modalName.trim(),
          isolation_mode: modalIsolationMode,
          is_default: false,
          description: modalDescription.trim() || null,
          icon: "box",
          color: "orange",
          project_path: null,
          bound_projects: [],
          env_dir: null,
          tool_overrides: {},
          managed_path_entries: [],
          env_vars: {},
          env_vars_remove: [],
          preferred_ide: null,
          preferred_ide_args: null,
          created_at: now,
          updated_at: now,
        };
      }

      bindingToSave.name = modalName.trim();
      bindingToSave.description = modalDescription.trim() || null;
      bindingToSave.isolation_mode = modalIsolationMode;

      const envMap: Record<string, string> = {};
      for (const row of modalEnvVars) {
        if (row.key.trim()) {
          envMap[row.key.trim()] = row.value;
        }
      }
      bindingToSave.env_vars = envMap;

      await saveEnvironment(bindingToSave);
      notifySuccess("Сохранено", `Окружение «${bindingToSave.name}» сохранено`);
      showModal = false;
      await loadData();
    } catch (e) {
      notifyError("Ошибка сохранения", String(e));
    } finally {
      saving = false;
    }
  }

  async function handleDelete(env: EnvironmentBinding) {
    if (env.is_default) {
      notifyError("Защита", "Нельзя удалить системное окружение по умолчанию");
      return;
    }
    const confirmed = confirm(
      i18n.t("environments.delete_confirm" as TranslationKey) || "Удалить это окружение?"
    );
    if (!confirmed) return;

    try {
      await deleteEnvironment(env.binding_id);
      notifySuccess("Удалено", `Окружение «${env.name || env.binding_id}» удалено`);
      await loadData();
    } catch (e) {
      notifyError("Ошибка удаления", String(e));
    }
  }

  async function handleOpenTerminal(env: EnvironmentBinding) {
    try {
      await openEnvironmentTerminal(env.binding_id);
      notifySuccess("Терминал", `Запущен терминал в окружении «${env.name || env.binding_id}»`);
    } catch (e) {
      notifyError("Ошибка терминала", String(e));
    }
  }

  async function handleBindProject(env: EnvironmentBinding) {
    try {
      const folder = await selectFolder();
      if (!folder) return;
      await bindProjectToEnvironment(env.binding_id, folder);
      notifySuccess("Проект привязан", `Проект ${folder} привязан к среде «${env.name}»`);
      await loadData();
    } catch (e) {
      notifyError("Ошибка привязки", String(e));
    }
  }

  async function handleUnbindProject(env: EnvironmentBinding, projectPath: string) {
    try {
      await unbindProjectFromEnvironment(env.binding_id, projectPath);
      notifySuccess("Отвязано", "Проект отвязан от окружения");
      await loadData();
    } catch (e) {
      notifyError("Ошибка отвязки", String(e));
    }
  }

  async function handleOpenInWorkspace(projectPath: string, env: EnvironmentBinding) {
    try {
      await openProject(projectPath);
      goto("/workspace");
    } catch (e) {
      notifyError("Ошибка открытия", String(e));
    }
  }

  async function handleConfigureVsCode(env: EnvironmentBinding, projectPath: string) {
    try {
      await configureVsCodeEnvironment(env.binding_id, projectPath);
      notifySuccess("VS Code", `Конфигурация .vscode/settings.json обновлена для «${env.name}»`);
    } catch (e) {
      notifyError("Ошибка VS Code", String(e));
    }
  }

  async function handleExportStandalone(env: EnvironmentBinding, projectPath?: string) {
    try {
      let target = projectPath;
      if (!target) {
        target = env.project_path || (env.bound_projects && env.bound_projects[0]);
      }
      if (!target) {
        target = (await selectFolder()) || undefined;
      }
      if (!target) return;

      const res = await exportStandaloneEnvironment(env.binding_id, target);
      notifySuccess(
        "Standalone скрипты экспортированы",
        `Созданы ${res.created_files.join(", ")} в директории: ${res.target_dir}`
      );
    } catch (e) {
      notifyError("Ошибка экспорта", String(e));
    }
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("environments.title" as TranslationKey) || "Окружения разработки"}
    description={i18n.t("environments.subtitle" as TranslationKey) || "Изолированные среды под проекты: независимые версии рантаймов, нулевое засорение системы и быстрый старт"}
    icon="package"
  >
    {#snippet actions()}
      <Button
        variant="primary"
        icon="sparkles"
        onclick={openCreateModal}
      >
        {i18n.t("environments.create_btn" as TranslationKey) || "Создать окружение"}
      </Button>
    {/snippet}
  </PageHeader>

  <!-- Statistics strip -->
  <div class="sp-env-metrics-grid">
    <Card padding="md" variant="elevated">
      <div class="metric-card">
        <div class="metric-icon metric-purple">
          <Icon name="layers" size={20} />
        </div>
        <div class="metric-content">
          <div class="metric-val">{environments.length}</div>
          <div class="metric-lbl">Всего сред</div>
        </div>
      </div>
    </Card>

    <Card padding="md" variant="elevated">
      <div class="metric-card">
        <div class="metric-icon metric-amber">
          <Icon name="package" size={20} />
        </div>
        <div class="metric-content">
          <div class="metric-val">{totalIsolated}</div>
          <div class="metric-lbl">Изолированных боксов</div>
        </div>
      </div>
    </Card>

    <Card padding="md" variant="elevated">
      <div class="metric-card">
        <div class="metric-icon metric-blue">
          <Icon name="folder" size={20} />
        </div>
        <div class="metric-content">
          <div class="metric-val">{totalProjects}</div>
          <div class="metric-lbl">Связанных проектов</div>
        </div>
      </div>
    </Card>

    <Card padding="md" variant="elevated">
      <div class="metric-card">
        <div class="metric-icon metric-cyan">
          <Icon name="database" size={20} />
        </div>
        <div class="metric-content">
          <div class="metric-val">{totalDiskUsageDisplay}</div>
          <div class="metric-lbl">Объём песочниц</div>
        </div>
      </div>
    </Card>
  </div>

  <!-- Search and filter bar -->
  <div class="sp-filter-bar">
    <div class="sp-search-wrapper">
      <Icon name="search" size={16} />
      <input
        type="text"
        placeholder="Поиск окружений по имени, инструментам или проектам…"
        bind:value={searchQuery}
        class="sp-search-input"
      />
      {#if searchQuery}
        <button class="sp-clear-btn" onclick={() => searchQuery = ""}>✕</button>
      {/if}
    </div>
  </div>

  {#if loading}
    <LoadingState label="Загрузка списка окружений…" />
  {:else if filteredEnvironments.length === 0}
    <EmptyState
      icon="package"
      title={i18n.t("environments.empty_title" as TranslationKey) || "Окружения не найдены"}
      description={searchQuery
        ? "По вашему запросу ничего не найдено"
        : (i18n.t("environments.empty_desc" as TranslationKey) || "Создайте новую изолированную среду для проекта.")}
    >
      {#snippet action()}
        <Button variant="secondary" icon="sparkles" onclick={openCreateModal}>
          Создать первую среду
        </Button>
      {/snippet}
    </EmptyState>
  {:else}
    <div class="sp-env-grid">
      {#each filteredEnvironments as env (env.binding_id)}
        <div class="sp-env-card" class:is-default={env.is_default}>
          <!-- Card Header -->
          <div class="sp-env-card-header">
            <div class="sp-env-title-block">
              <div class="sp-env-avatar" class:default-avatar={env.is_default}>
                <Icon name={env.is_default ? "globe" : "package"} size={20} />
              </div>
              <div>
                <div class="sp-env-name-row">
                  <h3 class="sp-env-name">{env.name || env.binding_id}</h3>
                  {#if env.is_default}
                    <Badge tone="cyan">По умолчанию (Система)</Badge>
                  {:else}
                    <Badge tone="amber">Изолированное</Badge>
                    {#if diskUsages[env.binding_id]}
                      <Badge tone="neutral">
                        <Icon name="database" size={12} class="mr-1" />
                        {diskUsages[env.binding_id].size_display}
                      </Badge>
                    {/if}
                  {/if}
                </div>
                {#if env.description}
                  <p class="sp-env-desc">{env.description}</p>
                {/if}
              </div>
            </div>

            <!-- Quick Card Actions -->
            <div class="sp-env-actions">
              <Button
                variant="ghost"
                size="sm"
                icon="terminal"
                onclick={() => handleOpenTerminal(env)}
                label="Открыть консоль с окружением этой среды"
              >
                Терминал
              </Button>
              <Button
                variant="ghost"
                size="sm"
                icon="download"
                onclick={() => handleExportStandalone(env)}
                label="Экспортировать автономные скрипты (activate.bat, activate.sh)"
              >
                Экспорт
              </Button>
              {#if env.isolation_mode === "isolated"}
                <Button
                  variant="ghost"
                  size="sm"
                  icon="trash"
                  onclick={() => handleCleanupSandbox(env)}
                  label="Очистить временный кэш песочницы"
                >
                  Очистить кэш
                </Button>
              {/if}
              <Button
                variant="ghost"
                size="sm"
                icon="settings"
                onclick={() => openEditModal(env)}
                label="Настроить окружение"
              >
                Настроить
              </Button>
              {#if !env.is_default}
                <button
                  type="button"
                  class="sp-icon-delete-btn"
                  onclick={() => handleDelete(env)}
                  title="Удалить окружение"
                >
                  <Icon name="trash" size={15} />
                </button>
              {/if}
            </div>
          </div>

          <!-- Tools Section -->
          <div class="sp-env-section">
            <div class="sp-section-title">
              <Icon name="wrench" size={13} />
              <span>Инструменты и рантаймы</span>
            </div>

            {#if Object.keys(env.tool_overrides || {}).length === 0}
              <div class="sp-env-tools-empty">
                {env.is_default
                  ? "Использует системные глобальные версии из PATH хоста"
                  : "Инструменты не переопределены (наследуются из хоста или песочницы)"}
              </div>
            {:else}
              <div class="sp-tools-tags">
                {#each Object.entries(env.tool_overrides) as [toolId, tool]}
                  <span class="sp-tool-tag">
                    <span class="tool-name">{toolId}</span>
                    {#if tool.version}
                      <span class="tool-ver">v{tool.version}</span>
                    {/if}
                  </span>
                {/each}
              </div>
            {/if}
          </div>

          <!-- Environment variables section -->
          {#if Object.keys(env.env_vars || {}).length > 0}
            <div class="sp-env-section">
              <div class="sp-section-title">
                <Icon name="layers" size={13} />
                <span>Переменные среды ({Object.keys(env.env_vars).length})</span>
              </div>
              <div class="sp-envvars-preview">
                {#each Object.entries(env.env_vars).slice(0, 3) as [k, v]}
                  <span class="sp-envvar-pill"><code>{k}</code> = {v}</span>
                {/each}
                {#if Object.keys(env.env_vars).length > 3}
                  <span class="sp-envvar-more">+{Object.keys(env.env_vars).length - 3} ещё</span>
                {/if}
              </div>
            </div>
          {/if}

          <!-- Bound Projects Section -->
          <div class="sp-env-section sp-env-projects-section">
            <div class="sp-section-header">
              <div class="sp-section-title">
                <Icon name="folder" size={13} />
                <span>Привязанные проекты ({(env.bound_projects || []).length})</span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                icon="plus"
                onclick={() => handleBindProject(env)}
              >
                Привязать папку
              </Button>
            </div>

            {#if !env.bound_projects || env.bound_projects.length === 0}
              <div class="sp-empty-projects">
                Нет привязанных проектов. Создайте проект через Wizard или привяжите существующую папку.
              </div>
            {:else}
              <div class="sp-projects-list">
                {#each env.bound_projects as projPath}
                  <div class="sp-project-item">
                    <div class="sp-project-path-wrap">
                      <Icon name="folder" size={14} />
                      <span class="sp-project-path" title={projPath}>{projPath}</span>
                    </div>
                    <div class="sp-project-actions">
                      <button
                        type="button"
                        class="sp-proj-btn"
                        onclick={() => handleOpenInWorkspace(projPath, env)}
                        title="Открыть в Workspace"
                      >
                        <Icon name="play" size={12} />
                        <span>Открыть</span>
                      </button>
                      <button
                        type="button"
                        class="sp-proj-btn"
                        onclick={() => handleConfigureVsCode(env, projPath)}
                        title="Настроить .vscode под эту среду"
                      >
                        <Icon name="terminal" size={12} />
                        <span>VS Code</span>
                      </button>
                      <button
                        type="button"
                        class="sp-proj-btn"
                        onclick={() => handleExportStandalone(env, projPath)}
                        title="Экспортировать автономные скрипты активации (activate.bat, activate.sh)"
                      >
                        <Icon name="download" size={12} />
                        <span>Экспорт</span>
                      </button>
                      <button
                        type="button"
                        class="sp-proj-remove-btn"
                        onclick={() => handleUnbindProject(env, projPath)}
                        title="Отвязать проект от этой среды"
                      >
                        ✕
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Create / Edit Modal -->
  <Modal
    open={showModal}
    onclose={() => showModal = false}
    title={editingId ? "Настройка окружения" : "Создание новой среды"}
    description="Изолированное окружение позволяет избежать конфликтов версий и обеспечивает чистый запуск проектов."
    size="lg"
  >
    <div class="sp-modal-form">
      <div class="sp-form-group">
        <label for="env-name" class="sp-form-label">Название окружения *</label>
        <input
          id="env-name"
          type="text"
          placeholder="Например: Modern Web (Node 22), Python Data Lab, Fintech API..."
          bind:value={modalName}
          class="sp-form-input"
        />
      </div>

      <div class="sp-form-group">
        <label for="env-desc" class="sp-form-label">Описание (необязательно)</label>
        <input
          id="env-desc"
          type="text"
          placeholder="Краткое описание назначения этой среды..."
          bind:value={modalDescription}
          class="sp-form-input"
        />
      </div>

      <!-- Mode Selector -->
      <div class="sp-form-group">
        <span class="sp-form-label">Тип изоляции</span>
        <div class="sp-isolation-options">
          <label class="sp-option-card" class:selected={modalIsolationMode === "isolated"}>
            <input
              type="radio"
              name="isolation"
              value="isolated"
              checked={modalIsolationMode === "isolated"}
              onchange={() => modalIsolationMode = "isolated"}
            />
            <div class="sp-option-text">
              <div class="sp-option-head">
                <span class="sp-option-title">Изолированный бокс (Рекомендуется)</span>
                <Badge tone="amber">Sandboxed</Badge>
              </div>
              <p class="sp-option-desc">
                Создаёт выделенную директорию среды со своими бинарниками, изолирует <code>npm -g</code> и <code>PYTHONNOUSERSITE</code>. Никаких конфликтов с системой!
              </p>
            </div>
          </label>

          <label class="sp-option-card" class:selected={modalIsolationMode === "global"}>
            <input
              type="radio"
              name="isolation"
              value="global"
              checked={modalIsolationMode === "global"}
              onchange={() => modalIsolationMode = "global"}
            />
            <div class="sp-option-text">
              <div class="sp-option-head">
                <span class="sp-option-title">Глобальное системное окружение</span>
                <Badge tone="neutral">Host</Badge>
              </div>
              <p class="sp-option-desc">
                Использует системные пути из PATH без изоляции директорий.
              </p>
            </div>
          </label>
        </div>
      </div>

      <!-- Extra Environment Variables -->
      <div class="sp-form-group">
        <div class="sp-envvars-header">
          <span class="sp-form-label">Пользовательские переменные окружения (.env)</span>
          <Button variant="ghost" size="sm" icon="plus" onclick={addEnvVarRow}>
            Добавить переменную
          </Button>
        </div>

        {#if modalEnvVars.length === 0}
          <p class="sp-muted-text">Переменные окружения не заданы. Можно добавить например <code>PORT=4000</code> или <code>DATABASE_URL</code>.</p>
        {:else}
          <div class="sp-envvars-table">
            {#each modalEnvVars as row, i}
              <div class="sp-envvar-row">
                <input
                  type="text"
                  placeholder="Имя (напр. PORT)"
                  bind:value={row.key}
                  class="sp-form-input env-key-input"
                />
                <span class="sp-eq">=</span>
                <input
                  type="text"
                  placeholder="Значение (напр. 3000)"
                  bind:value={row.value}
                  class="sp-form-input env-val-input"
                />
                <button
                  type="button"
                  class="sp-row-del-btn"
                  onclick={() => removeEnvVarRow(i)}
                  title="Удалить строку"
                >
                  ✕
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>

    {#snippet footer()}
      <Button variant="ghost" onclick={() => showModal = false}>
        Отмена
      </Button>
      <Button variant="primary" loading={saving} onclick={handleSave}>
        {editingId ? "Сохранить изменения" : "Создать среду"}
      </Button>
    {/snippet}
  </Modal>
</PageContainer>

<style>
  .sp-env-metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 16px;
    margin-bottom: 24px;
  }

  .metric-card {
    display: flex;
    align-items: center;
    gap: 14px;
  }

  .metric-icon {
    width: 44px;
    height: 44px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .metric-purple {
    background: rgba(168, 85, 247, 0.15);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.3);
  }

  .metric-amber {
    background: rgba(245, 158, 11, 0.15);
    color: #fbbf24;
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .metric-blue {
    background: rgba(59, 130, 246, 0.15);
    color: #60a5fa;
    border: 1px solid rgba(59, 130, 246, 0.3);
  }

  .metric-cyan {
    background: rgba(6, 182, 212, 0.15);
    color: #22d3ee;
    border: 1px solid rgba(6, 182, 212, 0.3);
  }

  .metric-val {
    font-size: 22px;
    font-weight: 700;
    color: var(--sp-text, #f1f5f9);
    line-height: 1.2;
  }

  .metric-lbl {
    font-size: 12px;
    color: var(--sp-text-muted, #94a3b8);
  }

  .sp-filter-bar {
    margin-bottom: 20px;
  }

  .sp-search-wrapper {
    display: flex;
    align-items: center;
    gap: 10px;
    background: var(--sp-card-bg, #1e222b);
    border: 1px solid var(--sp-border, #2d3343);
    border-radius: 8px;
    padding: 8px 14px;
    color: var(--sp-text-muted, #94a3b8);
    max-width: 480px;
  }

  .sp-search-input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--sp-text, #f1f5f9);
    font-size: 14px;
    width: 100%;
  }

  .sp-clear-btn {
    background: none;
    border: none;
    color: var(--sp-text-muted, #94a3b8);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 6px;
  }

  .sp-env-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(420px, 1fr));
    gap: 20px;
  }

  .sp-env-card {
    background: var(--sp-card-bg, #181b22);
    border: 1px solid var(--sp-border, #282e3e);
    border-radius: 12px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    transition: transform 0.15s ease, border-color 0.15s ease;
  }

  .sp-env-card:hover {
    border-color: rgba(245, 158, 11, 0.4);
    transform: translateY(-2px);
  }

  .sp-env-card.is-default {
    border-color: rgba(59, 130, 246, 0.35);
  }

  .sp-env-card-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }

  .sp-env-title-block {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }

  .sp-env-avatar {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .sp-env-avatar.default-avatar {
    background: rgba(59, 130, 246, 0.15);
    color: #3b82f6;
    border-color: rgba(59, 130, 246, 0.3);
  }

  .sp-env-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .sp-env-name {
    font-size: 16px;
    font-weight: 600;
    color: var(--sp-text, #f1f5f9);
    margin: 0;
  }

  .sp-env-desc {
    font-size: 13px;
    color: var(--sp-text-muted, #94a3b8);
    margin: 4px 0 0 0;
    line-height: 1.4;
  }

  .sp-env-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sp-icon-delete-btn {
    background: transparent;
    border: none;
    color: var(--sp-text-muted, #94a3b8);
    cursor: pointer;
    padding: 6px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.15s ease, background 0.15s ease;
  }

  .sp-icon-delete-btn:hover {
    color: #ef4444;
    background: rgba(239, 68, 68, 0.1);
  }

  .sp-env-section {
    background: rgba(0, 0, 0, 0.2);
    border-radius: 8px;
    padding: 12px;
    border: 1px solid rgba(255, 255, 255, 0.04);
  }

  .sp-section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }

  .sp-section-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--sp-text-muted, #94a3b8);
    margin-bottom: 6px;
  }

  .sp-env-tools-empty,
  .sp-empty-projects {
    font-size: 12px;
    color: var(--sp-text-muted, #64748b);
    font-style: italic;
  }

  .sp-tools-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .sp-tool-tag {
    background: var(--sp-card-bg, #1e222b);
    border: 1px solid var(--sp-border, #333b4f);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .tool-name {
    color: var(--sp-text, #f1f5f9);
    font-weight: 500;
  }

  .tool-ver {
    color: #f59e0b;
    font-family: monospace;
    font-size: 11px;
  }

  .sp-envvars-preview {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .sp-envvar-pill {
    font-size: 11px;
    font-family: monospace;
    background: rgba(255, 255, 255, 0.05);
    padding: 2px 6px;
    border-radius: 4px;
    color: #e2e8f0;
  }

  .sp-envvar-more {
    font-size: 11px;
    color: var(--sp-text-muted, #94a3b8);
  }

  .sp-projects-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sp-project-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--sp-card-bg, #1e222b);
    border: 1px solid var(--sp-border, #2d3343);
    border-radius: 6px;
    padding: 6px 10px;
  }

  .sp-project-path-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
  }

  .sp-project-path {
    font-size: 12px;
    color: var(--sp-text, #f1f5f9);
    text-overflow: ellipsis;
    white-space: nowrap;
    overflow: hidden;
    max-width: 200px;
  }

  .sp-project-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .sp-proj-btn {
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.08);
    color: var(--sp-text, #f1f5f9);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 11px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    transition: background 0.15s ease;
  }

  .sp-proj-btn:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .sp-proj-remove-btn {
    background: transparent;
    border: none;
    color: var(--sp-text-muted, #94a3b8);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
  }

  .sp-proj-remove-btn:hover {
    color: #ef4444;
  }

  /* Modal Form */
  .sp-modal-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .sp-form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sp-form-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--sp-text, #f1f5f9);
  }

  .sp-form-input {
    background: var(--sp-input-bg, #14171f);
    border: 1px solid var(--sp-border, #2d3343);
    border-radius: 8px;
    padding: 8px 12px;
    color: var(--sp-text, #f1f5f9);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s ease;
  }

  .sp-form-input:focus {
    border-color: #f59e0b;
  }

  .sp-isolation-options {
    display: grid;
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .sp-option-card {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    border: 1px solid var(--sp-border, #2d3343);
    border-radius: 8px;
    padding: 12px;
    background: var(--sp-card-bg, #181b22);
    cursor: pointer;
    transition: border-color 0.15s ease;
  }

  .sp-option-card.selected {
    border-color: #f59e0b;
    background: rgba(245, 158, 11, 0.05);
  }

  .sp-option-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .sp-option-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--sp-text, #f1f5f9);
  }

  .sp-option-desc {
    font-size: 12px;
    color: var(--sp-text-muted, #94a3b8);
    margin: 4px 0 0 0;
    line-height: 1.4;
  }

  .sp-envvars-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .sp-muted-text {
    font-size: 12px;
    color: var(--sp-text-muted, #64748b);
    margin: 0;
  }

  .sp-envvars-table {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .sp-envvar-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .env-key-input {
    flex: 1;
    font-family: monospace;
    font-size: 13px;
  }

  .env-val-input {
    flex: 2;
    font-family: monospace;
    font-size: 13px;
  }

  .sp-eq {
    color: var(--sp-text-muted, #94a3b8);
    font-weight: 700;
  }

  .sp-row-del-btn {
    background: transparent;
    border: none;
    color: var(--sp-text-muted, #94a3b8);
    cursor: pointer;
    font-size: 14px;
    padding: 4px 8px;
    border-radius: 4px;
  }

  .sp-row-del-btn:hover {
    color: #ef4444;
  }
</style>
