import { env } from 'cloudflare:test';
import { describe, expect, it } from 'vitest';
import { deriveMonitoredApps } from '../src/apps';
import { compareAppVersions } from '../src/comparator';
import { renderBadgeSvg, getStatusColor } from '../src/badges';
import worker, {
  computeAggregateCompatibilityStatus,
  timingSafeEqual,
} from '../src/index';
import type { KVVersionPayload, WorkerEnv } from '../src/types';

describe('Apps metadata derivation', () => {
  it('derives unique package metadata and latest target correctly', () => {
    const mockData = {
      version: '1.0.0',
      patches: [
        {
          name: 'Patch 1',
          compatiblePackages: [
            {
              packageName: 'com.test.one',
              name: 'App One',
              apkFileType: 'APKM',
              signatures: ['sig1'],
              targets: [{ version: '1.0.0' }, { version: '1.2.0' }],
            },
          ],
        },
        {
          name: 'Patch 2',
          compatiblePackages: [
            {
              packageName: 'com.test.one',
              name: 'App One',
              apkFileType: 'APKM',
              signatures: ['sig1'],
              targets: [{ version: '1.1.0' }],
            },
          ],
        },
      ],
    };

    const apps = deriveMonitoredApps(mockData);
    expect(apps).toHaveLength(1);
    expect(apps[0].packageName).toBe('com.test.one');
    expect(apps[0].name).toBe('App One');
    expect(apps[0].apkFileType).toBe('APKM');
    expect(apps[0].latestSupportedVersion).toBe('1.2.0');
    expect(apps[0].supportedVersions).toEqual(['1.0.0', '1.1.0', '1.2.0']);
    expect(apps[0].patchNames).toEqual(['Patch 1', 'Patch 2']);
  });

  it('rejects conflicting app names for the same package', () => {
    const conflictingData = {
      version: '1.0.0',
      patches: [
        {
          name: 'Patch 1',
          compatiblePackages: [
            {
              packageName: 'com.test.conflict',
              name: 'App Original',
              apkFileType: 'APK',
              targets: [{ version: '1.0.0' }],
            },
          ],
        },
        {
          name: 'Patch 2',
          compatiblePackages: [
            {
              packageName: 'com.test.conflict',
              name: 'App Conflict Name',
              apkFileType: 'APK',
              targets: [{ version: '1.0.0' }],
            },
          ],
        },
      ],
    };

    expect(() => deriveMonitoredApps(conflictingData)).toThrow(/Conflicting app name/);
  });

  it('rejects conflicting file types for the same package', () => {
    const conflictingData = {
      version: '1.0.0',
      patches: [
        {
          name: 'Patch 1',
          compatiblePackages: [
            {
              packageName: 'com.test.filetype',
              name: 'App Filetype',
              apkFileType: 'APK',
              targets: [{ version: '1.0.0' }],
            },
          ],
        },
        {
          name: 'Patch 2',
          compatiblePackages: [
            {
              packageName: 'com.test.filetype',
              name: 'App Filetype',
              apkFileType: 'XAPK',
              targets: [{ version: '1.0.0' }],
            },
          ],
        },
      ],
    };

    expect(() => deriveMonitoredApps(conflictingData)).toThrow(/Conflicting apkFileType/);
  });
});

describe('Comparator semantics', () => {
  it('returns unknown when play version is null or empty', () => {
    expect(compareAppVersions('1.54.0', null)).toBe('unknown');
    expect(compareAppVersions('1.54.0', '')).toBe('unknown');
  });

  it('returns unknown when play version indicates "varies with device"', () => {
    expect(compareAppVersions('1.54.0', 'Varies with device')).toBe('unknown');
    expect(compareAppVersions('1.54.0', 'varies')).toBe('unknown');
  });

  it('identifies newer versions correctly', () => {
    expect(compareAppVersions('1.53.0', '1.54.0')).toBe('newer-available');
    expect(compareAppVersions('1.0.0', '2.0.0')).toBe('newer-available');
  });

  it('identifies equal or older versions as up to date', () => {
    expect(compareAppVersions('1.54.0', '1.54.0')).toBe('up-to-date');
    expect(compareAppVersions('1.54.0', '1.53.0')).toBe('up-to-date');
  });
});

describe('Security and Timing helpers', () => {
  it('timingSafeEqual behaves correctly for matching and non-matching strings', () => {
    expect(timingSafeEqual('secret123', 'secret123')).toBe(true);
    expect(timingSafeEqual('secret123', 'secret124')).toBe(false);
    expect(timingSafeEqual('short', 'longer_string')).toBe(false);
  });
});

describe('Badge rendering and aggregate calculation', () => {
  it('renders valid SVG with escaped text', () => {
    const svg = renderBadgeSvg('test & label', '<status>', '#34D399');
    expect(svg).toContain('<svg');
    expect(svg).toContain('test &amp; label');
    expect(svg).toContain('&lt;status&gt;');
    expect(svg).toContain('#34D399');
  });

  it('computes aggregate status correctly', () => {
    const mockPayload: KVVersionPayload = {
      updatedAt: '2026-10-06T00:00:00Z',
      apps: {
        'com.ashtoncofer.Buzz': {
          checkedAt: '2026-10-06T00:00:00Z',
          status: 'up-to-date',
          supportedVersions: ['1.54.0'],
          latestSupportedVersion: '1.54.0',
          playVersion: '1.54.0',
          targetCompatibility: {
            requestId: 'req-1',
            role: 'target',
            versionName: '1.54.0',
            versionCode: 400032,
            patchBundleVersion: '1.4.0',
            gitRevision: 'rev123',
            testedAt: '2026-10-06T00:00:00Z',
            passedCount: 5,
            failedCount: 0,
            status: 'compatible',
          },
        },
      },
    };

    const agg = computeAggregateCompatibilityStatus(mockPayload);
    // Because not all 8 monitored apps are in this partial payload, it reports pending
    expect(agg.status).toBe('queued');
  });
});
describe('Worker fetch endpoints', () => {
  async function seedTestApps(kv: KVNamespace) {
    for (const app of deriveMonitoredApps({ patches: [] })) {
      // empty fallback
    }
    for (const app of [
      { packageName: 'com.ashtoncofer.Buzz', name: 'Fizz', latest: '1.54.0' },
      { packageName: 'com.aftership.AfterShip', name: 'AfterShip', latest: '5.25.8' },
      { packageName: 'com.adobe.scan.android', name: 'Adobe Scan', latest: '26.09.25' },
      { packageName: 'com.eab.se', name: 'Navigate360', latest: '26.19.22' },
      { packageName: 'com.instructure.candroid', name: 'Canvas Student', latest: '8.10.0' },
      { packageName: 'com.sezzle.sezzlemobile', name: 'Sezzle', latest: '5.3.9' },
      { packageName: 'com.sidelineswap.android', name: 'SidelineSwap', latest: '1.52.0' },
      { packageName: 'com.tripledot.blackjack', name: 'Blackjack', latest: '2.22.09' },
    ]) {
      const record: AppVersionRecord = {
        appName: app.name,
        playVersion: app.latest,
        iconUrl: null,
        updatedAt: '2026-10-06T00:00:00Z',
        updatedOn: 'Oct 6, 2026',
        checkedAt: '2026-10-06T00:00:00Z',
        status: 'up-to-date',
        supportedVersions: [app.latest],
        latestSupportedVersion: app.latest,
        targetCompatibility: {
          requestId: 'init-req',
          role: 'target',
          versionName: app.latest,
          versionCode: 100,
          patchBundleVersion: '1.4.0',
          gitRevision: 'rev1',
          testedAt: '2026-10-06T00:00:00Z',
          passedCount: 5,
          failedCount: 0,
          status: 'compatible',
        },
        latestCompatibility: null,
        outstandingRequest: null,
      };
      await kv.put(`app_version:${app.packageName}`, JSON.stringify(record));
    }
  }

  it('serves GET /badges/compatibility.svg with image/svg+xml from seeded KV', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
    };

    await seedTestApps(env.PLAY_VERSIONS_KV);

    const req = new Request('http://localhost/badges/compatibility.svg', { method: 'GET' });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toContain('image/svg+xml');
    expect(res.headers.get('Cache-Control')).toContain('max-age=300');
    const body = await res.text();
    expect(body).toContain('<svg');
    expect(body).toContain('compatibility');
  });

  it('serves GET /badges/compatibility/com.ashtoncofer.Buzz.svg for specific package', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
    };

    await seedTestApps(env.PLAY_VERSIONS_KV);

    const req = new Request('http://localhost/badges/compatibility/com.ashtoncofer.Buzz.svg', { method: 'GET' });
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toContain('image/svg+xml');
    const body = await res.text();
    expect(body).toContain('Fizz compatibility');
  });

  it('rejects unauthenticated POST /api/compatibility-results', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ packageName: 'com.ashtoncofer.Buzz' }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(401);
  });

  it('accepts authenticated batched POST /api/compatibility-results and updates per-package KV', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      playVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'req-test-uuid',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put('app_version:com.ashtoncofer.Buzz', JSON.stringify(initialRecord));

    const submission = {
      requestId: 'req-test-uuid',
      packageName: 'com.ashtoncofer.Buzz',
      results: [
        {
          role: 'target',
          versionName: '1.54.0',
          versionCode: 400032,
          patchBundleVersion: '1.4.0',
          gitRevision: 'git123',
          status: 'compatible',
          passedCount: 8,
          failedCount: 0,
          workflowRunUrl: 'https://github.com/test/runs/1',
        },
      ],
    };

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify(submission),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(200);
    const data = (await res.json()) as { success?: boolean };
    expect(data.success).toBe(true);

    // Verify individual per-package KV key was updated
    const savedRaw = await env.PLAY_VERSIONS_KV.get('app_version:com.ashtoncofer.Buzz');
    expect(savedRaw).not.toBeNull();
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.targetCompatibility?.status).toBe('compatible');
    expect(saved.targetCompatibility?.workflowRunUrl).toBe('https://github.com/test/runs/1');
    expect(saved.outstandingRequest).toBeNull();
  });

  it('returns 409 Conflict when request ID is stale or replayed', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      playVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'current-valid-id',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put('app_version:com.ashtoncofer.Buzz', JSON.stringify(initialRecord));

    // Post mismatched request ID
    const staleReq = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'old-stale-id',
        packageName: 'com.ashtoncofer.Buzz',
        results: [
          {
            role: 'target',
            versionName: '1.54.0',
            versionCode: 400032,
            patchBundleVersion: '1.4.0',
            gitRevision: 'git123',
            status: 'compatible',
            passedCount: 8,
            failedCount: 0,
          },
        ],
      }),
    });

    const res = await worker.fetch(staleReq, workerEnv);
    expect(res.status).toBe(409);
    const body = (await res.json()) as { error?: string };
    expect(body.error).toContain('Stale');
  });

  it('rejects callback with version mismatch against outstanding request', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      playVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'valid-req-123',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put('app_version:com.ashtoncofer.Buzz', JSON.stringify(initialRecord));

    const mismatchReq = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'valid-req-123',
        packageName: 'com.ashtoncofer.Buzz',
        results: [
          {
            role: 'target',
            versionName: '1.53.0', // Mismatch!
            versionCode: 400032,
            patchBundleVersion: '1.4.0',
            gitRevision: 'git123',
            status: 'compatible',
            passedCount: 8,
            failedCount: 0,
          },
        ],
      }),
    });

    const res = await worker.fetch(mismatchReq, workerEnv);
    expect(res.status).toBe(409);
  });
});
