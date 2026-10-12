// ============================================================
// Toolchain — структурированные секции и популярность инструментов
// ============================================================
// Единая каноническая классификация инструментов по смысловым
// секциям с упорядочиванием по популярности (must-have сверху,
// тяжеловесные сервисы/брокеры вроде Kafka и Grafana — снизу).

import type { IconName } from "$lib/components/ui/icons";

export type ToolchainSectionId =
  | "must_have"
  | "languages"
  | "package_managers"
  | "frameworks"
  | "databases"
  | "tooling"
  | "infrastructure";

export interface ToolchainSectionMeta {
  id: ToolchainSectionId;
  titleKey: string;
  defaultTitle: string;
  descKey: string;
  defaultDesc: string;
  icon: IconName;
  order: number;
}

export const TOOLCHAIN_SECTIONS: ToolchainSectionMeta[] = [
  {
    id: "must_have",
    titleKey: "tc.section.must_have",
    defaultTitle: "Базовые инструменты (Must-Have)",
    descKey: "tc.section.must_have_desc",
    defaultDesc: "Системные менеджеры пакетов, контроль версий и ключевые утилиты",
    icon: "sparkles",
    order: 1,
  },
  {
    id: "languages",
    titleKey: "tc.section.languages",
    defaultTitle: "Языки и платформы",
    descKey: "tc.section.languages_desc",
    defaultDesc: "Языки программирования, компиляторы и среды выполнения",
    icon: "terminal",
    order: 2,
  },
  {
    id: "package_managers",
    titleKey: "tc.section.package_managers",
    defaultTitle: "Пакетные менеджеры и сборка",
    descKey: "tc.section.package_managers_desc",
    defaultDesc: "Менеджеры зависимостей, компиляторы и инструменты сборки",
    icon: "package",
    order: 3,
  },
  {
    id: "frameworks",
    titleKey: "tc.section.frameworks",
    defaultTitle: "Фреймворки и мобильная разработка",
    descKey: "tc.section.frameworks_desc",
    defaultDesc: "Инструменты для GUI и кроссплатформенных приложений",
    icon: "rocket",
    order: 4,
  },
  {
    id: "databases",
    titleKey: "tc.section.databases",
    defaultTitle: "Базы данных",
    descKey: "tc.section.databases_desc",
    defaultDesc: "Реляционные, документные и in-memory СУБД для локальной разработки",
    icon: "database",
    order: 5,
  },
  {
    id: "tooling",
    titleKey: "tc.section.tooling",
    defaultTitle: "Инструменты и облачные CLI",
    descKey: "tc.section.tooling_desc",
    defaultDesc: "Утилиты автоматизации, CLI и вспомогательный софт",
    icon: "wrench",
    order: 6,
  },
  {
    id: "infrastructure",
    titleKey: "tc.section.infrastructure",
    defaultTitle: "Службы и брокеры (обычно в Docker)",
    descKey: "tc.section.infrastructure_desc",
    defaultDesc: "Тяжеловесные инфраструктурные сервисы; для локальной разработки обычно запускаются в Docker",
    icon: "server",
    order: 7,
  },
];

/** Маппинг конкретных ID инструментов на секции и ранги популярности внутри секции (0 = самый популярный) */
const TOOL_CONFIG: Record<string, { section: ToolchainSectionId; rank: number }> = {
  // Must-have / базовые
  winget: { section: "must_have", rank: 1 },
  brew: { section: "must_have", rank: 2 },
  git: { section: "must_have", rank: 3 },
  docker: { section: "must_have", rank: 4 },
  vscode: { section: "must_have", rank: 5 },
  curl: { section: "must_have", rank: 6 },
  tar: { section: "must_have", rank: 7 },

  // Языки и платформы
  node: { section: "languages", rank: 1 },
  python: { section: "languages", rank: 2 },
  rust: { section: "languages", rank: 3 },
  go: { section: "languages", rank: 4 },
  dotnet: { section: "languages", rank: 5 },
  java: { section: "languages", rank: 6 },
  php: { section: "languages", rank: 7 },
  kotlin: { section: "languages", rank: 8 },
  dart: { section: "languages", rank: 9 },
  zig: { section: "languages", rank: 10 },
  swift: { section: "languages", rank: 11 },
  elixir: { section: "languages", rank: 12 },
  erlang: { section: "languages", rank: 13 },
  gleam: { section: "languages", rank: 14 },

  // Пакетные менеджеры и сборка
  npm: { section: "package_managers", rank: 1 },
  pip: { section: "package_managers", rank: 2 },
  cargo: { section: "package_managers", rank: 3 },
  rustc: { section: "package_managers", rank: 4 },
  composer: { section: "package_managers", rank: 5 },
  maven: { section: "package_managers", rank: 6 },
  gradle: { section: "package_managers", rank: 7 },
  cmake: { section: "package_managers", rank: 8 },
  "msvc-build-tools": { section: "package_managers", rank: 9 },
  xcodebuild: { section: "package_managers", rank: 10 },

  // Фреймворки и мобильная разработка
  flutter: { section: "frameworks", rank: 1 },
  android: { section: "frameworks", rank: 2 },
  "tauri-cli": { section: "frameworks", rank: 3 },
  qt: { section: "frameworks", rank: 4 },

  // Базы данных
  sqlite: { section: "databases", rank: 1 },
  postgresql: { section: "databases", rank: 2 },
  redis: { section: "databases", rank: 3 },
  mysql: { section: "databases", rank: 4 },
  mongodb: { section: "databases", rank: 5 },

  // Инструменты и CLI
  terraform: { section: "tooling", rank: 1 },
  firebase: { section: "tooling", rank: 2 },
  csharprepl: { section: "tooling", rank: 3 },

  // Службы и брокеры (Docker-ориентированные, в самом низу)
  kafka: { section: "infrastructure", rank: 1 },
  grafana: { section: "infrastructure", rank: 2 },
};

/**
 * Определяет секцию инструмента по его tool_id или категории.
 */
export function getToolSectionId(toolId: string, category?: string): ToolchainSectionId {
  const direct = TOOL_CONFIG[toolId];
  if (direct) return direct.section;

  // Резервный маппинг по сырой категории из каталога
  switch (category) {
    case "vcs":
    case "editor":
    case "container":
      return "must_have";
    case "language":
      return "languages";
    case "package_manager":
    case "compiler":
      return "package_managers";
    case "framework":
    case "mobile":
      return "frameworks";
    case "database":
      return "databases";
    case "middleware":
    case "monitoring":
      return "infrastructure";
    case "utility":
      return "must_have";
    case "tooling":
    default:
      return "tooling";
  }
}

/**
 * Возвращает ранг популярности инструмента внутри секции (меньше = популярнее).
 */
export function getToolPopularityRank(toolId: string): number {
  return TOOL_CONFIG[toolId]?.rank ?? 99;
}

export interface SectionGroup<T> {
  meta: ToolchainSectionMeta;
  items: T[];
  totalCount: number;
  installedCount?: number;
  updateCount?: number;
}
