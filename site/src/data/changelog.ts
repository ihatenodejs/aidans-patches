import type { RawBundle, ReleaseChangeItem, ReleaseInfo } from './types';

export function parseChangelogSection(markdown: string): ReleaseChangeItem[] {
  const items: ReleaseChangeItem[] = [];
  const lines = markdown.split('\n');
  let currentSectionType: 'feat' | 'fix' | 'bump' | 'perf' | 'other' = 'other';

  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('### ')) {
      const heading = trimmed.toLowerCase();
      if (
        heading.includes('feature') ||
        heading.includes('feat') ||
        heading.includes('✨')
      ) {
        currentSectionType = 'feat';
      } else if (
        heading.includes('fix') ||
        heading.includes('bug') ||
        heading.includes('🐛')
      ) {
        currentSectionType = 'fix';
      } else if (
        heading.includes('app support') ||
        heading.includes('bump') ||
        heading.includes('🚀')
      ) {
        currentSectionType = 'bump';
      } else if (
        heading.includes('improvement') ||
        heading.includes('perf') ||
        heading.includes('🔧')
      ) {
        currentSectionType = 'perf';
      } else {
        currentSectionType = 'other';
      }
      continue;
    }

    if (trimmed.startsWith('* ') || trimmed.startsWith('- ')) {
      const rawText = trimmed.replace(/^[* -]\s+/, '');
      // Match scope: **scope:** message ([hash](url))
      const scopeMatch = rawText.match(/^\*\*([^*:]+)(?::\*\*|\*\*:)\s*(.*)$/);
      let scope: string | null = null;
      let remainder = rawText;
      if (scopeMatch) {
        scope = scopeMatch[1].trim();
        remainder = scopeMatch[2].trim();
      }

      // Match commit link: ([hash](url))
      const commitMatch = remainder.match(
        /\(\[([a-f0-9]{7,})\]\((https:\/\/[^)]+)\)\)/i,
      );
      let hash: string | null = null;
      let commitUrl: string | null = null;
      let description = remainder;

      if (commitMatch) {
        hash = commitMatch[1];
        commitUrl = commitMatch[2];
        description = remainder.replace(commitMatch[0], '').trim();
      }

      items.push({
        type: currentSectionType,
        scope,
        description,
        hash,
        commitUrl,
      });
    }
  }

  return items;
}

export function parseReleaseInfo(bundleJson: {
  version: string;
  created_at: string;
  description: string;
  download_url: string;
}): ReleaseInfo {
  const rawVersion = bundleJson.version || '';
  const version = rawVersion.replace(/^v/, '');
  const recentChanges = parseChangelogSection(bundleJson.description || '');
  return {
    version,
    releaseDate: bundleJson.created_at
      ? formatDisplayDate(bundleJson.created_at)
      : 'Recent',
    downloadUrl: bundleJson.download_url || '',
    rawChangelog: bundleJson.description || '',
    recentChanges,
  };
}

export function isPreRelease(version: string): boolean {
  if (!version) return true;
  const clean = version.trim().replace(/^v/, '');
  return clean.includes('-') || !/^\d+\.\d+\.\d+$/.test(clean);
}

export function extractLatestStableFromChangelog(changelogMarkdown: string): {
  version: string;
  date: string;
  body: string;
} | null {
  const lines = changelogMarkdown.split('\n');
  const headerRegex =
    /^##\s+(?:\[([0-9a-zA-Z.-]+)\](?:\([^)]+\))?|([0-9a-zA-Z.-]+))(?:\s+\(([^)]+)\))?/;
  let currentVersion: string | null = null;
  let currentDate: string | null = null;
  let capturing = false;
  const capturedLines: string[] = [];

  for (const line of lines) {
    const match = line.match(headerRegex);
    if (match) {
      const rawVersion = match[1] || match[2];
      const version = rawVersion.replace(/^v/, '');
      const date = match[3] || '';
      const isPre = isPreRelease(version);

      if (!isPre && !capturing) {
        capturing = true;
        currentVersion = version;
        currentDate = date;
      } else if (capturing) {
        break;
      }
    } else if (capturing) {
      capturedLines.push(line);
    }
  }

  if (!currentVersion) return null;

  return {
    version: currentVersion,
    date: currentDate || '',
    body: capturedLines.join('\n').trim(),
  };
}

export async function fetchLatestGitHubRelease(
  repo: string = 'ihatenodejs/aidans-patches',
): Promise<ReleaseInfo | null> {
  try {
    const headers: Record<string, string> = {
      Accept: 'application/vnd.github.v3+json',
      'User-Agent': 'aidans-patches-site',
    };
    const token =
      typeof process !== 'undefined' ? process.env?.GITHUB_TOKEN : undefined;
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    const response = await fetch(
      `https://api.github.com/repos/${repo}/releases/latest`,
      {
        headers,
        signal: AbortSignal.timeout(5000),
      },
    );

    if (!response.ok) {
      return null;
    }

    const data = await response.json();
    if (!data || data.prerelease || data.draft) {
      return null;
    }

    const rawVersion = String(data.tag_name || data.name || '').trim();
    const version = rawVersion.replace(/^v/, '');
    if (isPreRelease(version)) {
      return null;
    }

    const releaseDate =
      data.published_at || data.created_at
        ? formatDisplayDate(data.published_at || data.created_at)
        : 'Recent';

    let downloadUrl = `https://github.com/${repo}/releases/download/v${version}/patches-${version}.mpp`;
    if (Array.isArray(data.assets)) {
      const mppAsset = data.assets.find(
        (a: { name?: string; browser_download_url?: string }) =>
          typeof a.name === 'string' && a.name.endsWith('.mpp'),
      );
      if (mppAsset?.browser_download_url) {
        downloadUrl = mppAsset.browser_download_url;
      }
    }

    const rawChangelog = data.body || '';
    const recentChanges = parseChangelogSection(rawChangelog);

    return {
      version,
      releaseDate,
      downloadUrl,
      rawChangelog,
      recentChanges,
    };
  } catch {
    return null;
  }
}

export function resolveLatestReleaseSync(options: {
  bundleJson?: RawBundle | null;
  changelogContent?: string | null;
  repo?: string;
}): ReleaseInfo {
  const repo = options.repo || 'ihatenodejs/aidans-patches';

  // 1. If bundleJson is available and NOT a pre-release, use it
  if (options.bundleJson && options.bundleJson.version) {
    const cleanVersion = options.bundleJson.version.replace(/^v/, '');
    if (!isPreRelease(cleanVersion)) {
      return {
        version: cleanVersion,
        releaseDate: options.bundleJson.created_at
          ? formatDisplayDate(options.bundleJson.created_at)
          : 'Recent',
        downloadUrl:
          options.bundleJson.download_url ||
          `https://github.com/${repo}/releases/download/v${cleanVersion}/patches-${cleanVersion}.mpp`,
        rawChangelog: options.bundleJson.description || '',
        recentChanges: parseChangelogSection(
          options.bundleJson.description || '',
        ),
      };
    }
  }

  // 2. If changelogContent is available, find latest stable entry
  if (options.changelogContent) {
    const stable = extractLatestStableFromChangelog(options.changelogContent);
    if (stable) {
      const version = stable.version;
      const downloadUrl = `https://github.com/${repo}/releases/download/v${version}/patches-${version}.mpp`;
      return {
        version,
        releaseDate: stable.date ? formatDisplayDate(stable.date) : 'Recent',
        downloadUrl,
        rawChangelog: stable.body,
        recentChanges: parseChangelogSection(stable.body),
      };
    }
  }

  // 3. Fallback to bundleJson even if pre-release
  if (options.bundleJson) {
    const cleanVersion = (options.bundleJson.version || '1.0.0').replace(
      /^v/,
      '',
    );
    return {
      version: cleanVersion,
      releaseDate: options.bundleJson.created_at
        ? formatDisplayDate(options.bundleJson.created_at)
        : 'Recent',
      downloadUrl: options.bundleJson.download_url || '',
      rawChangelog: options.bundleJson.description || '',
      recentChanges: parseChangelogSection(
        options.bundleJson.description || '',
      ),
    };
  }

  return {
    version: '1.0.0',
    releaseDate: 'Recent',
    downloadUrl: '',
    rawChangelog: '',
    recentChanges: [],
  };
}

export async function resolveLatestRelease(options: {
  bundleJson?: RawBundle | null;
  changelogContent?: string | null;
  repo?: string;
}): Promise<ReleaseInfo> {
  const ghRelease = await fetchLatestGitHubRelease(options.repo);
  if (ghRelease) {
    return ghRelease;
  }
  return resolveLatestReleaseSync(options);
}

export function formatDisplayDate(dateStr: string): string {
  if (!dateStr) return 'Recent';
  const datePart = dateStr.split('T')[0];
  const [yearStr, monthStr, dayStr] = datePart.split('-');
  const year = parseInt(yearStr, 10);
  const month = parseInt(monthStr, 10);
  const day = parseInt(dayStr, 10);
  if (isNaN(year) || isNaN(month) || isNaN(day)) return dateStr;
  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  return `${months[month - 1]} ${day}, ${year}`;
}
