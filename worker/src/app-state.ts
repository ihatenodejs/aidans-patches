import { compareAppVersions } from './comparator';
import { mergeScrapedRecord } from './app-record';
import type { ScrapedAppInfo } from './scraper';
import type {
  AppVersionRecord,
  CompatibilityRecord,
  CompatibilityResultSubmission,
  CompatibilityRunStart,
  FailureStage,
  OutstandingCompatibilityRequest,
  PlayMetadataResultItem,
  TargetAppConfig,
  WorkerEnv,
} from './types';

const RECORD_STORAGE_KEY = 'app-record';
export const AUTHORITY_GATE_KEY = 'app_state_authority:v1';
export const AUTHORITY_GATE_SETTLE_MS = 2 * 60 * 1000;

export class AuthorityNotReadyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'AuthorityNotReadyError';
  }
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

export function appRecordKey(packageName: string): string {
  return `app_version:${packageName}`;
}

interface StoredAppState {
  initialized: true;
  packageName: string;
  record: AppVersionRecord | null;
}

export interface ScrapeSubmission {
  app: TargetAppConfig;
  scraped: ScrapedAppInfo;
  now: string;
  fallbackPaths?: Record<string, string>;
}

export type AppStateOperation =
  | { operation: 'read' }
  | { operation: 'write'; record: AppVersionRecord }
  | { operation: 'run-start'; submission: CompatibilityRunStart }
  | {
      operation: 'compatibility-result';
      submission: CompatibilityResultSubmission;
    }
  | { operation: 'play-metadata'; item: PlayMetadataResultItem }
  | { operation: 'scrape'; submission: ScrapeSubmission };

type AppStateRequest = AppStateOperation & { packageName: string };

export type AppStateResponse =
  | {
      ok: true;
      record: AppVersionRecord | null;
      roles?: string[];
      mirrored?: boolean;
    }
  | {
      ok: false;
      status: number;
      error: string;
      currentUpdatedAt?: string | null;
      receivedUpdatedAt?: string | null;
      outstanding?: string | null;
    };

function failure(
  status: number,
  error: string,
  extra: Partial<
    Pick<
      Extract<AppStateResponse, { ok: false }>,
      'currentUpdatedAt' | 'receivedUpdatedAt' | 'outstanding'
    >
  > = {},
): AppStateResponse {
  return { ok: false, status, error, ...extra };
}

function applyRunStart(
  prior: AppVersionRecord | null,
  body: CompatibilityRunStart,
): AppVersionRecord {
  const now = new Date().toISOString();
  const outstandingRequest: OutstandingCompatibilityRequest = {
    requestId: body.requestId,
    targetVersion: body.targetVersion,
    playVersion: prior?.playVersion ?? null,
    expectedRoles: ['target'],
    dispatchedAt: now,
    gitRevision: body.gitRevision,
  };

  return {
    appName: body.appName,
    playVersion: prior?.playVersion ?? null,
    playVersionCode: prior?.playVersionCode ?? null,
    playVersionSource: prior?.playVersionSource ?? null,
    playVersionVerifiedForUpdatedAt:
      prior?.playVersionVerifiedForUpdatedAt ?? null,
    needsAuthenticatedVersion: prior?.needsAuthenticatedVersion ?? false,
    iconUrl: prior?.iconUrl ?? null,
    iconAsset: prior?.iconAsset ?? null,
    playMetadata: prior?.playMetadata ?? null,
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
}

function applyCompatibilityResult(
  appRecord: AppVersionRecord | null,
  submission: CompatibilityResultSubmission,
): { write: boolean; next?: AppVersionRecord; response: AppStateResponse } {
  if (!appRecord) {
    return {
      write: false,
      response: failure(400, 'App record not initialized'),
    };
  }

  const outstanding = appRecord.outstandingRequest;
  if (!outstanding || outstanding.requestId !== submission.requestId) {
    return {
      write: false,
      response: failure(409, 'Stale, replayed, or mismatched request ID', {
        outstanding: outstanding?.requestId ?? null,
      }),
    };
  }

  const expectedRoles = outstanding.expectedRoles;
  if (submission.results.length !== expectedRoles.length) {
    return {
      write: false,
      response: failure(
        400,
        `Expected ${expectedRoles.length} results for roles [${expectedRoles.join(', ')}], got ${submission.results.length}`,
      ),
    };
  }

  const seenRoles = new Set<string>();
  const recordsToApply: CompatibilityRecord[] = [];

  for (const res of submission.results) {
    if (!res || typeof res !== 'object') {
      return { write: false, response: failure(400, 'Invalid result item') };
    }
    if (res.role !== 'target' && res.role !== 'latest') {
      return {
        write: false,
        response: failure(400, `Invalid role: ${res.role}`),
      };
    }
    if (!expectedRoles.includes(res.role)) {
      return {
        write: false,
        response: failure(400, `Unexpected role: ${res.role}`),
      };
    }
    if (seenRoles.has(res.role)) {
      return {
        write: false,
        response: failure(400, `Duplicate role in results: ${res.role}`),
      };
    }
    seenRoles.add(res.role);

    if (!FINAL_STATUSES[res.status]) {
      return {
        write: false,
        response: failure(400, `Invalid status: ${res.status}`),
      };
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
      return {
        write: false,
        response: failure(400, 'Malformed result fields'),
      };
    }
    if (
      outstanding.gitRevision &&
      res.gitRevision !== outstanding.gitRevision
    ) {
      return {
        write: false,
        response: failure(
          409,
          `Result git revision '${res.gitRevision}' does not match outstanding '${outstanding.gitRevision}'`,
        ),
      };
    }
    if (
      res.failureStage !== undefined &&
      res.failureStage !== null &&
      (typeof res.failureStage !== 'string' ||
        !VALID_FAILURE_STAGES[res.failureStage])
    ) {
      return {
        write: false,
        response: failure(400, 'Malformed failureStage field'),
      };
    }
    if (
      res.failureReason !== undefined &&
      res.failureReason !== null &&
      typeof res.failureReason !== 'string'
    ) {
      return {
        write: false,
        response: failure(400, 'Malformed failureReason field'),
      };
    }
    if (
      res.role === 'target' &&
      res.versionName !== outstanding.targetVersion
    ) {
      return {
        write: false,
        response: failure(
          409,
          `Tested target version '${res.versionName}' does not match expected '${outstanding.targetVersion}'`,
        ),
      };
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

  const next: AppVersionRecord = { ...appRecord };
  for (const rec of recordsToApply) {
    next.targetCompatibility = rec;
  }

  if (submission.acquiredPlayVersion) {
    const acquired = submission.acquiredPlayVersion.trim();
    next.playVersion = acquired;
    next.playVersionCode = submission.acquiredPlayVersionCode ?? null;
    next.playVersionSource = 'authenticated';
    next.playVersionVerifiedForUpdatedAt = next.updatedAt ?? null;
    next.needsAuthenticatedVersion = false;
    next.status = compareAppVersions(next.latestSupportedVersion, acquired);
    next.playVersionReleaseUpdatedAt = next.updatedAt ?? null;
  }

  next.outstandingRequest = null;
  next.outstandingRequestId = null;

  return {
    write: true,
    next,
    response: {
      ok: true,
      record: next,
      roles: recordsToApply.map((r) => r.role),
    },
  };
}

function applyPlayMetadataUpdate(
  appRecord: AppVersionRecord | null,
  item: PlayMetadataResultItem,
): { write: boolean; next?: AppVersionRecord; response: AppStateResponse } {
  if (!appRecord) {
    return {
      write: false,
      response: failure(400, `Unknown package: ${item.packageName}`),
    };
  }

  const currentUpdatedAt = appRecord.updatedAt ?? null;
  if (item.listingUpdatedAt !== currentUpdatedAt) {
    return {
      write: false,
      response: failure(
        409,
        `Stale listingUpdatedAt for package ${item.packageName}`,
        {
          currentUpdatedAt,
          receivedUpdatedAt: item.listingUpdatedAt,
        },
      ),
    };
  }

  const next: AppVersionRecord = {
    ...appRecord,
    playVersion: item.versionName.trim(),
    playVersionCode: item.versionCode,
    playVersionSource: 'authenticated',
    playVersionVerifiedForUpdatedAt: currentUpdatedAt,
    needsAuthenticatedVersion: false,
    status: compareAppVersions(
      appRecord.latestSupportedVersion,
      item.versionName.trim(),
    ),
  };

  return { write: true, next, response: { ok: true, record: next } };
}

/**
 * Per-package serialization authority for `app_version:` records.
 *
 * All read-modify-write transitions run inside one Durable Object instance and
 * one transactional storage commit. KV is only a migration source on first use
 * and a rollback mirror afterwards; it is never read to make a decision after
 * initialization.
 */
export class AppStateDurableObject {
  private readonly state: DurableObjectState;
  private readonly env: WorkerEnv;
  private tail: Promise<void> = Promise.resolve();

  constructor(state: DurableObjectState, env: WorkerEnv) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const body = (await request.json()) as AppStateRequest;
    if (!body || typeof body.packageName !== 'string' || !body.packageName) {
      return Response.json(failure(400, 'Missing package name'));
    }

    try {
      const response = await this.enqueue(() => this.handle(body));
      return Response.json(response);
    } catch (error) {
      if (error instanceof AuthorityNotReadyError) {
        return Response.json(
          { ok: false, status: 503, error: error.message },
          { status: 503 },
        );
      }
      console.error('App state authority failure', {
        packageName: body.packageName,
        operation: body.operation,
        error: error instanceof Error ? error.message : String(error),
      });
      return Response.json(
        { ok: false, status: 500, error: 'State authority unavailable' },
        { status: 500 },
      );
    }
  }

  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.tail.then(
      () => task(),
      () => task(),
    );
    this.tail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async handle(request: AppStateRequest): Promise<AppStateResponse> {
    await this.ensureInitialized(request.packageName);

    switch (request.operation) {
      case 'read': {
        const stored =
          await this.state.storage.get<StoredAppState>(RECORD_STORAGE_KEY);
        return stored?.record
          ? { ok: true, record: stored.record }
          : { ok: true, record: null };
      }
      case 'write':
        return this.mutate(request.packageName, () => ({
          write: true,
          next: request.record,
          response: { ok: true, record: request.record },
        }));
      case 'run-start':
        return this.mutate(request.packageName, (current) => {
          const updated = applyRunStart(current, request.submission);
          return {
            write: true,
            next: updated,
            response: { ok: true, record: updated },
          };
        });
      case 'compatibility-result':
        return this.mutate(request.packageName, (current) =>
          applyCompatibilityResult(current, request.submission),
        );
      case 'play-metadata':
        return this.mutate(request.packageName, (current) =>
          applyPlayMetadataUpdate(current, request.item),
        );
      case 'scrape':
        return this.mutate(request.packageName, (current) => {
          const updated = mergeScrapedRecord(
            current,
            request.submission.scraped,
            request.submission.app,
            request.submission.now,
            request.submission.fallbackPaths,
          );
          return {
            write: true,
            next: updated,
            response: { ok: true, record: updated },
          };
        });
      default:
        return failure(400, 'Unknown state operation');
    }
  }

  private async ensureInitialized(packageName: string): Promise<void> {
    const existing =
      await this.state.storage.get<StoredAppState>(RECORD_STORAGE_KEY);
    if (existing?.initialized) return;

    // Cutting over from KV is a one-way, authoritative seed. Require an
    // operator-set gate so an automatic deploy cannot seed while old-version
    // writers are still in flight, and wait out KV propagation after the gate
    // is written. If the gate is missing or still settling, no authority state
    // is created and callers receive 503.
    const gateValue = this.env.PATCHES_KV
      ? await this.env.PATCHES_KV.get(AUTHORITY_GATE_KEY)
      : null;
    if (!gateValue) {
      throw new AuthorityNotReadyError(
        `App state migration gate '${AUTHORITY_GATE_KEY}' is not set`,
      );
    }
    const gateTime = Date.parse(gateValue);
    if (
      Number.isNaN(gateTime) ||
      Date.now() < gateTime + AUTHORITY_GATE_SETTLE_MS
    ) {
      throw new AuthorityNotReadyError(
        `App state migration gate '${AUTHORITY_GATE_KEY}' is not stable yet`,
      );
    }

    let record: AppVersionRecord | null = null;
    if (this.env.PATCHES_KV) {
      const raw = await this.env.PATCHES_KV.get(appRecordKey(packageName));
      if (raw) {
        try {
          record = JSON.parse(raw) as AppVersionRecord;
        } catch {
          console.error('Ignoring malformed legacy app record', {
            packageName,
          });
        }
      }
    }

    await this.state.storage.put<StoredAppState>(RECORD_STORAGE_KEY, {
      initialized: true,
      packageName,
      record,
    });
  }

  private async mutate(
    packageName: string,
    apply: (current: AppVersionRecord | null) => {
      write: boolean;
      next?: AppVersionRecord;
      response: AppStateResponse;
    },
  ): Promise<AppStateResponse> {
    const outcome = this.state.storage.transactionSync(() => {
      const stored =
        this.state.storage.kv.get<StoredAppState>(RECORD_STORAGE_KEY);
      const applied = apply(stored?.record ?? null);
      if (applied.write) {
        this.state.storage.kv.put<StoredAppState>(RECORD_STORAGE_KEY, {
          initialized: true,
          packageName,
          record: applied.next ?? null,
        });
      }
      return applied;
    });

    let mirrored = true;
    if (outcome.write && outcome.next) {
      try {
        await this.mirror(packageName, outcome.next);
      } catch (error) {
        mirrored = false;
        console.error('App state authority committed but KV mirror failed', {
          packageName,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
    if (outcome.response.ok && outcome.write) {
      return { ...outcome.response, mirrored };
    }
    return outcome.response;
  }

  private async mirror(
    packageName: string,
    record: AppVersionRecord,
  ): Promise<void> {
    if (!this.env.PATCHES_KV) return;
    await this.env.PATCHES_KV.put(
      appRecordKey(packageName),
      JSON.stringify(record),
    );
  }
}

async function requestAppState(
  env: WorkerEnv,
  packageName: string,
  operation: AppStateOperation,
): Promise<AppStateResponse> {
  if (!env.APP_STATE) {
    throw new Error('APP_STATE binding is required for app state access');
  }
  const id = env.APP_STATE.idFromName(packageName);
  const stub = env.APP_STATE.get(id);
  const response = await stub.fetch('https://app-state.internal/', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ...operation, packageName }),
  });
  if (!response.ok) {
    if (response.status === 503) {
      const body = (await response.json().catch(() => ({}))) as {
        error?: string;
      };
      throw new AuthorityNotReadyError(
        body.error ?? 'App state authority is not ready',
      );
    }
    throw new Error(
      `App state authority failed for ${packageName}: HTTP ${response.status}`,
    );
  }
  return (await response.json()) as AppStateResponse;
}

export async function readAppRecord(
  env: WorkerEnv,
  packageName: string,
): Promise<AppVersionRecord | null> {
  const response = await requestAppState(env, packageName, {
    operation: 'read',
  });
  if (!response.ok) {
    throw new Error(response.error);
  }
  return response.record;
}

export async function writeAppRecord(
  env: WorkerEnv,
  packageName: string,
  record: AppVersionRecord,
): Promise<void> {
  const response = await requestAppState(env, packageName, {
    operation: 'write',
    record,
  });
  if (!response.ok) {
    throw new Error(response.error);
  }
}

export async function applyScrape(
  env: WorkerEnv,
  packageName: string,
  submission: ScrapeSubmission,
): Promise<AppVersionRecord> {
  const response = await requestAppState(env, packageName, {
    operation: 'scrape',
    submission,
  });
  if (!response.ok || !response.record) {
    throw new Error(response.ok ? 'Missing app record' : response.error);
  }
  return response.record;
}

export async function startCompatibilityRun(
  env: WorkerEnv,
  packageName: string,
  submission: CompatibilityRunStart,
): Promise<{ record: AppVersionRecord; mirrored: boolean }> {
  const response = await requestAppState(env, packageName, {
    operation: 'run-start',
    submission,
  });
  if (!response.ok || !response.record) {
    throw new Error(response.ok ? 'Missing app record' : response.error);
  }
  return {
    record: response.record,
    mirrored: response.mirrored !== false,
  };
}

export async function submitCompatibilityResult(
  env: WorkerEnv,
  packageName: string,
  submission: CompatibilityResultSubmission,
): Promise<AppStateResponse> {
  return requestAppState(env, packageName, {
    operation: 'compatibility-result',
    submission,
  });
}

export async function updatePlayMetadata(
  env: WorkerEnv,
  packageName: string,
  item: PlayMetadataResultItem,
): Promise<AppStateResponse> {
  return requestAppState(env, packageName, {
    operation: 'play-metadata',
    item,
  });
}
