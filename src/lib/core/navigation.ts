/**
 * Canonical navigation model.
 *
 * Two sections only — the platform-level launcher and the active project:
 *
 *   Platform / Global:
 *     /            Home — project launcher (recent projects, quick actions)
 *     /toolchain   Toolchain — system-wide dev environment tools
 *     /create      Project Creator — scaffolding wizard
 *     /devlauncher/analyze   Analyze — DevLauncher project analysis
 *     /settings    Settings — app preferences
 *
 *   Active Project:
 *     /workspace   Workspace — the single unified home for the active project.
 *
 * Legacy /devlauncher and alias routes (/profiles, /processes, /analyze,
 * /environment) still exist as SPA/server redirects into the canonical
 * workspace routes, so deep links and restored routes never crash. They match
 * the Workspace nav item so the sidebar highlights correctly while redirecting.
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
    id: "platform",
    label: "nav.section.platform",
    items: [
      {
        id: "home",
        label: "nav.home",
        href: "/",
        icon: "home",
        match: exact(["/"]),
      },
      {
        id: "toolchain",
        label: "nav.toolchain",
        href: "/toolchain",
        icon: "wrench",
        match: exact(["/toolchain", "/environment"]),
      },
      {
        id: "project-creator",
        label: "nav.project_creator",
        href: "/create",
        icon: "sparkles",
        match: exact(["/project-creator", "/create"]),
      },
      {
        id: "analyze",
        label: "nav.analyze",
        href: "/devlauncher/analyze",
        icon: "search",
        match: exact(["/devlauncher/analyze", "/analyze"]),
      },
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
    id: "project",
    label: "nav.section.project",
    items: [
      {
        id: "workspace",
        label: "nav.workspace",
        href: "/workspace",
        icon: "folder",
        match: (p) =>
          prefix(["/workspace", "/profiles", "/processes"])(p) ||
          (p.startsWith("/devlauncher") && !p.startsWith("/devlauncher/analyze")),
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