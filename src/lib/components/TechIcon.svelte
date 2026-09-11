<script lang="ts">
  // Универсальная иконка технологии/инструмента.
  //
  // Берёт имя файла иконки из wizard_tree (`icon`), рендерит <img> из
  // /images/ с фиксированным размером (object-contain). Если `icon`
  // пустой или картинка не загрузилась (onerror) — показывает
  // дефолтную SVG-шестерёнку.
  let {
    icon = null,
    alt = "",
    size = "sm",
    class: extraClass = "",
  }: {
    icon?: string | null;
    alt?: string;
    size?: "xs" | "sm" | "md" | "lg" | "xl";
    class?: string;
  } = $props();

  let failed = $state(false);

  const src = $derived(icon ? `/images/${icon}` : "");

  // Сброс ошибки при смене иконки (переиспользование компонента в #each)
  $effect(() => {
    if (src) failed = false;
  });
</script>

{#if icon && !failed}
  <img
    src={src}
    alt={alt}
    class="tech-icon tech-icon-{size} {extraClass}"
    draggable="false"
    onerror={() => (failed = true)}
  />
{:else}
  <span class="tech-icon tech-icon-fallback tech-icon-{size} {extraClass}" aria-hidden={alt === ""}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="12" cy="12" r="3"></circle>
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
    </svg>
  </span>
{/if}

<style>
  .tech-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    object-fit: contain;
    flex: 0 0 auto;
    vertical-align: middle;
  }

  .tech-icon-xs { width: 16px; height: 16px; }
  .tech-icon-sm { width: 20px; height: 20px; }
  .tech-icon-md { width: 26px; height: 26px; }
  .tech-icon-lg { width: 40px; height: 40px; }
  .tech-icon-xl { width: 56px; height: 56px; }

  .tech-icon-fallback {
    color: var(--tech-icon-fallback, var(--sp-text-3));
    opacity: 0.75;
  }
  .tech-icon-fallback svg {
    width: 100%;
    height: 100%;
  }
</style>