import { env } from 'cloudflare:test';
import { describe, expect, it, vi } from 'vitest';
import { deriveMonitoredApps } from '../src/apps';
import { compareAppVersions } from '../src/comparator';
import { renderBadgeSvg } from '../src/badges';
import worker, {
  computeAggregateCompatibilityStatus,
  timingSafeEqual,
  performVersionCheck,
} from '../src/index';
import type {
  AppVersionRecord,
  KVVersionPayload,
  WorkerEnv,
} from '../src/types';

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

    expect(() => deriveMonitoredApps(conflictingData)).toThrow(
      /Conflicting app name/,
    );
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

    expect(() => deriveMonitoredApps(conflictingData)).toThrow(
      /Conflicting apkFileType/,
    );
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
    for (const _app of deriveMonitoredApps({ patches: [] })) {
      // empty fallback
    }
    for (const app of [
      { packageName: 'com.ashtoncofer.Buzz', name: 'Fizz', latest: '1.54.0' },
      {
        packageName: 'com.aftership.AfterShip',
        name: 'AfterShip',
        latest: '5.25.8',
      },
      {
        packageName: 'com.adobe.scan.android',
        name: 'Adobe Scan',
        latest: '26.09.25',
      },
      { packageName: 'com.eab.se', name: 'Navigate360', latest: '26.19.22' },
      {
        packageName: 'com.instructure.candroid',
        name: 'Canvas Student',
        latest: '8.10.0',
      },
      {
        packageName: 'com.sezzle.sezzlemobile',
        name: 'Sezzle',
        latest: '5.3.9',
      },
      {
        packageName: 'com.sidelineswap.android',
        name: 'SidelineSwap',
        latest: '1.52.0',
      },
      {
        packageName: 'com.tripledot.blackjack',
        name: 'Blackjack',
        latest: '2.22.09',
      },
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

    const req = new Request('http://localhost/badges/compatibility.svg', {
      method: 'GET',
    });
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

    const req = new Request(
      'http://localhost/badges/compatibility/com.ashtoncofer.Buzz.svg',
      { method: 'GET' },
    );
    const res = await worker.fetch(req, workerEnv);

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toContain('image/svg+xml');
    const body = await res.text();
    expect(body).toContain('Fizz compatibility');
  });
  it.each([
    [undefined, 0, 0, '', 'no apk'],
    [null, 2, 3, 'No fixture slot matches target', 'no apk'],
    [undefined, 0, 1, 'Failed fixture retrieval', 'no apk'],
    ['acquisition', 2, 3, '', 'no apk'],
    ...[
      'r2-lookup',
      'r2-upload',
      'r2-download',
      'compatibility-check',
      'pipeline',
    ].flatMap((stage) => [
      [stage, 0, 0, '', 'error'],
      [stage, 0, 1, 'Missing fixture: no APK', 'error'],
    ]),
  ])(
    'renders failure stage %s with counts %s/%s and reason %s as %s',
    async (
      failureStage,
      passedCount,
      failedCount,
      failureReason,
      expectedStatus,
    ) => {
      const workerEnv: WorkerEnv = {
        ...env,
        PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      };

      await seedTestApps(env.PLAY_VERSIONS_KV);
      const appRecord = {
        appName: 'Sezzle',
        playVersion: '5.3.13',
        checkedAt: new Date().toISOString(),
        status: 'newer-available' as const,
        supportedVersions: ['5.3.9'],
        latestSupportedVersion: '5.3.9',
        targetCompatibility: {
          requestId: 'req-test',
          role: 'target' as const,
          versionName: '5.3.9',
          versionCode: 0,
          patchBundleVersion: '1.5.1',
          gitRevision: 'rev1',
          testedAt: new Date().toISOString(),
          passedCount,
          failedCount,
          status: 'error' as const,
          failureStage,
          failureReason,
        },
      };
      await env.PLAY_VERSIONS_KV.put(
        'app_version:com.sezzle.sezzlemobile',
        JSON.stringify(appRecord),
      );

      const req = new Request(
        'http://localhost/badges/compatibility/com.sezzle.sezzlemobile.svg',
        { method: 'GET' },
      );
      const res = await worker.fetch(req, workerEnv);

      expect(res.status).toBe(200);
      const body = await res.text();
      expect(body).toContain('Sezzle compatibility');
      expect(body).toContain(expectedStatus as string);
      if (expectedStatus === 'error') expect(body).not.toContain('no apk');
      expect(body).toContain('#EF4444');
    },
  );

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
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.ashtoncofer.Buzz',
      JSON.stringify(initialRecord),
    );

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
    const savedRaw = await env.PLAY_VERSIONS_KV.get(
      'app_version:com.ashtoncofer.Buzz',
    );
    expect(savedRaw).not.toBeNull();
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.targetCompatibility?.status).toBe('compatible');
    expect(saved.targetCompatibility?.workflowRunUrl).toBe(
      'https://github.com/test/runs/1',
    );
    expect(saved.outstandingRequest).toBeNull();
  });

  it('accepts acquiredPlayVersion in callback and updates playVersion, status, and release marker', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Adobe Scan: PDF Scanner, OCR',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'unknown',
      supportedVersions: ['26.09.25'],
      latestSupportedVersion: '26.09.25',
      playVersion: null,
      updatedAt: '2026-10-05T00:00:00Z',
      playVersionReleaseUpdatedAt: null,
      outstandingRequest: {
        requestId: 'req-adobe-scan',
        targetVersion: '26.09.25',
        playVersion: null,
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.adobe.scan.android',
      JSON.stringify(initialRecord),
    );

    const submission = {
      requestId: 'req-adobe-scan',
      packageName: 'com.adobe.scan.android',
      acquiredPlayVersion: '26.09.25',
      results: [
        {
          role: 'target',
          versionName: '26.09.25',
          versionCode: 260925,
          patchBundleVersion: '1.4.0',
          gitRevision: 'git123',
          status: 'compatible',
          passedCount: 8,
          failedCount: 0,
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

    const savedRaw = await env.PLAY_VERSIONS_KV.get(
      'app_version:com.adobe.scan.android',
    );
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.playVersion).toBe('26.09.25');
    expect(saved.status).toBe('up-to-date');
    expect(saved.playVersionReleaseUpdatedAt).toBe('2026-10-05T00:00:00Z');
    expect(saved.targetCompatibility?.status).toBe('compatible');
    expect(saved.outstandingRequest).toBeNull();
  });

  it('rejects invalid non-string or empty acquiredPlayVersion in callback', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'req-invalid-apv',
        packageName: 'com.adobe.scan.android',
        acquiredPlayVersion: '   ',
        results: [],
      }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(400);
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
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.ashtoncofer.Buzz',
      JSON.stringify(initialRecord),
    );

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
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.ashtoncofer.Buzz',
      JSON.stringify(initialRecord),
    );

    const mismatchReq = new Request(
      'http://localhost/api/compatibility-results',
      {
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
      },
    );

    const res = await worker.fetch(mismatchReq, workerEnv);
    expect(res.status).toBe(409);
  });

  it('accepts and persists failureReason in compatibility results', async () => {
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
        requestId: 'error-req-uuid',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.ashtoncofer.Buzz',
      JSON.stringify(initialRecord),
    );

    const submission = {
      requestId: 'error-req-uuid',
      packageName: 'com.ashtoncofer.Buzz',
      results: [
        {
          role: 'target',
          versionName: '1.54.0',
          versionCode: 0,
          patchBundleVersion: '1.4.0',
          gitRevision: 'git123',
          status: 'error',
          passedCount: 0,
          failedCount: 1,
          failureReason: 'Missing target fixture in R2 slot',
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

    const savedRaw = await env.PLAY_VERSIONS_KV.get(
      'app_version:com.ashtoncofer.Buzz',
    );
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.targetCompatibility?.status).toBe('error');
    expect(saved.targetCompatibility?.failureReason).toBe(
      'Missing target fixture in R2 slot',
    );
  });

  it('rejects malformed non-string failureReason in compatibility results', async () => {
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
        requestId: 'bad-reason-req',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      'app_version:com.ashtoncofer.Buzz',
      JSON.stringify(initialRecord),
    );

    const submission = {
      requestId: 'bad-reason-req',
      packageName: 'com.ashtoncofer.Buzz',
      results: [
        {
          role: 'target',
          versionName: '1.54.0',
          versionCode: 0,
          patchBundleVersion: '1.4.0',
          gitRevision: 'git123',
          status: 'error',
          passedCount: 0,
          failedCount: 1,
          failureReason: 12345, // invalid!
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
    expect(res.status).toBe(400);
  });
});

describe('Version check and self-healing dispatches', () => {
  const MOCK_PLAY_HTML = (version: string) => `
    <html>
      <body>
        [[["${version}"]],[[[1]],[[[1]]]
        <div>Updated on</div><div>Oct 6, 2026</div>
      </body>
    </html>
  `;

  it('rejects unauthenticated POST /api/compatibility-runs', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const req = new Request('http://localhost/api/compatibility-runs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ packageName: 'com.ashtoncofer.Buzz' }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(401);
  });

  it('rejects malformed POST /api/compatibility-runs payload', async () => {
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    // Missing targetVersion in supportedVersions
    const req = new Request('http://localhost/api/compatibility-runs', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'req-1',
        packageName: 'com.ashtoncofer.Buzz',
        appName: 'Fizz',
        targetVersion: '2.0.0',
        supportedVersions: ['1.0.0'],
        gitRevision: 'sha123',
      }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(400);
  });

  it('registers run-start for known app and preserves existing Play metadata', async () => {
    const pkg = 'com.ashtoncofer.Buzz';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      playVersion: '1.54.0',
      iconUrl: 'https://example.com/icon.png',
      updatedAt: '2026-10-06T00:00:00Z',
      updatedOn: 'Oct 6, 2026',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      targetCompatibility: null,
      outstandingRequest: null,
    };
    await env.PLAY_VERSIONS_KV.put(
      `app_version:${pkg}`,
      JSON.stringify(initialRecord),
    );

    const runStartPayload = {
      requestId: 'run-start-uuid',
      packageName: pkg,
      appName: 'Fizz',
      targetVersion: '1.54.0',
      supportedVersions: ['1.54.0'],
      gitRevision: 'git_commit_sha_123',
    };

    const req = new Request('http://localhost/api/compatibility-runs', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify(runStartPayload),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(200);
    const data = (await res.json()) as { success?: boolean; queued?: string };
    expect(data.success).toBe(true);
    expect(data.queued).toBe(pkg);

    const savedRaw = await env.PLAY_VERSIONS_KV.get(`app_version:${pkg}`);
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.playVersion).toBe('1.54.0');
    expect(saved.iconUrl).toBe('https://example.com/icon.png');
    expect(saved.outstandingRequest?.requestId).toBe('run-start-uuid');
    expect(saved.outstandingRequest?.gitRevision).toBe('git_commit_sha_123');
  });

  it('initializes unknown new app on POST /api/compatibility-runs and shows queued status', async () => {
    const pkg = 'com.new.awesomeapp';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const runStartPayload = {
      requestId: 'new-app-run-id',
      packageName: pkg,
      appName: 'Awesome App',
      targetVersion: '1.0.0',
      supportedVersions: ['1.0.0'],
      gitRevision: 'sha_new_app',
    };

    const req = new Request('http://localhost/api/compatibility-runs', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify(runStartPayload),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(200);

    const savedRaw = await env.PLAY_VERSIONS_KV.get(`app_version:${pkg}`);
    expect(savedRaw).not.toBeNull();
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.appName).toBe('Awesome App');
    expect(saved.latestSupportedVersion).toBe('1.0.0');
    expect(saved.outstandingRequest?.requestId).toBe('new-app-run-id');
    expect(saved.outstandingRequest?.gitRevision).toBe('sha_new_app');

    // Results can then be submitted for this new app
    const resultReq = new Request(
      'http://localhost/api/compatibility-results',
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: 'Bearer test_secret_abc',
        },
        body: JSON.stringify({
          requestId: 'new-app-run-id',
          packageName: pkg,
          results: [
            {
              role: 'target',
              versionName: '1.0.0',
              versionCode: 10,
              patchBundleVersion: '1.5.0',
              gitRevision: 'sha_new_app',
              status: 'compatible',
              passedCount: 3,
              failedCount: 0,
            },
          ],
        }),
      },
    );
    const resultRes = await worker.fetch(resultReq, workerEnv);
    expect(resultRes.status).toBe(200);

    const afterResultRaw = await env.PLAY_VERSIONS_KV.get(`app_version:${pkg}`);
    const afterResult = JSON.parse(afterResultRaw!) as AppVersionRecord;
    expect(afterResult.targetCompatibility?.status).toBe('compatible');
    expect(afterResult.outstandingRequest).toBeNull();
  });

  it('rejects POST /api/compatibility-results when gitRevision does not match run-start', async () => {
    const pkg = 'com.ashtoncofer.Buzz';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      playVersion: '1.54.0',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'req-git-rev-test',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
        gitRevision: 'correct_git_revision_123',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      `app_version:${pkg}`,
      JSON.stringify(initialRecord),
    );

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'req-git-rev-test',
        packageName: pkg,
        results: [
          {
            role: 'target',
            versionName: '1.54.0',
            versionCode: 100,
            patchBundleVersion: '1.4.0',
            gitRevision: 'wrong_git_revision_456',
            status: 'compatible',
            passedCount: 5,
            failedCount: 0,
          },
        ],
      }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(409);
    const body = (await res.json()) as { error?: string };
    expect(body.error).toContain('Result git revision');
  });

  it('persists failureStage on error result and serves in public status payload', async () => {
    const pkg = 'com.ashtoncofer.Buzz';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      playVersion: '1.54.0',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'req-fail-stage',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
        gitRevision: 'rev123',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      `app_version:${pkg}`,
      JSON.stringify(initialRecord),
    );

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'req-fail-stage',
        packageName: pkg,
        results: [
          {
            role: 'target',
            versionName: '1.54.0',
            versionCode: 0,
            patchBundleVersion: '1.4.0',
            gitRevision: 'rev123',
            status: 'error',
            passedCount: 0,
            failedCount: 1,
            failureStage: 'acquisition',
            failureReason: 'Artifact not available on APKMirror or Play',
          },
        ],
      }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(200);

    const savedRaw = await env.PLAY_VERSIONS_KV.get(`app_version:${pkg}`);
    const saved = JSON.parse(savedRaw!) as AppVersionRecord;
    expect(saved.targetCompatibility?.failureStage).toBe('acquisition');

    // Public status endpoint returns the failureStage
    const statusReq = new Request('http://localhost/api/status', {
      method: 'GET',
    });
    const statusRes = await worker.fetch(statusReq, workerEnv);
    expect(statusRes.status).toBe(200);
    const statusPayload = (await statusRes.json()) as KVVersionPayload;
    expect(statusPayload.apps[pkg].targetCompatibility?.failureStage).toBe(
      'acquisition',
    );
  });

  it('rejects invalid failureStage in result submission', async () => {
    const pkg = 'com.ashtoncofer.Buzz';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
      COMPATIBILITY_STATUS_SECRET: 'test_secret_abc',
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      playVersion: '1.54.0',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      outstandingRequest: {
        requestId: 'req-bad-stage',
        targetVersion: '1.54.0',
        playVersion: '1.54.0',
        expectedRoles: ['target'],
        dispatchedAt: '2026-10-06T00:00:00Z',
        gitRevision: 'rev123',
      },
    };
    await env.PLAY_VERSIONS_KV.put(
      `app_version:${pkg}`,
      JSON.stringify(initialRecord),
    );

    const req = new Request('http://localhost/api/compatibility-results', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer test_secret_abc',
      },
      body: JSON.stringify({
        requestId: 'req-bad-stage',
        packageName: pkg,
        results: [
          {
            role: 'target',
            versionName: '1.54.0',
            versionCode: 0,
            patchBundleVersion: '1.4.0',
            gitRevision: 'rev123',
            status: 'error',
            passedCount: 0,
            failedCount: 1,
            failureStage: 'invalid-stage-name',
          },
        ],
      }),
    });

    const res = await worker.fetch(req, workerEnv);
    expect(res.status).toBe(400);
    const data = (await res.json()) as { error?: string };
    expect(data.error).toContain('failureStage');
  });

  it('scheduled performVersionCheck updates Google Play freshness only without making any GitHub dispatch calls', async () => {
    const pkg = 'com.ashtoncofer.Buzz';
    const workerEnv: WorkerEnv = {
      ...env,
      PLAY_VERSIONS_KV: env.PLAY_VERSIONS_KV,
    };

    const initialRecord: AppVersionRecord = {
      appName: 'Fizz',
      playVersion: '1.53.0',
      checkedAt: '2026-10-06T00:00:00Z',
      status: 'up-to-date',
      supportedVersions: ['1.54.0'],
      latestSupportedVersion: '1.54.0',
      targetCompatibility: {
        requestId: 'req-prior',
        role: 'target',
        versionName: '1.54.0',
        versionCode: 100,
        patchBundleVersion: '1.4.0',
        gitRevision: 'rev1',
        testedAt: '2026-10-06T00:00:00Z',
        passedCount: 5,
        failedCount: 0,
        status: 'compatible',
      },
      outstandingRequest: null,
    };
    await env.PLAY_VERSIONS_KV.put(
      `app_version:${pkg}`,
      JSON.stringify(initialRecord),
    );

    let githubCalled = false;
    const originalFetch = globalThis.fetch;
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url.includes('api.github.com')) {
        githubCalled = true;
        throw new Error(
          'GitHub API should never be called during scheduled check',
        );
      }
      if (url.includes('play.google.com')) {
        return new Response(MOCK_PLAY_HTML('1.55.0'), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    try {
      await performVersionCheck(workerEnv);

      expect(githubCalled).toBe(false);

      const savedRaw = await env.PLAY_VERSIONS_KV.get(`app_version:${pkg}`);
      const saved = JSON.parse(savedRaw!) as AppVersionRecord;
      expect(saved.playVersion).toBe('1.55.0');
      expect(saved.status).toBe('newer-available');
      // Target compatibility result preserved from prior check
      expect(saved.targetCompatibility?.status).toBe('compatible');
      // No outstanding request created
      expect(saved.outstandingRequest).toBeNull();
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
