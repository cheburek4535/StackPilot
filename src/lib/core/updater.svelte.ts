import { APP_VERSION, GITHUB_REPO, GITHUB_RELEASES_URL } from "./app";

export interface ReleaseInfo {
  tagName: string;
  version: string;
  name: string;
  htmlUrl: string;
  publishedAt?: string;
}

/**
 * Extracts normalized version number (e.g. "v1.4.0" -> "1.4.0", "1.5.0-rc1" -> "1.5.0-rc1").
 */
export function extractVersion(tag: string): string {
  if (!tag) return "0.0.0";
  const trimmed = tag.trim();
  const match = trimmed.match(/(\d+\.\d+(?:\.\d+)?(?:-[0-9A-Za-z.-]+)?)/);
  if (match) return match[1];
  return trimmed.replace(/^[vV]/, "");
}

/**
 * Compares two semantic version strings.
 * Returns:
 *   1 if a > b (a is newer than b)
 *  -1 if a < b (a is older than b)
 *   0 if a == b (equal)
 */
export function compareVersions(a: string, b: string): number {
  const verA = extractVersion(a);
  const verB = extractVersion(b);

  const [coreA, preA] = verA.split("-", 2);
  const [coreB, preB] = verB.split("-", 2);

  const partsA = coreA.split(".").map((n) => parseInt(n, 10) || 0);
  const partsB = coreB.split(".").map((n) => parseInt(n, 10) || 0);

  const maxLen = Math.max(partsA.length, partsB.length, 3);
  for (let i = 0; i < maxLen; i++) {
    const numA = partsA[i] ?? 0;
    const numB = partsB[i] ?? 0;
    if (numA > numB) return 1;
    if (numA < numB) return -1;
  }

  // According to SemVer: a version with no prerelease is greater than one with prerelease (1.4.0 > 1.4.0-rc1)
  if (!preA && preB) return 1;
  if (preA && !preB) return -1;
  if (preA && preB) {
    return preA.localeCompare(preB, undefined, { numeric: true });
  }

  return 0;
}

/**
 * Checks if remote tag represents a version strictly newer than current.
 */
export function isNewerVersion(remoteTag: string, currentVersion: string = APP_VERSION): boolean {
  return compareVersions(remoteTag, currentVersion) > 0;
}

/**
 * Opens a release URL using Tauri's opener plugin or browser fallback.
 */
export async function openReleaseUrl(url: string): Promise<void> {
  const target = url || GITHUB_RELEASES_URL;
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(target);
  } catch {
    if (typeof window !== "undefined") {
      window.open(target, "_blank", "noopener,noreferrer");
    }
  }
}

/**
 * Reactive service checking GitHub for new StackPilot releases upon application startup.
 */
class UpdaterService {
  checking = $state(false);
  hasUpdate = $state(false);
  dismissed = $state(false);
  latestRelease = $state<ReleaseInfo | null>(null);
  error = $state<string | null>(null);
  lastChecked = $state<number | null>(null);

  /**
   * Check GitHub Releases API for new version.
   * Runs non-blocking with a safe timeout, completely resilient to offline / rate-limit scenarios.
   */
  async check(force = false): Promise<boolean> {
    if (this.checking) return false;
    if (this.lastChecked && !force && (this.hasUpdate || this.dismissed)) {
      return this.hasUpdate;
    }

    this.checking = true;
    this.error = null;

    try {
      const endpoints = [
        `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`,
        `https://api.github.com/repos/cheburek4535/devlauncher/releases/latest`,
      ];

      let releaseData: any = null;

      for (const url of endpoints) {
        try {
          const res = await fetch(url, {
            headers: {
              Accept: "application/vnd.github.v3+json",
            },
            signal: AbortSignal.timeout(6000),
          });

          if (res.ok) {
            releaseData = await res.json();
            if (releaseData?.tag_name) break;
          }
        } catch {
          // Continue to next endpoint if failed
        }
      }

      this.lastChecked = Date.now();

      if (!releaseData || !releaseData.tag_name) {
        this.checking = false;
        return false;
      }

      const tagName = String(releaseData.tag_name);
      const parsedVersion = extractVersion(tagName);
      const htmlUrl =
        releaseData.html_url ||
        `https://github.com/${GITHUB_REPO}/releases/tag/${tagName}`;

      if (isNewerVersion(tagName, APP_VERSION)) {
        this.latestRelease = {
          tagName,
          version: parsedVersion,
          name: releaseData.name || `StackPilot ${tagName}`,
          htmlUrl,
          publishedAt: releaseData.published_at,
        };
        this.hasUpdate = true;
      } else {
        this.hasUpdate = false;
      }

      return this.hasUpdate;
    } catch (e) {
      this.error = String(e);
      return false;
    } finally {
      this.checking = false;
    }
  }

  /**
   * Dismiss the banner for this session.
   */
  dismiss(): void {
    this.dismissed = true;
  }

  /**
   * Open the latest release page in default browser.
   */
  async openRelease(): Promise<void> {
    const url = this.latestRelease?.htmlUrl || GITHUB_RELEASES_URL;
    await openReleaseUrl(url);
  }
}

export const updater = new UpdaterService();
