<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Кольцо оценки окружения: тон честно отражает качество данных.
  // Нет данных / частичный скан — нейтральный/янтарный, никогда «зелёный успех».
  export type ScoreTone = "lime" | "amber" | "red" | "neutral";

  let {
    score = null,
    size = 96,
    label,
    tone = "neutral",
    partial = false,
  }: {
    score?: number | null;
    size?: number;
    label?: string;
    tone?: ScoreTone;
    /** Данные неполные (частичный скан) — штриховая окружность. */
    partial?: boolean;
  } = $props();

  const stroke = 8;
  const radius = $derived((size - stroke) / 2);
  const circumference = $derived(2 * Math.PI * radius);
  const dash = $derived(
    score == null ? "0" : `${(Math.max(0, Math.min(100, score)) / 100) * circumference} ${circumference}`,
  );

  const toneColor = $derived(
    tone === "lime"
      ? "var(--sp-lime)"
      : tone === "amber"
        ? "var(--sp-amber)"
        : tone === "red"
          ? "var(--sp-red)"
          : "var(--sp-text-3)",
  );
</script>

<div
  class="score-ring"
  style="--ring-size: {size}px; --ring-color: {toneColor};"
  role="img"
  aria-label={label ?? (score == null ? (i18n.t("tc.score.unavailable") as TranslationKey) : (i18n.t("tc.score.label", { score }) as TranslationKey))}
>
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke="var(--sp-bg-3)"
      stroke-width={stroke}
    />
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke="var(--ring-color)"
      stroke-width={stroke}
      stroke-linecap="round"
      stroke-dasharray={dash}
      transform="rotate(-90 {size / 2} {size / 2})"
      class="ring-fill"
      class:partial
    />
  </svg>
  <div class="ring-center">
    <span class="ring-value">{score == null ? "—" : score}</span>
    <span class="ring-unit">{i18n.t("tc.score.of_100") as TranslationKey}</span>
  </div>
</div>

<style>
  .score-ring {
    position: relative;
    width: var(--ring-size);
    height: var(--ring-size);
    flex: 0 0 auto;
  }

  .ring-fill {
    transition: stroke-dasharray 0.5s ease;
    filter: drop-shadow(0 0 6px color-mix(in srgb, var(--ring-color) 45%, transparent));
  }

  .ring-fill.partial {
    opacity: 0.75;
  }

  .ring-center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0;
  }

  .ring-value {
    font-size: calc(var(--ring-size) * 0.26);
    font-weight: var(--sp-fw-bold);
    line-height: 1.1;
    color: var(--sp-text-1);
    font-variant-numeric: tabular-nums;
  }

  .ring-unit {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }
</style>
