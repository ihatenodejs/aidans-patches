import type {
  CompatibilityRecord,
  OutstandingCompatibilityRequest,
} from './types';

export interface PlayMediaAsset {
  sourceUrl: string;
  fallbackPath?: string | null;
  width?: number | null;
  height?: number | null;
  alt?: string | null;
}

export interface PlayReview {
  author: string;
  rating: number;
  date?: string | null;
  text: string;
  helpfulCount?: number | null;
  avatar?: PlayMediaAsset | null;
}

export interface PlayStoreMetadata {
  summary?: string | null;
  description?: string | null;
  developer?: string | null;
  category?: string | null;
  categoryId?: string | null;
  releaseNotes?: string | null;
  installs?: string | null;
  score?: number | null;
  ratingsCount?: number | null;
  reviewsCount?: number | null;
  contentRating?: string | null;
  contentRatingIcon?: PlayMediaAsset | null;
  releasedOn?: string | null;
  minAndroid?: string | null;
  inAppPurchases?: string | null;
  permissionGroups?: string[] | null;
  screenshots?: PlayMediaAsset[] | null;
  featuredReviews?: PlayReview[] | null;
}

export interface PublicAppStatus {
  appName?: string | null;
  playVersion?: string | null;
  playVersionCode?: number | null;
  playVersionSource?: 'web' | 'authenticated' | null;
  playVersionVerifiedForUpdatedAt?: string | null;
  needsAuthenticatedVersion?: boolean | null;
  iconUrl?: string | null;
  iconAsset?: PlayMediaAsset | null;
  updatedAt?: string | null;
  updatedOn?: string | null;
  checkedAt?: string | null;
  status?: string | null;
  supportedVersions?: string[] | null;
  latestSupportedVersion?: string | null;
  targetCompatibility?: CompatibilityRecord | null;
  outstandingRequest?: OutstandingCompatibilityRequest | null;
  outstandingRequestId?: string | null;
  playMetadata?: PlayStoreMetadata | null;
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

export function resolveFallbackMediaUrl(
  fallbackPath: string | null | undefined,
  workerBaseUrl?: string | null,
): string | null {
  if (!fallbackPath || typeof fallbackPath !== 'string') return null;
  const cleanPath = fallbackPath.trim();
  if (!cleanPath.startsWith('/media/play/')) return null;
  const base =
    workerBaseUrl ||
    (typeof document !== 'undefined'
      ? document.body?.getAttribute('data-worker-url') ||
        'https://worker.patch.p0ntus.com'
      : 'https://worker.patch.p0ntus.com');
  try {
    return new URL(cleanPath, base).toString();
  } catch {
    return null;
  }
}
