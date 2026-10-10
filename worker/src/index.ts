import type {
  AppVersionRecord,
  CompatibilityRecord,
  CompatibilityResultSubmission,
  CompatibilityRunStart,
  FailureStage,
  FreshnessStatus,
  KVVersionPayload,
  LatestReleaseSummary,
  OutstandingCompatibilityRequest,
  PatchCompatibilityStatus,
  WorkerEnv,
} from './types';
import { fetchGooglePlayApp } from './scraper';
import { compareAppVersions } from './comparator';
import { MONITORED_APPS } from './apps';
import { getStatusColor, renderBadgeSvg } from './badges';

let memoryCache: { payload: KVVersionPayload; expiresAt: number } | null = null;
const MEMORY_CACHE_TTL_MS = 60 * 60 * 1000; // 1 hour fallback
export const OUTSTANDING_REQUEST_TTL_MS = 36 * 60 * 60 * 1000; // 36 hours TTL (exceeds max 24h queue + 6h execution)

const PUBLIC_CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
};

export function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let c = 0;
  for (let i = 0; i < a.length; i++) {
    c |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return c === 0;
}

export function appRecordKey(packageName: string): string {
  return `app_version:${packageName}`;
}

export async function loadAppRecord(
  env: WorkerEnv,
  packageName: string,
): Promise<AppVersionRecord | null> {
  if (!env.PLAY_VERSIONS_KV) return null;
  const raw = await env.PLAY_VERSIONS_KV.get(appRecordKey(packageName));
  if (!raw) return null;
  try {
    return JSON.parse(raw) as AppVersionRecord;
  } catch {
    return null;
  }
}

export async function saveAppRecord(
  env: WorkerEnv,
  packageName: string,
  record: AppVersionRecord,
): Promise<void> {
  if (env.PLAY_VERSIONS_KV) {
    await env.PLAY_VERSIONS_KV.put(
      appRecordKey(packageName),
      JSON.stringify(record),
    );
  }
  memoryCache = null;
}

export async function loadStatusPayload(
  env: WorkerEnv,
  forceRefresh = false,
): Promise<KVVersionPayload> {
  if (!forceRefresh && memoryCache && Date.now() < memoryCache.expiresAt) {
    return memoryCache.payload;
  }

  const records = await Promise.all(
    MONITORED_APPS.map(async (app) => {
      const rec = await loadAppRecord(env, app.packageName);
      return { app, rec };
    }),
  );

  const appsRecord: Record<string, AppVersionRecord> = {};
  let latestRelease: LatestReleaseSummary | null = null;
  let latestTime = 0;
  let latestCheckedAt = '';

  for (const { app, rec } of records) {
    if (rec) {
      appsRecord[app.packageName] = {
        ...rec,
        outstandingRequestId:
          rec.outstandingRequest?.requestId ?? rec.outstandingRequestId ?? null,
      };
      if (rec.checkedAt && rec.checkedAt > latestCheckedAt) {
        latestCheckedAt = rec.checkedAt;
      }
      if (rec.updatedAt) {
        const t = new Date(rec.updatedAt).getTime();
        if (t > latestTime) {
          latestTime = t;
          latestRelease = {
            packageName: app.packageName,
            appName: app.name,
            playVersion: rec.playVersion,
            iconUrl: rec.iconUrl || null,
            updatedAt: rec.updatedAt,
            updatedOn: rec.updatedOn || null,
          };
        }
      }
    } else {
      appsRecord[app.packageName] = {
        appName: app.name,
        playVersion: null,
        iconUrl: null,
        updatedAt: null,
        updatedOn: null,
        checkedAt: new Date().toISOString(),
        status: 'unknown',
        supportedVersions: app.supportedVersions,
        latestSupportedVersion: app.latestSupportedVersion,
        targetCompatibility: null,
        outstandingRequest: null,
        outstandingRequestId: null,
      };
    }
  }

  if (records.every(({ rec }) => rec === null)) {
    return performVersionCheck(env);
  }

  const payload: KVVersionPayload = {
    updatedAt: latestCheckedAt || new Date().toISOString(),
    apps: appsRecord,
    latestRelease,
  };

  memoryCache = {
    payload,
    expiresAt: Date.now() + MEMORY_CACHE_TTL_MS,
  };

  return payload;
}

export async function performVersionCheck(
  env: WorkerEnv,
): Promise<KVVersionPayload> {
  const now = new Date().toISOString();
  const appsRecord: Record<string, AppVersionRecord> = {};

  for (const app of MONITORED_APPS) {
    const priorRecord = await loadAppRecord(env, app.packageName);
    const scraped = await fetchGooglePlayApp(app.packageName);

    let playVersion: string | null = null;
    let iconUrl = scraped.iconUrl || priorRecord?.iconUrl || null;
    let updatedAt = scraped.updatedAt || priorRecord?.updatedAt || null;
    let updatedOn = scraped.updatedOn || priorRecord?.updatedOn || null;
    let status: FreshnessStatus;
    let playVersionReleaseUpdatedAt: string | null =
      priorRecord?.playVersionReleaseUpdatedAt ?? null;

    if (scraped.isNotFound) {
      status = 'not-on-play-store';
      playVersion = null;
      playVersionReleaseUpdatedAt = null;
    } else if (scraped.rawError) {
      status = 'check-failed';
      playVersion = priorRecord?.playVersion ?? null;
    } else if (scraped.playVersion) {
      playVersion = scraped.playVersion;
      status = compareAppVersions(app.latestSupportedVersion, playVersion);
      playVersionReleaseUpdatedAt = scraped.updatedAt ?? null;
    } else {
      // App exists on Google Play (not 404, not error), but Google Play web HTML omits
      // the version string (e.g. multi-split App Bundle where version varies with device).
      if (priorRecord?.playVersion) {
        playVersion = priorRecord.playVersion;
        status = compareAppVersions(app.latestSupportedVersion, playVersion);
      } else {
        playVersion = null;
        status = 'unknown';
      }
    }
    let targetCompat = priorRecord?.targetCompatibility ?? null;

    const targetVersionChanged = Boolean(
      priorRecord &&
      priorRecord.latestSupportedVersion !== app.latestSupportedVersion,
    );

    if (targetVersionChanged) {
      targetCompat = null;
    }

    let outstandingRequest = priorRecord?.outstandingRequest ?? null;
    if (
      outstandingRequest &&
      outstandingRequest.targetVersion !== app.latestSupportedVersion
    ) {
      outstandingRequest = null;
    }

    if (outstandingRequest) {
      const dispatchedTime = new Date(
        outstandingRequest.dispatchedAt,
      ).getTime();
      const isStaleOrInvalid =
        Number.isNaN(dispatchedTime) ||
        Date.now() - dispatchedTime > OUTSTANDING_REQUEST_TTL_MS;
      if (isStaleOrInvalid) {
        outstandingRequest = null;
      }
    }

    const appRecord: AppVersionRecord = {
      appName: app.name,
      playVersion,
      iconUrl,
      updatedAt,
      updatedOn,
      checkedAt: now,
      status,
      supportedVersions: app.supportedVersions,
      latestSupportedVersion: app.latestSupportedVersion,
      playVersionReleaseUpdatedAt,
      targetCompatibility: targetCompat,
      outstandingRequest,
      outstandingRequestId: outstandingRequest?.requestId ?? null,
    };
    await saveAppRecord(env, app.packageName, appRecord);
    appsRecord[app.packageName] = appRecord;
  }

  let latestRelease: LatestReleaseSummary | null = null;
  let latestTime = 0;

  for (const app of MONITORED_APPS) {
    const record = appsRecord[app.packageName];
    if (record?.updatedAt) {
      const t = new Date(record.updatedAt).getTime();
      if (t > latestTime) {
        latestTime = t;
        latestRelease = {
          packageName: app.packageName,
          appName: app.name,
          playVersion: record.playVersion,
          iconUrl: record.iconUrl || null,
          updatedAt: record.updatedAt,
          updatedOn: record.updatedOn || null,
        };
      }
    }
  }

  const payload: KVVersionPayload = {
    updatedAt: now,
    apps: appsRecord,
    latestRelease,
  };

  memoryCache = {
    payload,
    expiresAt: Date.now() + MEMORY_CACHE_TTL_MS,
  };

  return payload;
}

export function computeAggregateCompatibilityStatus(
  payload: KVVersionPayload,
): { status: PatchCompatibilityStatus; label: string; color: string } {
  let anyError = false;
  let anyIncompatible = false;
  let anyPending = false;
  let allCompatible = true;
  let monitoredAppsFound = 0;

  for (const app of MONITORED_APPS) {
    const record = payload.apps[app.packageName];
    if (!record) {
      anyPending = true;
      allCompatible = false;
      continue;
    }

    monitoredAppsFound++;

    const targetStatus = record.targetCompatibility?.status || 'not-tested';

    if (targetStatus === 'error') {
      anyError = true;
      allCompatible = false;
    } else if (targetStatus === 'incompatible') {
      anyIncompatible = true;
      allCompatible = false;
    } else if (
      targetStatus === 'queued' ||
      targetStatus === 'running' ||
      targetStatus === 'not-tested'
    ) {
      anyPending = true;
      allCompatible = false;
    } else if (targetStatus !== 'compatible') {
      allCompatible = false;
    }
  }

  if (monitoredAppsFound < MONITORED_APPS.length) {
    anyPending = true;
    allCompatible = false;
  }

  let finalStatus: PatchCompatibilityStatus = 'compatible';
  let label = 'all compatible';
  let color = '#4c1';

  if (anyError) {
    finalStatus = 'error';
    label = 'check error';
    color = '#e05d44';
  } else if (anyIncompatible) {
    finalStatus = 'incompatible';
    label = 'incompatible';
    color = '#dfb317';
  } else if (anyPending) {
    finalStatus = 'queued';
    label = 'checks pending';
    color = '#9f9f9f';
  } else if (allCompatible) {
    finalStatus = 'compatible';
    label = 'all compatible';
    color = '#4c1';
  }

  return { status: finalStatus, label, color };
}
function isCompatibilityResultSubmission(
  val: unknown,
): val is CompatibilityResultSubmission {
  if (!val || typeof val !== 'object') return false;
  if (
    !('requestId' in val) ||
    typeof val.requestId !== 'string' ||
    !val.requestId.trim()
  )
    return false;
  if (!('packageName' in val) || typeof val.packageName !== 'string')
    return false;
  if ('acquiredPlayVersion' in val) {
    const apv = (val as Record<string, unknown>).acquiredPlayVersion;
    if (
      apv !== undefined &&
      apv !== null &&
      (typeof apv !== 'string' || !apv.trim())
    ) {
      return false;
    }
  }
  if (!('results' in val) || !Array.isArray(val.results)) return false;
  return true;
}

function isCompatibilityRunStart(val: unknown): val is CompatibilityRunStart {
  if (!val || typeof val !== 'object') return false;
  const obj = val as Record<string, unknown>;
  if (typeof obj.requestId !== 'string' || !obj.requestId.trim()) return false;
  if (typeof obj.packageName !== 'string' || !obj.packageName.trim())
    return false;
  if (typeof obj.appName !== 'string' || !obj.appName.trim()) return false;
  if (typeof obj.targetVersion !== 'string' || !obj.targetVersion.trim())
    return false;
  if (typeof obj.gitRevision !== 'string' || !obj.gitRevision.trim())
    return false;
  if (
    !Array.isArray(obj.supportedVersions) ||
    obj.supportedVersions.length === 0
  )
    return false;
  if (
    !obj.supportedVersions.every(
      (v): v is string => typeof v === 'string' && v.trim().length > 0,
    )
  )
    return false;
  if (!obj.supportedVersions.includes(obj.targetVersion)) return false;
  return true;
}

const FINAL_STATUSES: Record<string, true> = {
  compatible: true,
  incompatible: true,
  error: true,
};

const VALID_FAILURE_STAGES: Record<string, true> = {
  acquisition: true,
  'r2-lookup': true,
  'r2-upload': true,
  'r2-download': true,
  'compatibility-check': true,
  pipeline: true,
};

export default {
  async scheduled(
    event: ScheduledEvent,
    env: WorkerEnv,
    ctx: ExecutionContext,
  ): Promise<void> {
    ctx.waitUntil(performVersionCheck(env));
  },

  async fetch(request: Request, env: WorkerEnv): Promise<Response> {
    const url = new URL(request.url);

    if (request.method === 'OPTIONS') {
      return new Response(null, {
        status: 204,
        headers: PUBLIC_CORS_HEADERS,
      });
    }

    // 1. Authenticated compatibility run start registration
    if (
      url.pathname === '/api/compatibility-runs' &&
      request.method === 'POST'
    ) {
      const authHeader = request.headers.get('Authorization') || '';
      const token = authHeader.startsWith('Bearer ')
        ? authHeader.slice(7).trim()
        : '';

      if (
        !env.COMPATIBILITY_STATUS_SECRET ||
        !token ||
        !timingSafeEqual(token, env.COMPATIBILITY_STATUS_SECRET)
      ) {
        return new Response(JSON.stringify({ error: 'Unauthorized' }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      let rawBody: unknown;
      try {
        rawBody = await request.json();
      } catch {
        return new Response(JSON.stringify({ error: 'Malformed JSON' }), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      if (!isCompatibilityRunStart(rawBody)) {
        return new Response(
          JSON.stringify({ error: 'Invalid run-start submission format' }),
          {
            status: 400,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      const body = rawBody as CompatibilityRunStart;
      const prior = await loadAppRecord(env, body.packageName);
      const now = new Date().toISOString();

      const outstandingRequest: OutstandingCompatibilityRequest = {
        requestId: body.requestId,
        targetVersion: body.targetVersion,
        playVersion: prior?.playVersion ?? null,
        expectedRoles: ['target'],
        dispatchedAt: now,
        gitRevision: body.gitRevision,
      };

      const updatedRecord: AppVersionRecord = {
        appName: body.appName,
        playVersion: prior?.playVersion ?? null,
        iconUrl: prior?.iconUrl ?? null,
        updatedAt: prior?.updatedAt ?? null,
        updatedOn: prior?.updatedOn ?? null,
        checkedAt: prior?.checkedAt ?? now,
        status: prior?.status ?? 'unknown',
        supportedVersions: body.supportedVersions,
        latestSupportedVersion: body.targetVersion,
        playVersionReleaseUpdatedAt: prior?.playVersionReleaseUpdatedAt ?? null,
        targetCompatibility:
          prior && prior.latestSupportedVersion === body.targetVersion
            ? (prior.targetCompatibility ?? null)
            : null,
        outstandingRequest,
        outstandingRequestId: body.requestId,
      };

      await saveAppRecord(env, body.packageName, updatedRecord);

      return new Response(
        JSON.stringify({
          success: true,
          queued: body.packageName,
          requestId: body.requestId,
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      );
    }

    // 2. Authenticated compatibility test result ingestion
    if (
      url.pathname === '/api/compatibility-results' &&
      request.method === 'POST'
    ) {
      const authHeader = request.headers.get('Authorization') || '';
      const token = authHeader.startsWith('Bearer ')
        ? authHeader.slice(7).trim()
        : '';

      if (
        !env.COMPATIBILITY_STATUS_SECRET ||
        !token ||
        !timingSafeEqual(token, env.COMPATIBILITY_STATUS_SECRET)
      ) {
        return new Response(JSON.stringify({ error: 'Unauthorized' }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      let rawBody: unknown;
      try {
        rawBody = await request.json();
      } catch {
        return new Response(JSON.stringify({ error: 'Malformed JSON' }), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      if (!isCompatibilityResultSubmission(rawBody)) {
        return new Response(
          JSON.stringify({ error: 'Invalid submission format' }),
          {
            status: 400,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      const submission = rawBody;
      const appRecord = await loadAppRecord(env, submission.packageName);
      if (!appRecord) {
        return new Response(
          JSON.stringify({ error: 'App record not initialized' }),
          {
            status: 400,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      const outstanding = appRecord.outstandingRequest;
      if (!outstanding || outstanding.requestId !== submission.requestId) {
        return new Response(
          JSON.stringify({
            error: 'Stale, replayed, or mismatched request ID',
            outstanding: outstanding?.requestId ?? null,
          }),
          {
            status: 409,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      const expectedRoles = outstanding.expectedRoles;
      if (submission.results.length !== expectedRoles.length) {
        return new Response(
          JSON.stringify({
            error: `Expected ${expectedRoles.length} results for roles [${expectedRoles.join(', ')}], got ${submission.results.length}`,
          }),
          {
            status: 400,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      const seenRoles = new Set<string>();
      const recordsToApply: CompatibilityRecord[] = [];

      for (const res of submission.results) {
        if (!res || typeof res !== 'object') {
          return new Response(
            JSON.stringify({ error: 'Invalid result item' }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (res.role !== 'target' && res.role !== 'latest') {
          return new Response(
            JSON.stringify({ error: `Invalid role: ${res.role}` }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (!expectedRoles.includes(res.role)) {
          return new Response(
            JSON.stringify({ error: `Unexpected role: ${res.role}` }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (seenRoles.has(res.role)) {
          return new Response(
            JSON.stringify({ error: `Duplicate role in results: ${res.role}` }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }
        seenRoles.add(res.role);

        if (!FINAL_STATUSES[res.status]) {
          return new Response(
            JSON.stringify({ error: `Invalid status: ${res.status}` }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (
          !Number.isInteger(res.versionCode) ||
          res.versionCode < 0 ||
          !Number.isInteger(res.passedCount) ||
          res.passedCount < 0 ||
          !Number.isInteger(res.failedCount) ||
          res.failedCount < 0 ||
          typeof res.versionName !== 'string' ||
          !res.versionName.trim() ||
          typeof res.patchBundleVersion !== 'string' ||
          typeof res.gitRevision !== 'string'
        ) {
          return new Response(
            JSON.stringify({ error: 'Malformed result fields' }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (
          outstanding.gitRevision &&
          res.gitRevision !== outstanding.gitRevision
        ) {
          return new Response(
            JSON.stringify({
              error: `Result git revision '${res.gitRevision}' does not match outstanding '${outstanding.gitRevision}'`,
            }),
            {
              status: 409,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (
          res.failureStage !== undefined &&
          res.failureStage !== null &&
          (typeof res.failureStage !== 'string' ||
            !VALID_FAILURE_STAGES[res.failureStage])
        ) {
          return new Response(
            JSON.stringify({ error: 'Malformed failureStage field' }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (
          res.failureReason !== undefined &&
          res.failureReason !== null &&
          typeof res.failureReason !== 'string'
        ) {
          return new Response(
            JSON.stringify({ error: 'Malformed failureReason field' }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (
          res.role === 'target' &&
          res.versionName !== outstanding.targetVersion
        ) {
          return new Response(
            JSON.stringify({
              error: `Tested target version '${res.versionName}' does not match expected '${outstanding.targetVersion}'`,
            }),
            {
              status: 409,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        recordsToApply.push({
          requestId: submission.requestId,
          role: res.role,
          versionName: res.versionName,
          versionCode: res.versionCode,
          patchBundleVersion: res.patchBundleVersion,
          gitRevision: res.gitRevision,
          testedAt: new Date().toISOString(),
          passedCount: res.passedCount,
          failedCount: res.failedCount,
          status: res.status,
          workflowRunUrl: res.workflowRunUrl || null,
          failureReason:
            typeof res.failureReason === 'string' ? res.failureReason : null,
          failureStage:
            typeof res.failureStage === 'string'
              ? (res.failureStage as FailureStage)
              : null,
        });
      }
      for (const rec of recordsToApply) {
        appRecord.targetCompatibility = rec;
      }

      if (submission.acquiredPlayVersion) {
        const acquired = submission.acquiredPlayVersion.trim();
        appRecord.playVersion = acquired;
        appRecord.status = compareAppVersions(
          appRecord.latestSupportedVersion,
          acquired,
        );
        appRecord.playVersionReleaseUpdatedAt = appRecord.updatedAt ?? null;
      }

      appRecord.outstandingRequest = null;
      appRecord.outstandingRequestId = null;

      await saveAppRecord(env, submission.packageName, appRecord);
      return new Response(
        JSON.stringify({
          success: true,
          updated: submission.packageName,
          roles: recordsToApply.map((r) => r.role),
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      );
    }

    // 2. On-demand refresh endpoint
    if (url.pathname === '/api/refresh' && request.method === 'POST') {
      const authHeader = request.headers.get('Authorization') || '';
      const token = authHeader.startsWith('Bearer ')
        ? authHeader.slice(7).trim()
        : '';

      if (
        !env.REFRESH_SECRET ||
        !token ||
        !timingSafeEqual(token, env.REFRESH_SECRET)
      ) {
        return new Response(JSON.stringify({ error: 'Unauthorized' }), {
          status: 401,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      const refreshed = await performVersionCheck(env);
      return new Response(JSON.stringify(refreshed), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // 3. Aggregate compatibility badge
    if (url.pathname === '/badges/compatibility.svg') {
      const payload = await loadStatusPayload(env);
      const agg = computeAggregateCompatibilityStatus(payload);
      const svg = renderBadgeSvg('compatibility', agg.label, agg.color);

      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=300',
        },
      });
    }

    // 4. Per-package compatibility badge: /badges/compatibility/:packageName.svg
    if (url.pathname.startsWith('/badges/compatibility/')) {
      const rawPkg = url.pathname
        .replace('/badges/compatibility/', '')
        .replace(/\.svg$/, '');
      const packageName = decodeURIComponent(rawPkg);

      const appConfig = MONITORED_APPS.find(
        (a) => a.packageName === packageName,
      );
      if (!appConfig) {
        const notFoundSvg = renderBadgeSvg(
          'compatibility',
          'app not found',
          '#999',
        );
        return new Response(notFoundSvg, {
          status: 404,
          headers: {
            'Content-Type': 'image/svg+xml; charset=utf-8',
            'Cache-Control': 'public, max-age=300',
          },
        });
      }

      const record = await loadAppRecord(env, packageName);
      const targetStatus = record?.targetCompatibility?.status || 'not-tested';
      const stage = record?.targetCompatibility?.failureStage;
      const reasonLower = (
        record?.targetCompatibility?.failureReason || ''
      ).toLowerCase();
      const isNoApk =
        targetStatus === 'error' &&
        (stage === 'acquisition' ||
          (stage == null &&
            (reasonLower.includes('no fixture slot') ||
              reasonLower.includes('no apk') ||
              reasonLower.includes('missing fixture') ||
              ((record?.targetCompatibility?.passedCount ?? 0) === 0 &&
                (record?.targetCompatibility?.failedCount ?? 0) === 0) ||
              ((record?.targetCompatibility?.passedCount ?? 0) === 0 &&
                (record?.targetCompatibility?.failedCount ?? 0) <= 1 &&
                reasonLower.includes('fixture')))));

      const badgeStatus = isNoApk ? 'no apk' : targetStatus;
      const label = `${appConfig.name} compatibility`;
      const color = isNoApk ? '#EF4444' : getStatusColor(targetStatus);
      const svg = renderBadgeSvg(label, badgeStatus, color);
      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=300',
        },
      });
    }

    // 5. Public Status endpoint (GET / or GET /api/status)
    if (
      url.pathname === '/api/status' ||
      url.pathname === '/' ||
      url.pathname === '/status'
    ) {
      const payload = await loadStatusPayload(env);
      return new Response(JSON.stringify(payload), {
        status: 200,
        headers: {
          ...PUBLIC_CORS_HEADERS,
          'Content-Type': 'application/json',
          'Cache-Control': 'public, max-age=300, s-maxage=3600',
        },
      });
    }

    return new Response(JSON.stringify({ error: 'Not Found' }), {
      status: 404,
      headers: {
        ...PUBLIC_CORS_HEADERS,
        'Content-Type': 'application/json',
      },
    });
  },
};
