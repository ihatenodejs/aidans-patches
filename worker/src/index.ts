import type {
  AppVersionRecord,
  CompatibilityResultSubmission,
  CompatibilityRunStart,
  KVVersionPayload,
  LatestReleaseSummary,
  PatchCompatibilityStatus,
  PlayMediaAsset,
  PlayMetadataResultItem,
  PlayMetadataSubmission,
  WorkerEnv,
} from './types';
import { fetchGooglePlayApp } from './scraper';
import { MONITORED_APPS } from './apps';
import { getStatusColor, renderBadgeSvg } from './badges';
import {
  deriveReleaseLineDownloads,
  loadDownloadStatistics,
  refreshDownloadStatistics,
} from './download-statistics';
import {
  renderErrorChartSvg,
  renderReleaseDownloadsChartSvg,
  renderTotalDownloadsChartSvg,
} from './statistics-charts';
import { mirrorAssetsBatch } from './media-mirror';
import { mergeScrapedRecord } from './app-record';
import {
  AUTHORITY_GATE_KEY,
  AUTHORITY_GATE_SETTLE_MS,
  AuthorityNotReadyError,
  applyScrape,
  readAppRecord,
  startCompatibilityRun,
  submitCompatibilityResult,
  updatePlayMetadata,
  writeAppRecord,
} from './app-state';

export { AppStateDurableObject, appRecordKey } from './app-state';
export {
  reconcileMediaAsset,
  reconcileMediaAssetsList,
  reconcileReviews,
} from './app-record';

const PUBLIC_CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
};

const APP_STATE_PATHS = [
  '/api/compatibility-runs',
  '/api/compatibility-results',
  '/api/refresh',
  '/api/play-metadata',
  '/api/version',
  '/api/status',
  '/status',
  '/',
];

function routeUsesAppState(pathname: string): boolean {
  return (
    APP_STATE_PATHS.includes(pathname) ||
    pathname.startsWith('/api/version/') ||
    pathname.startsWith('/badges/')
  );
}

async function authorityGateOpen(env: WorkerEnv): Promise<boolean> {
  const gate = env.PATCHES_KV
    ? await env.PATCHES_KV.get(AUTHORITY_GATE_KEY)
    : null;
  if (!gate) return false;
  const gateTime = Date.parse(gate);
  return (
    !Number.isNaN(gateTime) && Date.now() >= gateTime + AUTHORITY_GATE_SETTLE_MS
  );
}

export function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let c = 0;
  for (let i = 0; i < a.length; i++) {
    c |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return c === 0;
}

export async function loadAppRecord(
  env: WorkerEnv,
  packageName: string,
): Promise<AppVersionRecord | null> {
  return readAppRecord(env, packageName);
}

export async function saveAppRecord(
  env: WorkerEnv,
  packageName: string,
  record: AppVersionRecord,
): Promise<void> {
  await writeAppRecord(env, packageName, record);
}

export async function loadStatusPayload(
  env: WorkerEnv,
): Promise<KVVersionPayload> {
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
        playMetadata: null,
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

  return payload;
}

export async function performVersionCheck(
  env: WorkerEnv,
): Promise<KVVersionPayload> {
  const now = new Date().toISOString();
  const appsRecord: Record<string, AppVersionRecord> = {};
  let mirroredMediaCount = 0;
  const MAX_MEDIA_PER_INVOCATION = 24;

  for (const app of MONITORED_APPS) {
    const priorRecord = await loadAppRecord(env, app.packageName);
    const scraped = await fetchGooglePlayApp(app.packageName);
    if (scraped.rawError) {
      console.error(
        `Scrape failed for ${app.packageName}: ${scraped.rawError}`,
      );
    }

    // Mirroring uses a preview of scrape-owned fields. The preview is not
    // authoritative; applyScrape re-merges against the current record inside
    // the per-package authority, and only that result is persisted.
    const preview = mergeScrapedRecord(priorRecord, scraped, app, now);
    if (
      env.PLAY_MEDIA &&
      preview.playMetadata &&
      mirroredMediaCount < MAX_MEDIA_PER_INVOCATION
    ) {
      const assetsToMirror: PlayMediaAsset[] = [];
      if (preview.iconAsset && !preview.iconAsset.fallbackPath) {
        assetsToMirror.push(preview.iconAsset);
      }
      if (
        preview.playMetadata.contentRatingIcon &&
        !preview.playMetadata.contentRatingIcon.fallbackPath
      ) {
        assetsToMirror.push(preview.playMetadata.contentRatingIcon);
      }
      for (const s of preview.playMetadata.screenshots) {
        if (!s.fallbackPath) assetsToMirror.push(s);
      }
      for (const r of preview.playMetadata.featuredReviews) {
        if (r.avatar && !r.avatar.fallbackPath) assetsToMirror.push(r.avatar);
      }
      const remainingQuota = MAX_MEDIA_PER_INVOCATION - mirroredMediaCount;
      if (assetsToMirror.length > 0 && remainingQuota > 0) {
        const toMirror = assetsToMirror.slice(0, remainingQuota);
        await mirrorAssetsBatch(
          env.PLAY_MEDIA,
          app.packageName,
          toMirror,
          remainingQuota,
          4,
        );
        mirroredMediaCount += toMirror.length;
      }
    }

    const fallbackPaths: Record<string, string> = {};
    const recordFallback = (asset: PlayMediaAsset | null | undefined) => {
      if (asset?.fallbackPath) {
        fallbackPaths[asset.sourceUrl] = asset.fallbackPath;
      }
    };
    recordFallback(preview.iconAsset);
    recordFallback(preview.playMetadata?.contentRatingIcon);
    for (const screenshot of preview.playMetadata?.screenshots ?? []) {
      recordFallback(screenshot);
    }
    for (const review of preview.playMetadata?.featuredReviews ?? []) {
      recordFallback(review.avatar);
    }

    const appRecord = await applyScrape(env, app.packageName, {
      app,
      scraped,
      now,
      fallbackPaths,
    });
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

export async function runScheduledTasks(
  cron: string,
  env: WorkerEnv,
): Promise<void> {
  try {
    if (cron === '0 */6 * * *') {
      await performVersionCheck(env);
    } else if (cron === '15 0 * * *') {
      await refreshDownloadStatistics(env);
    } else {
      console.warn(JSON.stringify({ event: 'unknown_cron', cron }));
    }
  } catch (error) {
    if (error instanceof AuthorityNotReadyError) {
      console.warn(
        JSON.stringify({
          event: 'authority_not_ready',
          cron,
          error: error.message,
        }),
      );
      return;
    }
    throw error;
  }
}

export default {
  async scheduled(
    event: ScheduledEvent,
    env: WorkerEnv,
    ctx: ExecutionContext,
  ): Promise<void> {
    ctx.waitUntil(runScheduledTasks(event.cron, env));
  },

  async fetch(request: Request, env: WorkerEnv): Promise<Response> {
    const url = new URL(request.url);

    if (routeUsesAppState(url.pathname) && !(await authorityGateOpen(env))) {
      return new Response(
        JSON.stringify({
          error:
            'App state authority migration gate is not set or still propagating',
        }),
        {
          status: 503,
          headers: {
            ...PUBLIC_CORS_HEADERS,
            'Content-Type': 'application/json',
            'Cache-Control': 'no-store',
          },
        },
      );
    }

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
      const started = await startCompatibilityRun(env, body.packageName, body);

      return new Response(
        JSON.stringify({
          success: true,
          queued: body.packageName,
          requestId: body.requestId,
          ...(started.mirrored ? {} : { mirrored: false }),
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
      const stateResult = await submitCompatibilityResult(
        env,
        submission.packageName,
        submission,
      );

      if (!stateResult.ok) {
        return new Response(
          JSON.stringify({
            error: stateResult.error,
            ...(stateResult.outstanding !== undefined
              ? { outstanding: stateResult.outstanding }
              : {}),
          }),
          {
            status: stateResult.status,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      return new Response(
        JSON.stringify({
          success: true,
          updated: submission.packageName,
          roles: stateResult.roles ?? [],
          ...(stateResult.mirrored === false ? { mirrored: false } : {}),
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

    // 3. Authenticated Play metadata enrichment endpoint
    if (url.pathname === '/api/play-metadata' && request.method === 'POST') {
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

      let body: PlayMetadataSubmission;
      try {
        body = (await request.json()) as PlayMetadataSubmission;
      } catch {
        return new Response(JSON.stringify({ error: 'Malformed JSON' }), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      if (!body || !Array.isArray(body.results)) {
        return new Response(
          JSON.stringify({
            error: 'Invalid submission format: results array required',
          }),
          {
            status: 400,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      // Phase 1: parse, validate and check freshness for every item before any
      // record is modified. Validation failures leave all records unchanged.
      const seenPackages = new Set<string>();
      const validated: PlayMetadataResultItem[] = [];

      for (const item of body.results) {
        if (
          !item ||
          typeof item !== 'object' ||
          typeof item.packageName !== 'string' ||
          !item.packageName.trim() ||
          typeof item.versionName !== 'string' ||
          !item.versionName.trim() ||
          !Number.isInteger(item.versionCode) ||
          item.versionCode <= 0
        ) {
          return new Response(
            JSON.stringify({ error: 'Malformed result item' }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        if (seenPackages.has(item.packageName)) {
          return new Response(
            JSON.stringify({
              error: `Duplicate package in submission: ${item.packageName}`,
            }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }
        seenPackages.add(item.packageName);

        const appRecord = await loadAppRecord(env, item.packageName);
        if (!appRecord) {
          return new Response(
            JSON.stringify({ error: `Unknown package: ${item.packageName}` }),
            {
              status: 400,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        // listingUpdatedAt must equal current scraped updatedAt
        const currentUpdatedAt = appRecord.updatedAt ?? null;
        if (item.listingUpdatedAt !== currentUpdatedAt) {
          return new Response(
            JSON.stringify({
              error: `Stale listingUpdatedAt for package ${item.packageName}`,
              currentUpdatedAt,
              receivedUpdatedAt: item.listingUpdatedAt,
            }),
            {
              status: 409,
              headers: { 'Content-Type': 'application/json' },
            },
          );
        }

        validated.push(item);
      }

      // Phase 2: apply each item through its per-package authority. The
      // authority re-checks listingUpdatedAt, so a listing change between
      // validation and write is reported rather than silently overwritten.
      const updatedPackages: string[] = [];
      const mirrorWarnings: string[] = [];
      const failures: Array<{
        packageName: string;
        reason: 'conflict' | 'backend-error';
        error: string;
        currentUpdatedAt?: string | null;
      }> = [];

      for (const item of validated) {
        try {
          const result = await updatePlayMetadata(env, item.packageName, item);
          if (result.ok) {
            updatedPackages.push(item.packageName);
            if (result.mirrored === false) {
              mirrorWarnings.push(item.packageName);
            }
          } else {
            failures.push({
              packageName: item.packageName,
              reason: 'conflict',
              error: result.error,
              ...(result.currentUpdatedAt !== undefined
                ? { currentUpdatedAt: result.currentUpdatedAt }
                : {}),
            });
          }
        } catch (error) {
          failures.push({
            packageName: item.packageName,
            reason: 'backend-error',
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }

      if (failures.length === 0) {
        return new Response(
          JSON.stringify({
            success: true,
            updatedPackages,
            ...(mirrorWarnings.length > 0
              ? { mirrored: false, mirrorWarnings }
              : {}),
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          },
        );
      }

      // KV does not provide a multi-record transaction. Report exactly which
      // packages committed and which did not instead of claiming atomicity.
      const status = failures.some((f) => f.reason === 'backend-error')
        ? 500
        : 409;
      return new Response(
        JSON.stringify({
          success: false,
          error: 'Partial metadata update',
          updatedPackages,
          failures,
          ...(mirrorWarnings.length > 0
            ? { mirrored: false, mirrorWarnings }
            : {}),
        }),
        {
          status,
          headers: { 'Content-Type': 'application/json' },
        },
      );
    }

    // 4. Public R2 media fallback proxy route
    if (
      url.pathname.startsWith('/media/play/') &&
      (request.method === 'GET' || request.method === 'HEAD')
    ) {
      // Expected path: /media/play/<package>/<hash>.<ext>
      const segments = url.pathname.replace(/^\/media\/play\//, '').split('/');
      if (segments.length !== 2) {
        return new Response('Invalid media path', { status: 400 });
      }
      const [pkg, file] = segments;
      if (
        !/^[a-zA-Z0-9_.]+$/.test(pkg) ||
        !/^[a-f0-9]{64}\.(png|jpg|webp|gif|svg)$/i.test(file)
      ) {
        return new Response('Invalid media path parameters', { status: 400 });
      }

      if (!env.PLAY_MEDIA) {
        return new Response('Media bucket unavailable', { status: 503 });
      }

      const r2Key = `play-media/${pkg}/${file}`;
      const object = await env.PLAY_MEDIA.get(r2Key);

      if (!object) {
        return new Response('Media not found', {
          status: 404,
          headers: {
            'Cache-Control': 'no-cache',
            'Access-Control-Allow-Origin': '*',
          },
        });
      }

      const headers = new Headers();
      object.writeHttpMetadata(headers);
      headers.set('ETag', object.httpEtag);
      headers.set('Cache-Control', 'public, max-age=31536000, immutable');
      headers.set('Access-Control-Allow-Origin', '*');
      headers.set('Access-Control-Allow-Methods', 'GET, HEAD, OPTIONS');

      if (request.headers.get('if-none-match') === object.httpEtag) {
        return new Response(null, {
          status: 304,
          headers,
        });
      }

      return new Response(request.method === 'HEAD' ? null : object.body, {
        status: 200,
        headers,
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

    // 4. Aggregate tests badge: /badges/tests.svg
    if (
      url.pathname === '/badges/tests.svg' ||
      url.pathname === '/badges/tests'
    ) {
      const payload = await loadStatusPayload(env);
      let totalPassed = 0;
      let totalFailed = 0;

      for (const appRecord of Object.values(payload.apps)) {
        const compat = appRecord.targetCompatibility;
        if (compat) {
          totalPassed += compat.passedCount ?? 0;
          totalFailed += compat.failedCount ?? 0;
        }
      }

      const totalTests = totalPassed + totalFailed;
      const amountStr = `${totalPassed}/${totalTests}`;

      let color = '#cf222e'; // red (<90% or 0 tests)
      if (totalTests > 0) {
        const pct = (totalPassed / totalTests) * 100;
        if (totalPassed === totalTests) {
          color = '#238636'; // green (100%)
        } else if (pct >= 90) {
          color = '#bc4c00'; // orange (90-99%)
        } else {
          color = '#cf222e'; // red (<90%)
        }
      }

      const svg = renderBadgeSvg('tests passing', amountStr, color, true);
      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=300',
        },
      });
    }

    // 5. Per-package compatibility badge: /badges/compatibility/:packageName.svg
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

    // 6. Per-package tests passing badge: /badges/tests/:packageName.svg
    if (url.pathname.startsWith('/badges/tests/')) {
      const rawPkg = url.pathname
        .replace('/badges/tests/', '')
        .replace(/\.svg$/, '');
      const packageName = decodeURIComponent(rawPkg);

      const record = await loadAppRecord(env, packageName);
      const targetCompat = record?.targetCompatibility;
      const passedCount = targetCompat?.passedCount ?? 0;
      const failedCount = targetCompat?.failedCount ?? 0;
      const totalTests = passedCount + failedCount;

      // For apps with no tests (including untested or 0/0), return an empty SVG so
      // the image link can always be present without displaying an empty badge.
      if (!targetCompat || totalTests === 0) {
        return new Response(
          '<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>',
          {
            status: 200,
            headers: {
              'Content-Type': 'image/svg+xml; charset=utf-8',
              'Cache-Control': 'public, max-age=300',
            },
          },
        );
      }

      const status = targetCompat.status;
      const allPassing = status === 'compatible' && failedCount === 0;
      const color = allPassing ? '#238636' : '#cf222e';
      const amountStr = `${passedCount}/${totalTests}`;

      const svg = renderBadgeSvg('tests passing', amountStr, color, true);
      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=300',
        },
      });
    }

    // 7. Download Statistics Charts
    if (
      url.pathname === '/charts/total-downloads.svg' &&
      request.method === 'GET'
    ) {
      let stats = await loadDownloadStatistics(env);
      if (!stats) {
        try {
          stats = await refreshDownloadStatistics(env);
        } catch (err) {
          console.error('Failed to initialize download statistics', err);
          const errorSvg = renderErrorChartSvg(
            'Total Downloads',
            'Download statistics temporarily unavailable',
          );
          return new Response(errorSvg, {
            status: 503,
            headers: {
              'Content-Type': 'image/svg+xml; charset=utf-8',
              'Cache-Control': 'no-store',
            },
          });
        }
      }

      const svg = renderTotalDownloadsChartSvg(stats.history, stats.updatedAt);
      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=3600, s-maxage=3600',
        },
      });
    }

    if (
      url.pathname === '/charts/release-downloads.svg' &&
      request.method === 'GET'
    ) {
      let stats = await loadDownloadStatistics(env);
      if (!stats) {
        try {
          stats = await refreshDownloadStatistics(env);
        } catch (err) {
          console.error('Failed to initialize download statistics', err);
          const errorSvg = renderErrorChartSvg(
            'Release Downloads',
            'Download statistics temporarily unavailable',
          );
          return new Response(errorSvg, {
            status: 503,
            headers: {
              'Content-Type': 'image/svg+xml; charset=utf-8',
              'Cache-Control': 'no-store',
            },
          });
        }
      }

      const lines = deriveReleaseLineDownloads(stats.assets);
      const svg = renderReleaseDownloadsChartSvg(lines, stats.updatedAt);
      return new Response(svg, {
        status: 200,
        headers: {
          'Content-Type': 'image/svg+xml; charset=utf-8',
          'Cache-Control': 'public, max-age=3600, s-maxage=3600',
        },
      });
    }

    // 4.5. Dedicated lightweight version API endpoint: GET /api/version/:packageName
    if (
      (url.pathname === '/api/version' ||
        url.pathname.startsWith('/api/version/')) &&
      (request.method === 'GET' || request.method === 'HEAD')
    ) {
      const rawPkg = url.pathname.replace(/^\/api\/version\/?/, '').trim();
      let packageName = decodeURIComponent(rawPkg);
      if (packageName.toLowerCase() === 'fizz') {
        packageName = 'com.ashtoncofer.Buzz';
      }
      if (!packageName) {
        return new Response(JSON.stringify({ error: 'Missing package name' }), {
          status: 400,
          headers: {
            ...PUBLIC_CORS_HEADERS,
            'Content-Type': 'application/json',
          },
        });
      }

      const record = await loadAppRecord(env, packageName);
      if (!record || !record.playVersion) {
        return new Response(
          JSON.stringify({ error: 'App version not found' }),
          {
            status: 404,
            headers: {
              ...PUBLIC_CORS_HEADERS,
              'Content-Type': 'application/json',
            },
          },
        );
      }

      const payload = {
        packageName,
        versionName: record.playVersion,
        versionCode: record.playVersionCode ?? null,
        updatedAt: record.updatedAt ?? record.checkedAt ?? null,
      };

      return new Response(
        request.method === 'HEAD' ? null : JSON.stringify(payload),
        {
          status: 200,
          headers: {
            ...PUBLIC_CORS_HEADERS,
            'Content-Type': 'application/json',
            'Cache-Control': 'public, max-age=3600',
          },
        },
      );
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
