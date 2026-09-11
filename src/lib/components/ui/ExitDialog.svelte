<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { ExitAskPayload, ExitAskProcess, ExitProcessKind } from "$lib/core/exit";
  import { resolveExitRequest, cancelExitRequest } from "$lib/core/exit";

  let {
    payload,
    onclose,
  }: {
    payload: ExitAskPayload;
    onclose: () => void;
  } = $props();

  /** Какая кнопка сейчас выполняется (блокирует повторные клики). */
  let acting = $state<null | "terminate" | "leave" | "cancel">(null);

  const KIND_INFO: Record<ExitProcessKind, { icon: string; labelKey: string }> = {
    docker: { icon: "🐳", labelKey: "exit.kind.docker" },
    node: { icon: "🟩", labelKey: "exit.kind.node" },
    python: { icon: "🐍", labelKey: "exit.kind.python" },
    jvm: { icon: "☕", labelKey: "exit.kind.jvm" },
    compiled: { icon: "🔩", labelKey: "exit.kind.compiled" },
    shell: { icon: "🖥️", labelKey: "exit.kind.shell" },
    other: { icon: "⚙️", labelKey: "exit.kind.other" },
  };

  const KIND_ORDER: ExitProcessKind[] = [
    "docker",
    "node",
    "python",
    "jvm",
    "compiled",
    "shell",
    "other",
  ];

  /** Процессы, сгруппированные по «понятной» группе (head-процессы). */
  let groups = $derived.by<{ kind: ExitProcessKind; processes: ExitAskProcess[] }[]>(() => {
    const byKind = new Map<ExitProcessKind, ExitAskProcess[]>();
    for (const p of payload.processes) {
      const kind = KIND_INFO[p.kind] ? p.kind : "other";
      const list = byKind.get(kind) ?? [];
      list.push(p);
      byKind.set(kind, list);
    }
    return KIND_ORDER.filter((k) => byKind.has(k)).map((k) => ({
      kind: k,
      processes: byKind.get(k) ?? [],
    }));
  });

  const totalCount = $derived(payload.processes.length);

  async function act(action: "terminate" | "leave" | "cancel") {
    if (acting) return;
    acting = action;
    try {
      if (action === "cancel") {
        await cancelExitRequest();
        onclose();
      } else {
        await resolveExitRequest(action === "terminate");
      }
    } catch {
      acting = null;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") void act("cancel");
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="exit-overlay">
  <div class="exit-dialog" role="dialog" aria-modal="true" aria-labelledby="exit-title">
    <div class="exit-head">
      <span class="exit-icon" aria-hidden="true">🛑</span>
      <div class="exit-head-text">
        <h2 class="exit-title" id="exit-title">{i18n.t("exit.title") as string}</h2>
        <p class="exit-desc">{i18n.t("exit.description", { n: totalCount }) as string}</p>
      </div>
    </div>

    <div class="exit-groups">
      {#each groups as group}
        <div class="exit-group">
          <p class="exit-group-label">
            <span class="exit-group-icon" aria-hidden="true">{KIND_INFO[group.kind].icon}</span>
            {i18n.t(KIND_INFO[group.kind].labelKey) as string}
            <span class="exit-group-count">{group.processes.length}</span>
          </p>
          <ul class="exit-procs">
            {#each group.processes as proc}
              <li class="exit-proc">
                <span class="exit-proc-title" title={proc.command}>{proc.title}</span>
                {#if proc.command && proc.command !== proc.title}
                  <code class="exit-proc-cmd" title={proc.command}>{proc.command}</code>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>

    <p class="exit-note">{i18n.t("exit.keep_note") as string}</p>

    <div class="exit-actions">
      <button
        class="exit-btn exit-btn-danger"
        disabled={acting !== null}
        onclick={() => act("terminate")}
      >
        {acting === "terminate" ? (i18n.t("exit.terminating") as string) : (i18n.t("exit.terminate") as string)}
      </button>
      <button
        class="exit-btn exit-btn-ghost"
        disabled={acting !== null}
        onclick={() => act("leave")}
      >
        {i18n.t("exit.leave") as string}
      </button>
      <button
        class="exit-btn exit-btn-plain"
        disabled={acting !== null}
        onclick={() => act("cancel")}
      >
        {i18n.t("exit.cancel") as string}
      </button>
    </div>
  </div>
</div>

<style>
  .exit-overlay {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(2, 3, 6, 0.72);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
  }
  .exit-dialog {
    width: min(560px, calc(100vw - 3rem));
    max-height: calc(100vh - 4rem);
    display: flex;
    flex-direction: column;
    background: var(--sp-glass-strong);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-xl);
    padding: 1.4rem;
    box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-3);
  }
  .exit-head { display: flex; align-items: flex-start; gap: 0.75rem; }
  .exit-icon { font-size: 1.5rem; line-height: 1.2; }
  .exit-title { margin: 0; font-size: 1.15rem; font-weight: 700; color: var(--sp-text-1); }
  .exit-desc { margin: 0.25rem 0 0; font-size: 0.85rem; color: var(--sp-text-2); }
  .exit-groups {
    margin: 1rem 0 0.5rem;
    padding-right: 0.25rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }
  .exit-group { display: flex; flex-direction: column; gap: 0.3rem; }
  .exit-group-label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0;
    font-size: 0.75rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sp-text-2);
  }
  .exit-group-count {
    min-width: 1.25rem;
    padding: 0.05rem 0.35rem;
    border-radius: 999px;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    font-size: 0.68rem;
    text-align: center;
    color: var(--sp-text-3);
  }
  .exit-procs {
    list-style: none;
    margin: 0;
    padding: 0 0 0 1.35rem;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .exit-proc { display: flex; flex-direction: column; min-width: 0; }
  .exit-proc-title {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .exit-proc-cmd {
    font-family: var(--sp-font-mono);
    font-size: 0.72rem;
    color: var(--sp-text-3);
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: 4px;
    padding: 0.1rem 0.4rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: fit-content;
    max-width: 100%;
  }
  .exit-note { margin: 0.25rem 0 0; font-size: 0.75rem; color: var(--sp-text-3); font-style: italic; }
  .exit-actions {
    display: flex;
    gap: 0.6rem;
    margin-top: 1.1rem;
    flex-wrap: wrap;
  }
  .exit-btn {
    flex: 1 1 auto;
    min-width: 9rem;
    padding: 0.6rem 1rem;
    border-radius: var(--sp-radius-lg);
    border: 1px solid var(--sp-border-strong);
    font-family: var(--sp-font-sans);
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s, box-shadow 0.15s;
  }
  .exit-btn:disabled { opacity: 0.6; cursor: default; }
  .exit-btn-danger {
    background: var(--sp-danger-soft);
    color: var(--sp-danger);
    border-color: var(--sp-danger);
  }
  .exit-btn-danger:hover:not(:disabled) { background: var(--sp-danger); color: #fff; }
  .exit-btn-ghost { background: var(--sp-accent-soft); color: var(--sp-text-1); }
  .exit-btn-ghost:hover:not(:disabled) { border-color: var(--sp-accent-strong); color: #fff; }
  .exit-btn-plain { background: var(--sp-bg-1); color: var(--sp-text-2); }
  .exit-btn-plain:hover:not(:disabled) { color: var(--sp-text-1); }
</style>