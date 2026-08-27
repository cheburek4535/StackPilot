/**
 * Canonical navigation model.
 *
 * Nav items point at routes that render content TODAY. DevLauncher owns the
 * canonical `/devlauncher` hub (Overview / Profiles / Processes) — Analyze is
 * its own module and lives outside the DevLauncher group; the legacy aliases
 * still work (SPA redirects), so the match rules cover both:
 *   /devlauncher/**                                -> DevLauncher
 *   /analyze, /profiles/**, /processes             -> DevLauncher (legacy)
 *   /workspace/**                                  -> Workspace
 *   /project-creator, /create                      -> Project Creator
 *   /toolchain (canonical), /environment (alias)   -> Toolchain
 *   /settings                                      -> Settings
 */

export type NavMatch = (pathname: string) => boolean;

export type NavItem = {
  id: string;
  label: string;
  href: string;
  icon: string;
  match: NavMatch;
};

export type NavGroup = {
  id: string;
  label: string | null;
  items: NavItem[];
};

const exact = (paths: string[]): NavMatch => (p) => paths.includes(p);
const prefix = (paths: string[]): NavMatch => (p) =>
  paths.some((path) => p === path || p.startsWith(path + "/"));

export const NAV_GROUPS: NavGroup[] = [
  {
    id: "general",
    label: null,
    items: [
      {
        id: "home",
        label: "nav.home",
        href: "/",
        icon: "home",
        match: exact(["/"]),
      },
    ],
  },
  {
    id: "devlauncher",
    label: "nav.devlauncher",
    items: [
      {
        id: "devlauncher",
        label: "nav.devlauncher.overview",
        href: "/devlauncher",
        icon: "layers",
        match: exact(["/devlauncher"]),
      },
      {
        id: "profiles",
        label: "nav.devlauncher.profiles",
        href: "/devlauncher/profiles",
        icon: "bookmark",
        match: prefix(["/devlauncher/profiles", "/profiles"]),
      },
      {
        id: "processes",
        label: "nav.devlauncher.processes",
        href: "/devlauncher/processes",
        icon: "terminal",
        match: prefix(["/devlauncher/processes", "/processes"]),
      },
    ],
  },
  {
    id: "analyze",
    label: null,
    items: [
      {
        id: "analyze",
        label: "nav.analyze",
        href: "/devlauncher/analyze",
        icon: "search",
        match: exact(["/devlauncher/analyze", "/analyze"]),
      },
    ],
  },
  {
    id: "create",
    label: null,
    items: [
      {
        id: "project-creator",
        label: "nav.project_creator",
        href: "/create",
        icon: "sparkles",
        match: exact(["/project-creator", "/create"]),
      },
    ],
  },
  {
    id: "toolchain",
    label: null,
    items: [
      {
        id: "toolchain",
        label: "nav.toolchain",
        href: "/toolchain",
        icon: "wrench",
        match: exact(["/toolchain", "/environment"]),
      },
    ],
  },
  {
    id: "workspace",
    label: null,
    items: [
      {
        id: "workspace",
        label: "nav.workspace",
        href: "/workspace",
        icon: "folder",
        match: prefix(["/workspace"]),
      },
    ],
  },
  {
    id: "settings",
    label: null,
    items: [
      {
        id: "settings",
        label: "nav.settings",
        href: "/settings",
        icon: "settings",
        match: exact(["/settings"]),
      },
    ],
  },
  {
    id: "roadmap",
    label: null,
    items: [
      {
        id: "roadmap",
        label: "nav.roadmap",
        href: "/roadmap",
        icon: "map",
        match: exact(["/roadmap"]),
      },
    ],
  },
];

export function isNavItemActive(item: NavItem, pathname: string): boolean {
  return item.match(pathname);
}

export function isNavGroupActive(group: NavGroup, pathname: string): boolean {
  return group.items.some((item) => item.match(pathname));
}