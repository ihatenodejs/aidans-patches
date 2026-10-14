import type {
  CompatibilityRecord,
  OutstandingCompatibilityRequest,
} from './types';

export interface PublicAppStatus {
  appName?: string | null;
  playVersion?: string | null;
  iconUrl?: string | null;
  updatedAt?: string | null;
  updatedOn?: string | null;
  checkedAt?: string | null;
  status?: string | null;
  supportedVersions?: string[] | null;
  latestSupportedVersion?: string | null;
  targetCompatibility?: CompatibilityRecord | null;
  outstandingRequest?: OutstandingCompatibilityRequest | null;
  outstandingRequestId?: string | null;
}

export interface PublicLatestRelease {
  packageName?: string | null;
  appName?: string | null;
  playVersion?: string | null;
  iconUrl?: string | null;
  updatedAt?: string | null;
  updatedOn?: string | null;
}

export interface PublicStatusPayload {
  updatedAt?: string | null;
  apps: Record<string, PublicAppStatus>;
  latestRelease?: PublicLatestRelease | null;
}

export const STATUS_CACHE_KEY = 'aidans_patches_status_cache';
export const STATUS_CACHE_TTL_MS = 5 * 60 * 1000; // 5 minutes

let inFlightStatusPromise: Promise<PublicStatusPayload | null> | null = null;

function isObject(val: unknown): val is Record<string, unknown> {
  return typeof val === 'object' && val !== null && !Array.isArray(val);
}

function isValidStatusPayload(val: unknown): val is PublicStatusPayload {
  if (!isObject(val)) return false;
  return isObject(val.apps);
}

function getStoredPayload(): PublicStatusPayload | null {
  if (typeof window === 'undefined' || !window.sessionStorage) return null;
  try {
    const raw = window.sessionStorage.getItem(STATUS_CACHE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (
      isObject(parsed) &&
      typeof parsed.timestamp === 'number' &&
      Date.now() - parsed.timestamp < STATUS_CACHE_TTL_MS &&
      isValidStatusPayload(parsed.data)
    ) {
      return parsed.data;
    }
  } catch {
    // Ignore sessionStorage read or parse errors
  }
  return null;
}

function storePayload(data: PublicStatusPayload): void {
  if (typeof window === 'undefined' || !window.sessionStorage) return;
  try {
    window.sessionStorage.setItem(
      STATUS_CACHE_KEY,
      JSON.stringify({ data, timestamp: Date.now() }),
    );
  } catch {
    // Ignore sessionStorage write errors (e.g. quota exceeded)
  }
}

function getWorkerApiUrl(): string | null {
  if (typeof document === 'undefined') return null;
  const workerBase =
    document.body?.getAttribute('data-worker-url') ||
    'https://worker.patch.p0ntus.com';
  try {
    return new URL('/api/status', workerBase).toString();
  } catch {
    return null;
  }
}

export async function loadStatusPayload(): Promise<PublicStatusPayload | null> {
  const cached = getStoredPayload();
  if (cached) return cached;

  if (inFlightStatusPromise) {
    return inFlightStatusPromise;
  }

  inFlightStatusPromise = (async () => {
    try {
      const endpoint = getWorkerApiUrl();
      if (!endpoint) return null;

      const response = await fetch(endpoint);
      if (!response.ok) return null;

      const json: unknown = await response.json();
      if (!isValidStatusPayload(json)) return null;

      storePayload(json);
      return json;
    } catch {
      return null;
    } finally {
      inFlightStatusPromise = null;
    }
  })();

  return inFlightStatusPromise;
}
