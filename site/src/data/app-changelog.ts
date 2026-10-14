import { parseChangelogSection, formatDisplayDate } from './changelog';
import { resolveAppForScope } from './commit-apps';
import type { App, ReleaseChangeItem } from './types';

export interface AppReleaseHistory {
  version: string;
  releaseDate: string;
  changes: ReleaseChangeItem[];
}

export function findLatestReleaseForApp(
  changelogMarkdown: string,
  app: App,
): AppReleaseHistory | null {
  if (!changelogMarkdown) return null;

  const lines = changelogMarkdown.split('\n');
  const headerRegex =
    /^##\s+(?:\[([0-9a-zA-Z.-]+)\](?:\([^)]+\))?|([0-9a-zA-Z.-]+))(?:\s+\(([^)]+)\))?/;

  let currentVersion: string | null = null;
  let currentDate: string | null = null;
  let currentSectionLines: string[] = [];

  const processSection = (): AppReleaseHistory | null => {
    if (!currentVersion || currentSectionLines.length === 0) return null;
    const items = parseChangelogSection(currentSectionLines.join('\n'));
    const matchingChanges: ReleaseChangeItem[] = [];

    for (const item of items) {
      const matched = resolveAppForScope(item.scope, [app]);
      if (matched && matched.packageName === app.packageName) {
        matchingChanges.push({
          ...item,
          appName: matched.name,
          appPackageName: matched.packageName,
        });
      }
    }

    if (matchingChanges.length > 0) {
      return {
        version: currentVersion,
        releaseDate: currentDate ? formatDisplayDate(currentDate) : 'Recent',
        changes: matchingChanges,
      };
    }
    return null;
  };

  for (const line of lines) {
    const match = line.match(headerRegex);
    if (match) {
      const found = processSection();
      if (found) return found;

      const rawVersion = match[1] || match[2];
      currentVersion = rawVersion.replace(/^v/, '');
      currentDate = match[3] || '';
      currentSectionLines = [];
    } else {
      currentSectionLines.push(line);
    }
  }

  return processSection();
}
