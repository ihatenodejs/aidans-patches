import type { PlayMediaAsset } from './types';
import { ALLOWED_MEDIA_HOST } from './scraper';

export async function sha256Hex(input: string): Promise<string> {
  const data = new TextEncoder().encode(input);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, '0')).join('');
}

export function getFileExtensionFromContentType(contentType: string): string {
  const ct = contentType.toLowerCase().split(';')[0].trim();
  switch (ct) {
    case 'image/png':
      return 'png';
    case 'image/jpeg':
    case 'image/jpg':
      return 'jpg';
    case 'image/webp':
      return 'webp';
    case 'image/gif':
      return 'gif';
    case 'image/svg+xml':
      return 'svg';
    default:
      return 'png';
  }
}

export async function mirrorMediaAsset(
  bucket: R2Bucket,
  packageName: string,
  asset: PlayMediaAsset,
): Promise<string | null> {
  if (asset.fallbackPath) {
    return asset.fallbackPath;
  }

  try {
    const parsed = new URL(asset.sourceUrl);
    if (
      parsed.protocol !== 'https:' ||
      parsed.hostname !== ALLOWED_MEDIA_HOST
    ) {
      return null;
    }

    const res = await fetch(asset.sourceUrl, {
      headers: {
        'User-Agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
      },
    });

    if (!res.ok) {
      return null;
    }

    const contentType = res.headers.get('content-type') || '';
    if (!contentType.toLowerCase().startsWith('image/')) {
      return null;
    }

    const contentLengthHeader = res.headers.get('content-length');
    let contentLength: number | null = null;
    if (contentLengthHeader) {
      contentLength = parseInt(contentLengthHeader, 10);
      if (
        isNaN(contentLength) ||
        contentLength <= 0 ||
        contentLength > 8 * 1024 * 1024
      ) {
        return null;
      }
    }

    const bodyBytes = await res.arrayBuffer();
    if (bodyBytes.byteLength === 0 || bodyBytes.byteLength > 8 * 1024 * 1024) {
      return null;
    }

    const ext = getFileExtensionFromContentType(contentType);
    const hash = await sha256Hex(asset.sourceUrl);
    const r2Key = `play-media/${packageName}/${hash}.${ext}`;
    const fallbackPath = `/media/play/${packageName}/${hash}.${ext}`;

    await bucket.put(r2Key, bodyBytes, {
      httpMetadata: {
        contentType,
        cacheControl: 'public, max-age=31536000, immutable',
      },
      customMetadata: {
        sourceUrl: asset.sourceUrl,
      },
    });

    return fallbackPath;
  } catch {
    return null;
  }
}

export async function mirrorAssetsBatch(
  bucket: R2Bucket | undefined,
  packageName: string,
  assets: PlayMediaAsset[],
  maxToMirror = 24,
  concurrency = 4,
): Promise<void> {
  if (!bucket) return;

  const toMirror = assets.filter((a) => !a.fallbackPath).slice(0, maxToMirror);
  for (let i = 0; i < toMirror.length; i += concurrency) {
    const chunk = toMirror.slice(i, i + concurrency);
    await Promise.all(
      chunk.map(async (asset) => {
        const path = await mirrorMediaAsset(bucket, packageName, asset);
        if (path) {
          asset.fallbackPath = path;
        }
      }),
    );
  }
}
