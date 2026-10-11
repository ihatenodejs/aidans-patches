import type {
  DownloadAssetRecord,
  DownloadSnapshot,
  DownloadStatistics,
  ReleaseLineDownloads,
  WorkerEnv,
} from './types';

export const DOWNLOAD_STATISTICS_KV_KEY = 'download_statistics:v1';
const MAX_PAGES = 50;
const SEMVER_TAG_REGEX = /^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/;

interface GitHubAssetResponse {
  id: number;
  download_count: number;
}

interface GitHubReleaseResponse {
  draft: boolean;
  tag_name: string;
  assets: GitHubAssetResponse[];
}

function validateGitHubRelease(
  item: unknown,
  index: number,
  page: number,
): GitHubReleaseResponse {
  if (typeof item !== 'object' || item === null) {
    throw new Error(
      `Invalid release at page ${page}, index ${index}: expected object, received ${typeof item}`,
    );
  }

  const record = item as Record<string, unknown>;

  if (typeof record.draft !== 'boolean') {
    throw new Error(
      `Invalid release at page ${page}, index ${index}: missing or non-boolean 'draft'`,
    );
  }

  if (
    typeof record.tag_name !== 'string' ||
    record.tag_name.trim().length === 0
  ) {
    throw new Error(
      `Invalid release at page ${page}, index ${index}: missing or non-string 'tag_name'`,
    );
  }

  if (!Array.isArray(record.assets)) {
    throw new Error(
      `Invalid release at page ${page}, index ${index}: missing or non-array 'assets'`,
    );
  }

  const validatedAssets: GitHubAssetResponse[] = [];
  for (let aIdx = 0; aIdx < record.assets.length; aIdx++) {
    const asset = record.assets[aIdx];
    if (typeof asset !== 'object' || asset === null) {
      throw new Error(
        `Invalid asset at page ${page}, release ${index}, asset ${aIdx}: expected object`,
      );
    }
    const assetRecord = asset as Record<string, unknown>;
    if (
      typeof assetRecord.id !== 'number' ||
      !Number.isSafeInteger(assetRecord.id)
    ) {
      throw new Error(
        `Invalid asset at page ${page}, release ${index}, asset ${aIdx}: invalid 'id'`,
      );
    }
    if (
      typeof assetRecord.download_count !== 'number' ||
      !Number.isSafeInteger(assetRecord.download_count) ||
      assetRecord.download_count < 0
    ) {
      throw new Error(
        `Invalid asset at page ${page}, release ${index}, asset ${aIdx}: invalid 'download_count'`,
      );
    }
    validatedAssets.push({
      id: assetRecord.id,
      download_count: assetRecord.download_count,
    });
  }

  return {
    draft: record.draft,
    tag_name: record.tag_name,
    assets: validatedAssets,
  };
}

export async function loadDownloadStatistics(
  env: WorkerEnv,
): Promise<DownloadStatistics | null> {
  if (!env.PATCHES_KV) return null;
  const raw = await env.PATCHES_KV.get(DOWNLOAD_STATISTICS_KV_KEY);
  if (!raw) return null;

  try {
    const parsed = JSON.parse(raw) as DownloadStatistics;
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      typeof parsed.updatedAt === 'string' &&
      typeof parsed.assets === 'object' &&
      parsed.assets !== null &&
      Array.isArray(parsed.history)
    ) {
      return parsed;
    }
    return null;
  } catch {
    return null;
  }
}

export async function fetchAllGitHubReleases(
  repo: string,
): Promise<GitHubReleaseResponse[]> {
  const allReleases: GitHubReleaseResponse[] = [];
  let page = 1;

  while (true) {
    if (page > MAX_PAGES) {
      throw new Error(
        `GitHub releases pagination exceeded safety limit of ${MAX_PAGES} pages for ${repo}`,
      );
    }

    const url = `https://api.github.com/repos/${repo}/releases?per_page=100&page=${page}`;
    const res = await fetch(url, {
      headers: {
        Accept: 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
        'User-Agent':
          'aidans-patches-worker/1.0 (+https://github.com/ihatenodejs/aidans-patches)',
      },
    });

    if (!res.ok) {
      throw new Error(
        `GitHub API releases fetch failed: HTTP ${res.status} ${res.statusText} for ${url}`,
      );
    }

    const data: unknown = await res.json();
    if (!Array.isArray(data)) {
      throw new Error(
        `GitHub API releases response at page ${page} was not an array`,
      );
    }

    const validatedPage: GitHubReleaseResponse[] = [];
    for (let i = 0; i < data.length; i++) {
      validatedPage.push(validateGitHubRelease(data[i], i, page));
    }

    allReleases.push(...validatedPage);

    if (validatedPage.length < 100) {
      break;
    }

    page++;
  }

  return allReleases;
}

export function mergeReleaseAssets(
  previousAssets: Record<string, DownloadAssetRecord>,
  releases: GitHubReleaseResponse[],
): Record<string, DownloadAssetRecord> {
  // Start with shallow copy of previous assets so unobserved / deleted assets are retained
  const merged: Record<string, DownloadAssetRecord> = { ...previousAssets };

  for (const release of releases) {
    if (release.draft) {
      continue;
    }

    for (const asset of release.assets) {
      const assetKey = String(asset.id);
      const existing = merged[assetKey];

      if (existing) {
        merged[assetKey] = {
          releaseTag: release.tag_name,
          downloadCount: Math.max(existing.downloadCount, asset.download_count),
        };
      } else {
        merged[assetKey] = {
          releaseTag: release.tag_name,
          downloadCount: asset.download_count,
        };
      }
    }
  }

  return merged;
}

export function calculateTotalDownloads(
  assets: Record<string, DownloadAssetRecord>,
): number {
  let total = 0;
  for (const asset of Object.values(assets)) {
    total += asset.downloadCount;
  }
  return total;
}

export function updateDownloadHistory(
  existingHistory: DownloadSnapshot[],
  utcDate: string,
  totalDownloads: number,
): DownloadSnapshot[] {
  const history = [...existingHistory];
  const existingIdx = history.findIndex((h) => h.date === utcDate);

  if (existingIdx >= 0) {
    history[existingIdx] = { date: utcDate, totalDownloads };
  } else {
    history.push({ date: utcDate, totalDownloads });
  }

  history.sort((a, b) => a.date.localeCompare(b.date));
  return history;
}

export function deriveReleaseLineDownloads(
  assets: Record<string, DownloadAssetRecord>,
): ReleaseLineDownloads[] {
  const groups: Record<
    string,
    { totalDownloads: number; major: number; minor: number }
  > = {};

  for (const asset of Object.values(assets)) {
    const match = SEMVER_TAG_REGEX.exec(asset.releaseTag);
    if (!match) {
      continue;
    }

    const major = parseInt(match[1], 10);
    const minor = parseInt(match[2], 10);
    const label = `v${major}.${minor}`;

    if (!groups[label]) {
      groups[label] = { totalDownloads: 0, major, minor };
    }
    groups[label].totalDownloads += asset.downloadCount;
  }

  const result: ReleaseLineDownloads[] = Object.entries(groups).map(
    ([label, data]) => ({
      label,
      totalDownloads: data.totalDownloads,
      major: data.major,
      minor: data.minor,
    }),
  );

  result.sort((a, b) => {
    if (a.major !== b.major) {
      return b.major - a.major;
    }
    return b.minor - a.minor;
  });

  return result;
}

export async function refreshDownloadStatistics(
  env: WorkerEnv,
  options?: { now?: Date },
): Promise<DownloadStatistics> {
  const repo = env.GITHUB_REPO;
  if (!repo) {
    throw new Error('GITHUB_REPO environment variable is not set');
  }
  if (!env.PATCHES_KV) {
    throw new Error('PATCHES_KV binding is not available');
  }

  const existingStats = await loadDownloadStatistics(env);
  const previousAssets = existingStats?.assets ?? {};
  const previousHistory = existingStats?.history ?? [];

  // Fetch all releases with full structural validation
  const releases = await fetchAllGitHubReleases(repo);

  // Merge assets
  const mergedAssets = mergeReleaseAssets(previousAssets, releases);
  const totalDownloads = calculateTotalDownloads(mergedAssets);

  // UTC date YYYY-MM-DD
  const now = options?.now ?? new Date();
  const utcDate = now.toISOString().slice(0, 10);
  const updatedHistory = updateDownloadHistory(
    previousHistory,
    utcDate,
    totalDownloads,
  );

  const newStats: DownloadStatistics = {
    updatedAt: now.toISOString(),
    assets: mergedAssets,
    history: updatedHistory,
  };

  await env.PATCHES_KV.put(
    DOWNLOAD_STATISTICS_KV_KEY,
    JSON.stringify(newStats),
  );

  return newStats;
}
