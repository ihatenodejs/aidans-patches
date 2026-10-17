import type { PublicStatusPayload } from './status-client';

const ALLOWED_GOOGLE_PLAY_HOST = 'play-lh.googleusercontent.com';

/**
 * Returns the encoded relative URL for the committed icon asset.
 * Returns null if packageName is empty.
 */
export function getCachedAppIconUrl(packageName: string): string | null {
  if (!packageName || !packageName.trim()) return null;
  return `/icons/${encodeURIComponent(packageName.trim())}.png`;
}

/**
 * Resolves the Google Play icon URL for a package from the public status payload.
 * Accepts only HTTPS URLs whose hostname is exactly play-lh.googleusercontent.com.
 * Returns null if the payload, package, or URL is missing, malformed, or from a disallowed host.
 */
export function resolveGooglePlayIconUrl(
  payload: PublicStatusPayload | null | undefined,
  packageName: string,
): string | null {
  if (!payload || !payload.apps || !packageName || !packageName.trim()) {
    return null;
  }

  const trimmedPkg = packageName.trim();
  const appStatus = payload.apps[trimmedPkg];
  const rawUrl = appStatus?.iconUrl;

  if (!rawUrl || typeof rawUrl !== 'string' || !rawUrl.trim()) {
    return null;
  }

  try {
    const parsed = new URL(rawUrl.trim());
    if (parsed.protocol !== 'https:') {
      return null;
    }
    if (parsed.hostname !== ALLOWED_GOOGLE_PLAY_HOST) {
      return null;
    }
    return parsed.toString();
  } catch {
    return null;
  }
}
