/**
 * Canonical navigation model.
 *
 * Nav items point at routes that render content TODAY. DevLauncher now owns
 * the canonical `/devlauncher` hub (Overview / Analyze / Profiles / Processes);
 * the legacy aliases still work (SPA redirects), so the match rules cover both:
 *   /devlauncher/**                                -> DevLauncher
 *   /analyze, /profiles/**, /processes             -> DevLauncher (legacy)
 *   /workspace/**                                  -> Workspace
 *   /project-creator, /create                      -> Project Creator
 *   /toolchain, /environment                       -> Toolchain
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
        label: "Home",
        href: "/",
        icon: "home",
        match: exact(["/"]),
      },
    ],
  },
  {
    id: "devlauncher",
    label: "DevLauncher",
    items: [
      {
        id: "devlauncher",
        label: "Overview",
        href: "/devlauncher",
        icon: "layers",
        match: exact(["/devlauncher"]),
      },
      {
        id: "analyze",
        label: "Analyze",
        href: "/devlauncher/analyze",
        icon: "search",
        match: exact(["/devlauncher/analyze", "/analyze"]),
      },
      {
        id: "profiles",
        label: "Profiles",
        href: "/devlauncher/profiles",
        icon: "bookmark",
        match: prefix(["/devlauncher/profiles", "/profiles"]),
      },
      {
        id: "processes",
        label: "Processes",
        href: "/devlauncher/processes",
        icon: "terminal",
        match: prefix(["/devlauncher/processes", "/processes"]),
      },
    ],
  },
  {
    id: "create",
    label: null,
    items: [
      {
        id: "project-creator",
        label: "Project Creator",
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
        label: "Toolchain",
        href: "/environment",
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
        label: "Workspace",
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
        label: "Settings",
        href: "/settings",
        icon: "settings",
        match: exact(["/settings"]),
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