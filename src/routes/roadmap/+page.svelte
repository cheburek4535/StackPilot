<script lang="ts">
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  type RoadmapItem = {
    id: string;
    status: "done" | "planned" | "in_progress";
    icon: string;
  };

  const roadmapItems: RoadmapItem[] = [
    { id: "ai_assistant", status: "in_progress", icon: "bot" },
    { id: "env_isolation", status: "planned", icon: "shield" },
    { id: "tech_expansion", status: "in_progress", icon: "layers" },
    { id: "cli_service", status: "planned", icon: "terminal" },
    { id: "plugin_system", status: "planned", icon: "store" },
    { id: "bug_fixes", status: "in_progress", icon: "refresh" },
  ];

  const statusBadge: Record<string, { tone: "lime" | "cyan" | "amber"; label: string }> = {
    done: { tone: "lime", label: "roadmap.status.done" },
    planned: { tone: "amber", label: "roadmap.status.planned" },
    in_progress: { tone: "cyan", label: "roadmap.status.in_progress" },
  };

  const techList = [
    "C", "Ruby", "Angular", "Vite", "Rails", "Astro", "Hono",
    "Actix Web", "Echo", "Blazor", "Remix", "AdonisJS", "Rails",
    "Angular CLI", "GCC (MinGW-w64)"
  ];

  const techListTranslated = $derived([
    ...techList,
    i18n.t("roadmap.other"),
    i18n.t("roadmap.your_suggestions")
  ]);

  type FeatureItem = {
    icon: string;
    titleKey: string;
    descKey: string;
  };

  const features: FeatureItem[] = [
    { icon: "💾", titleKey: "roadmap.features.local", descKey: "roadmap.features.local_desc" },
    { icon: "🌍", titleKey: "roadmap.features.open_source", descKey: "roadmap.features.open_source_desc" },
    { icon: "🎁", titleKey: "roadmap.features.free", descKey: "roadmap.features.free_desc" },
    { icon: "🖥️", titleKey: "roadmap.features.cross_platform", descKey: "roadmap.features.cross_platform_desc" },
    { icon: "🔧", titleKey: "roadmap.features.tools", descKey: "roadmap.features.tools_desc" },
    { icon: "⚡", titleKey: "roadmap.features.scaffold", descKey: "roadmap.features.scaffold_desc" },
    { icon: "🚀", titleKey: "roadmap.features.profiles", descKey: "roadmap.features.profiles_desc" },
    { icon: "📊", titleKey: "roadmap.features.analyze", descKey: "roadmap.features.analyze_desc" },
  ];

  type ChangelogEntry = {
    version: string;
    date: string;
    changesKey: string;
  };

  const changelog: ChangelogEntry[] = [
    {
      version: "1.2.1",
      date: "2026-08-28",
      changesKey: "roadmap.changelog.v050"
    },
  ];
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("roadmap.title") as TranslationKey}
    description={i18n.t("roadmap.description") as TranslationKey}
    icon="map"
  />

  <div class="sp-roadmap">
    <Card
      title={i18n.t("roadmap.section.plans") as TranslationKey}
      description={i18n.t("roadmap.section.plans_desc") as TranslationKey}
    >
      <div class="sp-roadmap-list">
        {#each roadmapItems as item}
          {@const badge = statusBadge[item.status]}
          <div class="sp-roadmap-item">
            <div class="sp-roadmap-icon">
              {#if item.icon === "bot"}
                <span class="sp-icon">🤖</span>
              {:else if item.icon === "shield"}
                <span class="sp-icon">🛡️</span>
              {:else if item.icon === "layers"}
                <span class="sp-icon">📦</span>
              {:else if item.icon === "terminal"}
                <span class="sp-icon">💻</span>
              {:else if item.icon === "store"}
                <span class="sp-icon">🧩</span>
              {:else if item.icon === "refresh"}
                <span class="sp-icon">🔧</span>
              {/if}
            </div>
            <div class="sp-roadmap-content">
              <h4 class="sp-roadmap-name">
                {i18n.t(`roadmap.item.${item.id}.title`) as TranslationKey}
              </h4>
              <p class="sp-roadmap-desc">
                {i18n.t(`roadmap.item.${item.id}.desc`) as TranslationKey}
              </p>
              {#if item.id === "tech_expansion"}
                <div class="sp-tech-list">
                  {#each techListTranslated as tech}
                    <Badge tone="blue">{tech}</Badge>
                  {/each}
                </div>
              {/if}
            </div>
            <Badge tone={badge.tone}>
              {i18n.t(badge.label) as TranslationKey}
            </Badge>
          </div>
        {/each}
      </div>
    </Card>

    <Card
      title={i18n.t("roadmap.section.about") as TranslationKey}
      description={i18n.t("roadmap.section.about_desc") as TranslationKey}
    >
      <div class="sp-about-content">
        <p class="sp-about-text">
          {i18n.t("roadmap.about.text1") as TranslationKey}
        </p>
        <p class="sp-about-text">
          {i18n.t("roadmap.about.text2") as TranslationKey}
        </p>

        <div class="sp-support-section">
          <h4 class="sp-support-title">
            {i18n.t("roadmap.support.title") as TranslationKey}
          </h4>
          <p class="sp-support-desc">
            {i18n.t("roadmap.support.desc") as TranslationKey}
          </p>
          <a
            href="https://boosty.to/niperc/purchase/4073941?ssource=DIRECT&share=subscription_link"
            target="_blank"
            rel="noopener noreferrer"
            class="sp-boosty-link"
          >
            {i18n.t("roadmap.support.boosty") as TranslationKey}
          </a>
        </div>

        <div class="sp-community-section">
          <h4 class="sp-community-title">
            {i18n.t("roadmap.community.title") as TranslationKey}
          </h4>
          <p class="sp-community-desc">
            {i18n.t("roadmap.community.desc") as TranslationKey}
          </p>
          <a href="mailto:niperc77@gmail.com" class="sp-email-link">
            niperc77@gmail.com
          </a>
        </div>
      </div>
    </Card>

    <Card
      title={i18n.t("roadmap.features.title") as TranslationKey}
      description={i18n.t("roadmap.features.desc") as TranslationKey}
    >
      <div class="sp-features-grid">
        {#each features as feature}
          <div class="sp-feature-item">
            <span class="sp-feature-icon">{feature.icon}</span>
            <div class="sp-feature-content">
              <h4 class="sp-feature-title">
                {i18n.t(feature.titleKey) as TranslationKey}
              </h4>
              <p class="sp-feature-desc">
                {i18n.t(feature.descKey) as TranslationKey}
              </p>
            </div>
          </div>
        {/each}
      </div>
    </Card>

    <Card
      title={i18n.t("roadmap.changelog.title") as TranslationKey}
      description={i18n.t("roadmap.changelog.desc") as TranslationKey}
    >
      <div class="sp-changelog-list">
        {#each changelog as entry}
          <div class="sp-changelog-item">
            <div class="sp-changelog-header">
              <Badge tone="violet">{i18n.t("roadmap.changelog.version") as TranslationKey} {entry.version}</Badge>
              <span class="sp-changelog-date">{entry.date}</span>
            </div>
            <p class="sp-changelog-changes">
              {i18n.t(entry.changesKey) as TranslationKey}
            </p>
          </div>
        {/each}
      </div>
    </Card>
  </div>
</PageContainer>

<style>
  .sp-roadmap {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .sp-roadmap-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .sp-roadmap-item {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    transition: border-color 0.15s ease;
  }

  .sp-roadmap-item:hover {
    border-color: var(--sp-accent-border);
  }

  .sp-roadmap-icon {
    flex-shrink: 0;
    width: 2rem;
    height: 2rem;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-md);
  }

  .sp-icon {
    font-size: 1.25rem;
  }

  .sp-roadmap-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-roadmap-name {
    margin: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-roadmap-desc {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: 1.5;
  }

  .sp-tech-list {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
    margin-top: var(--sp-1);
  }

  .sp-about-content {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .sp-about-text {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: 1.6;
  }

  .sp-support-section,
  .sp-community-section {
    padding: var(--sp-4);
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-md);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-support-title,
  .sp-community-title {
    margin: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-support-desc,
  .sp-community-desc {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: 1.5;
  }

  .sp-boosty-link,
  .sp-email-link {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-accent);
    color: white;
    border-radius: var(--sp-radius-md);
    text-decoration: none;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    transition: opacity 0.15s ease;
    width: fit-content;
  }

  .sp-boosty-link:hover,
  .sp-email-link:hover {
    opacity: 0.9;
    text-decoration: none;
    color: white;
  }

  .sp-features-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: var(--sp-3);
  }

  .sp-feature-item {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-feature-icon {
    font-size: 1.5rem;
    flex-shrink: 0;
  }

  .sp-feature-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-feature-title {
    margin: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-feature-desc {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: 1.5;
  }

  .sp-changelog-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .sp-changelog-item {
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-changelog-header {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-changelog-date {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-family: var(--sp-font-mono);
  }

  .sp-changelog-changes {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    line-height: 1.5;
  }
</style>