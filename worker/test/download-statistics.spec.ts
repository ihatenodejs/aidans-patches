import { env } from 'cloudflare:test';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  deriveReleaseLineDownloads,
  DOWNLOAD_STATISTICS_KV_KEY,
  loadDownloadStatistics,
  refreshDownloadStatistics,
} from '../src/download-statistics';
import worker, { runScheduledTasks } from '../src/index';
import type { DownloadStatistics, WorkerEnv } from '../src/types';

describe('Download statistics collection and aggregation', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  const workerEnv: WorkerEnv = {
    ...env,
    PATCHES_KV: env.PATCHES_KV,
    GITHUB_REPO: 'ihatenodejs/aidans-patches',
  };

  it('paginates GitHub releases, aggregates stable and prereleases by line, excludes drafts, and keeps non-semver in total only', async () => {
    // Page 1: 100 items (first few real, then dummy filler to trigger page 2)
    const page1Items = [
      {
        draft: false,
        tag_name: 'v1.6.0',
        assets: [{ id: 101, download_count: 50 }],
      },
      {
        draft: false,
        tag_name: 'v1.6.0-dev.1',
        assets: [{ id: 102, download_count: 30 }],
      },
      {
        draft: false,
        tag_name: 'v1.6.1',
        assets: [
          { id: 103, download_count: 20 },
          { id: 104, download_count: 10 },
        ],
      },
      {
        draft: false,
        tag_name: 'nightly-build',
        assets: [{ id: 105, download_count: 15 }],
      },
      {
        draft: true,
        tag_name: 'v1.7.0',
        assets: [{ id: 106, download_count: 100 }],
      },
    ];

    // Pad page 1 to 100 items so pagination continues to page 2
    for (let i = page1Items.length; i < 100; i++) {
      page1Items.push({
        draft: false,
        tag_name: `v1.0.${i}`,
        assets: [{ id: 2000 + i, download_count: 1 }],
      });
    }

    const page2Items = [
      {
        draft: false,
        tag_name: 'v1.5.0',
        assets: [{ id: 107, download_count: 40 }],
      },
    ];

    let page1Requested = false;
    let page2Requested = false;

    globalThis.fetch = vi.fn(
      async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input.toString();
        const headers = init?.headers as Record<string, string> | undefined;

        expect(headers?.Accept).toBe('application/vnd.github+json');
        expect(headers?.['X-GitHub-Api-Version']).toBe('2022-11-28');
        expect(headers?.['User-Agent']).toContain('aidans-patches-worker');
        const parsedUrl = new URL(url);
        const pageParam = parsedUrl.searchParams.get('page');

        if (pageParam === '1') {
          page1Requested = true;
          return new Response(JSON.stringify(page1Items), { status: 200 });
        }
        if (pageParam === '2') {
          page2Requested = true;
          return new Response(JSON.stringify(page2Items), { status: 200 });
        }
        return new Response('Not found', { status: 404 });
      },
    );

    const fixedNow = new Date('2026-03-30T12:00:00Z');
    const stats = await refreshDownloadStatistics(workerEnv, { now: fixedNow });

    expect(page1Requested).toBe(true);
    expect(page2Requested).toBe(true);

    // Draft asset 106 must be excluded
    expect(stats.assets['106']).toBeUndefined();

    // Published assets must be retained
    expect(stats.assets['101'].downloadCount).toBe(50);
    expect(stats.assets['102'].downloadCount).toBe(30);
    expect(stats.assets['103'].downloadCount).toBe(20);
    expect(stats.assets['104'].downloadCount).toBe(10);
    expect(stats.assets['105'].downloadCount).toBe(15);
    expect(stats.assets['107'].downloadCount).toBe(40);

    // Total downloads includes non-semver (105: 15) and padding assets (95 items * 1 = 95)
    // 50 + 30 + 20 + 10 + 15 + 40 + 95 = 260
    expect(stats.history[0].totalDownloads).toBe(260);
    expect(stats.history[0].date).toBe('2026-03-30');

    // Release line grouping
    const lines = deriveReleaseLineDownloads(stats.assets);
    const lineV1_6 = lines.find((l) => l.label === 'v1.6');
    const lineV1_5 = lines.find((l) => l.label === 'v1.5');
    expect(lineV1_6).toBeDefined();
    expect(lineV1_6?.totalDownloads).toBe(110); // 50 + 30 + 20 + 10

    expect(lineV1_5).toBeDefined();
    expect(lineV1_5?.totalDownloads).toBe(40);

    // Non-semver release tag 'nightly-build' is omitted from release line bars
    const nonSemverLine = lines.find((l) => l.label.includes('nightly'));
    expect(nonSemverLine).toBeUndefined();

    // Lines sorted descending by major then minor
    expect(lines[0].major).toBeGreaterThanOrEqual(lines[1].major);
    if (lines[0].major === lines[1].major) {
      expect(lines[0].minor).toBeGreaterThanOrEqual(lines[1].minor);
    }
  });

  it('retains missing assets, never lowers counts, and replaces same-day snapshot without duplicates', async () => {
    // Seed existing record
    const priorStats: DownloadStatistics = {
      updatedAt: '2026-03-30T00:00:00Z',
      assets: {
        '999': { releaseTag: 'v1.4.0', downloadCount: 88 }, // now missing from API
        '101': { releaseTag: 'v1.6.0', downloadCount: 60 }, // prior count is 60
      },
      history: [
        { date: '2026-03-29', totalDownloads: 100 },
        { date: '2026-03-30', totalDownloads: 148 },
      ],
    };
    await env.PATCHES_KV.put(
      DOWNLOAD_STATISTICS_KV_KEY,
      JSON.stringify(priorStats),
    );

    // Mock API returning observed count 50 for asset 101 (lower than prior 60)
    // and omitting asset 999
    globalThis.fetch = vi.fn(async () => {
      return new Response(
        JSON.stringify([
          {
            draft: false,
            tag_name: 'v1.6.0',
            assets: [{ id: 101, download_count: 50 }],
          },
        ]),
        { status: 200 },
      );
    });

    const fixedNow = new Date('2026-03-30T15:00:00Z');
    const updated = await refreshDownloadStatistics(workerEnv, {
      now: fixedNow,
    });

    // Missing asset 999 retained
    expect(updated.assets['999']).toBeDefined();
    expect(updated.assets['999'].downloadCount).toBe(88);

    // Asset 101 retains highest observed count (60, not decreased to 50)
    expect(updated.assets['101'].downloadCount).toBe(60);

    // Total downloads = 88 + 60 = 148
    // History should not duplicate 2026-03-30
    expect(updated.history).toHaveLength(2);
    expect(updated.history[0].date).toBe('2026-03-29');
    expect(updated.history[1].date).toBe('2026-03-30');
    expect(updated.history[1].totalDownloads).toBe(148);
  });

  it('fails atomically when a pagination page fails and leaves KV byte-for-byte unchanged', async () => {
    const originalSeededJson = JSON.stringify({
      updatedAt: '2026-03-28T00:00:00Z',
      assets: {
        '555': { releaseTag: 'v1.2.0', downloadCount: 42 },
      },
      history: [{ date: '2026-03-28', totalDownloads: 42 }],
    });
    await env.PATCHES_KV.put(DOWNLOAD_STATISTICS_KV_KEY, originalSeededJson);

    // Page 1 succeeds with 100 items; Page 2 returns 500 error
    const page1Items = [];
    for (let i = 0; i < 100; i++) {
      page1Items.push({
        draft: false,
        tag_name: `v1.0.${i}`,
        assets: [{ id: 3000 + i, download_count: 2 }],
      });
    }

    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      const parsedUrl = new URL(url);
      const pageParam = parsedUrl.searchParams.get('page');
      if (pageParam === '1') {
        return new Response(JSON.stringify(page1Items), { status: 200 });
      }
      return new Response('Internal Server Error', { status: 500 });
    });

    await expect(refreshDownloadStatistics(workerEnv)).rejects.toThrow(
      /HTTP 500/,
    );

    // KV must remain untouched
    const currentKv = await env.PATCHES_KV.get(DOWNLOAD_STATISTICS_KV_KEY);
    expect(currentKv).toBe(originalSeededJson);
  });

  it('fails atomically when response shape is malformed', async () => {
    const originalSeededJson = JSON.stringify({
      updatedAt: '2026-03-28T00:00:00Z',
      assets: {},
      history: [],
    });
    await env.PATCHES_KV.put(DOWNLOAD_STATISTICS_KV_KEY, originalSeededJson);

    globalThis.fetch = vi.fn(async () => {
      return new Response(
        JSON.stringify([
          {
            draft: false,
            tag_name: 'v1.0.0',
            assets: [{ id: 'not-a-number', download_count: 5 }],
          },
        ]),
        { status: 200 },
      );
    });

    await expect(refreshDownloadStatistics(workerEnv)).rejects.toThrow(
      /invalid 'id'/,
    );

    const currentKv = await env.PATCHES_KV.get(DOWNLOAD_STATISTICS_KV_KEY);
    expect(currentKv).toBe(originalSeededJson);
  });
});

describe('Scheduled task dispatching', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  const workerEnv: WorkerEnv = {
    ...env,
    PATCHES_KV: env.PATCHES_KV,
    GITHUB_REPO: 'ihatenodejs/aidans-patches',
  };

  it('runs performVersionCheck only for 0 */6 * * * and never calls GitHub API', async () => {
    let playStoreCalled = false;
    let githubCalled = false;

    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.includes('api.github.com')) {
        githubCalled = true;
        throw new Error('GitHub API should not be called');
      }
      if (url.includes('play.google.com')) {
        playStoreCalled = true;
        return new Response(
          '<html><body>[[[["1.0.0"]],[[[1]],[[[1]]]]</body></html>',
          { status: 200 },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    await runScheduledTasks('0 */6 * * *', workerEnv);

    expect(playStoreCalled).toBe(true);
    expect(githubCalled).toBe(false);
  });

  it('runs refreshDownloadStatistics only for 15 0 * * * and never calls Google Play', async () => {
    let playStoreCalled = false;
    let githubCalled = false;

    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.includes('play.google.com')) {
        playStoreCalled = true;
        throw new Error('Google Play should not be called');
      }
      if (url.includes('api.github.com')) {
        githubCalled = true;
        return new Response(
          JSON.stringify([
            {
              draft: false,
              tag_name: 'v1.6.0',
              assets: [{ id: 501, download_count: 120 }],
            },
          ]),
          { status: 200 },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    await runScheduledTasks('15 0 * * *', workerEnv);

    expect(githubCalled).toBe(true);
    expect(playStoreCalled).toBe(false);

    const stats = await loadDownloadStatistics(workerEnv);
    expect(stats?.assets['501'].downloadCount).toBe(120);
  });

  it('ignores unknown cron expressions gracefully without mutating state', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    let fetchCalled = false;

    globalThis.fetch = vi.fn(async () => {
      fetchCalled = true;
      return new Response('ok', { status: 200 });
    });

    await runScheduledTasks('0 12 * * *', workerEnv);

    expect(fetchCalled).toBe(false);
    expect(warnSpy).toHaveBeenCalled();
  });
});

describe('Chart HTTP routes', () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  const workerEnv: WorkerEnv = {
    ...env,
    PATCHES_KV: env.PATCHES_KV,
    GITHUB_REPO: 'ihatenodejs/aidans-patches',
  };

  it('serves GET /charts/total-downloads.svg with 200, correct headers, dark-purple palette, and valid geometry', async () => {
    const testStats: DownloadStatistics = {
      updatedAt: '2026-03-30T10:00:00Z',
      assets: {
        '1': { releaseTag: 'v1.6.0', downloadCount: 1500 },
        '2': { releaseTag: 'v1.5.0', downloadCount: 500 },
      },
      history: [
        { date: '2026-03-28', totalDownloads: 1200 },
        { date: '2026-03-29', totalDownloads: 1800 },
        { date: '2026-03-30', totalDownloads: 2000 },
      ],
    };
    await env.PATCHES_KV.put(
      DOWNLOAD_STATISTICS_KV_KEY,
      JSON.stringify(testStats),
    );

    const req = new Request('http://localhost/charts/total-downloads.svg', {
      method: 'GET',
    });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toBe(
      'image/svg+xml; charset=utf-8',
    );
    expect(res.headers.get('Cache-Control')).toBe(
      'public, max-age=3600, s-maxage=3600',
    );

    const body = await res.text();
    expect(body).toContain('<svg');
    expect(body).toContain('Total Downloads');
    expect(body).toContain('#160d2b'); // dark-purple background
    expect(body).toContain('#4c3575'); // grid/axes
    expect(body).toContain('#a78bfa'); // violet accent
    expect(body).toContain('Updated 2026-03-30');
    expect(body).not.toContain('NaN');
    expect(body).not.toContain('Infinity');
  });

  it('serves GET /charts/release-downloads.svg with 200, correct headers, grouped bars, and valid geometry', async () => {
    const testStats: DownloadStatistics = {
      updatedAt: '2026-03-30T10:00:00Z',
      assets: {
        '1': { releaseTag: 'v1.6.0', downloadCount: 1500 },
        '2': { releaseTag: 'v1.6.0-dev.1', downloadCount: 200 },
        '3': { releaseTag: 'v1.5.0', downloadCount: 800 },
      },
      history: [{ date: '2026-03-30', totalDownloads: 2500 }],
    };
    await env.PATCHES_KV.put(
      DOWNLOAD_STATISTICS_KV_KEY,
      JSON.stringify(testStats),
    );

    const req = new Request('http://localhost/charts/release-downloads.svg', {
      method: 'GET',
    });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toBe(
      'image/svg+xml; charset=utf-8',
    );
    expect(res.headers.get('Cache-Control')).toBe(
      'public, max-age=3600, s-maxage=3600',
    );

    const body = await res.text();
    expect(body).toContain('<svg');
    expect(body).toContain('Release Downloads');
    expect(body).toContain('v1.6');
    expect(body).toContain('v1.5');
    expect(body).toContain('1,700'); // 1500 + 200 for v1.6
    expect(body).toContain('800'); // v1.5
    expect(body).toContain('#160d2b');
    expect(body).not.toContain('NaN');
    expect(body).not.toContain('Infinity');
  });

  it('performs synchronous refresh when KV is empty and returns 200', async () => {
    await env.PATCHES_KV.delete(DOWNLOAD_STATISTICS_KV_KEY);

    globalThis.fetch = vi.fn(async () => {
      return new Response(
        JSON.stringify([
          {
            draft: false,
            tag_name: 'v1.6.0',
            assets: [{ id: 9991, download_count: 350 }],
          },
        ]),
        { status: 200 },
      );
    });

    const req = new Request('http://localhost/charts/total-downloads.svg', {
      method: 'GET',
    });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    const body = await res.text();
    expect(body).toContain('Total Downloads');
    expect(body).toContain('350');
  });

  it('returns 503 error SVG with no-store when KV is empty and GitHub refresh fails', async () => {
    await env.PATCHES_KV.delete(DOWNLOAD_STATISTICS_KV_KEY);

    globalThis.fetch = vi.fn(async () => {
      return new Response('GitHub Down', { status: 503 });
    });

    const req = new Request('http://localhost/charts/total-downloads.svg', {
      method: 'GET',
    });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(503);
    expect(res.headers.get('Content-Type')).toBe(
      'image/svg+xml; charset=utf-8',
    );
    expect(res.headers.get('Cache-Control')).toBe('no-store');

    const body = await res.text();
    expect(body).toContain('<svg');
    expect(body).toContain('Total Downloads');
    expect(body).toContain('#160d2b');
    expect(body).toContain('unavailable');
  });
});
