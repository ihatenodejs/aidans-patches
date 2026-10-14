import type { PlayMediaAsset, PlayReview, PlayStoreMetadata } from './types';

export interface ScrapedAppInfo {
  playVersion: string | null;
  iconUrl: string | null;
  updatedAt: string | null;
  updatedOn: string | null;
  isNotFound: boolean;
  rawError?: string;
  metadata?: Partial<PlayStoreMetadata>;
}

const USER_AGENT =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

const PRIMARY_VERSION_PATTERN =
  /\[\[\["([0-9]+\.[0-9]+(?:\.[0-9]+)*[^"]*)"\]\],\[\[\[\d+\]\],\[\[\[\d+/;

const SECONDARY_VERSION_PATTERN =
  /\[\[\["([0-9]+\.[0-9]+(?:\.[0-9]+)*[^"]*)"\]\]/;

const OG_IMAGE_PATTERN = /<meta\s+property="og:image"\s+content="([^"]+)"/i;
const OG_DESC_PATTERN =
  /<meta\s+(?:property="og:description"|name="description")\s+content="([^"]+)"/i;

const HTML_UPDATE_DATE_PATTERN =
  /Updated on<\/div>\s*<div[^>]*>([^<]+)<\/div>/i;

const JSON_UPDATE_DATE_PATTERN =
  /\[\["([A-Za-z]{3}\s+\d{1,2},\s+\d{4})",\[(\d+),/;

const HTML_CATEGORY_PATTERN =
  /href="\/store\/apps\/category\/([A-Z0-9_]+)"[^>]*>([\s\S]*?)<\/a>/i;

const HTML_DESCRIPTION_PATTERN =
  /data-g-id="description"[^>]*>([\s\S]*?)<\/div>/i;

const HTML_WHATSNEW_PATTERN = /data-g-id="whatsnew"[^>]*>([\s\S]*?)<\/div>/i;

export const ALLOWED_MEDIA_HOST = 'play-lh.googleusercontent.com';

export function isAllowedMediaUrl(urlStr: string): boolean {
  try {
    const parsed = new URL(urlStr);
    return (
      parsed.protocol === 'https:' && parsed.hostname === ALLOWED_MEDIA_HOST
    );
  } catch {
    return false;
  }
}

export function buildMediaAsset(
  rawUrl: string | null | undefined,
  width: number | null = null,
  height: number | null = null,
  alt: string | null = null,
): PlayMediaAsset | null {
  if (!rawUrl) return null;
  const cleaned = rawUrl.replace(/&amp;/g, '&').trim();
  if (!isAllowedMediaUrl(cleaned)) return null;
  return {
    sourceUrl: cleaned,
    fallbackPath: null,
    width,
    height,
    alt: alt ? alt.trim().slice(0, 200) : null,
  };
}

export function htmlToPlainText(html: string): string {
  if (!html) return '';
  let text = html
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'")
    .replace(/&nbsp;/g, ' ')
    .replace(/&#(\d+);/g, (_, code) => {
      const num = parseInt(code, 10);
      return !isNaN(num) ? String.fromCharCode(num) : '';
    })
    .replace(/&#x([0-9a-f]+);/gi, (_, code) => {
      const num = parseInt(code, 16);
      return !isNaN(num) ? String.fromCharCode(num) : '';
    });

  text = text
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/p>/gi, '\n\n')
    .replace(/<\/li>/gi, '\n')
    .replace(/<li[^>]*>/gi, '• ')
    .replace(/<\/h[1-6]>/gi, '\n\n')
    .replace(/<[^>]+>/g, '');

  text = text
    .split('\n')
    .map((line) => line.replace(/[ \t]+/g, ' ').trim())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();

  return text;
}

function getNested(obj: unknown, path: number[]): unknown {
  let curr = obj;
  for (const step of path) {
    if (curr === null || curr === undefined) return undefined;
    if (!Array.isArray(curr) && typeof curr !== 'object') return undefined;
    curr = (curr as Record<number, unknown>)[step];
  }
  return curr;
}

export function parseGooglePlayHtml(html: string): {
  playVersion: string | null;
  iconUrl: string | null;
  updatedAt: string | null;
  updatedOn: string | null;
  metadata: Partial<PlayStoreMetadata>;
} {
  // 1. Icon
  let iconUrl: string | null = null;
  const ogMatch = html.match(OG_IMAGE_PATTERN);
  if (ogMatch && ogMatch[1]) {
    const raw = ogMatch[1].replace(/&amp;/g, '&');
    if (isAllowedMediaUrl(raw)) {
      iconUrl = raw;
    }
  }

  // 2. Update Date
  let updatedOn: string | null = null;
  let updatedAt: string | null = null;
  const jsonDateMatch = html.match(JSON_UPDATE_DATE_PATTERN);
  if (jsonDateMatch) {
    updatedOn = jsonDateMatch[1].trim();
    const tsSeconds = parseInt(jsonDateMatch[2], 10);
    if (!isNaN(tsSeconds) && tsSeconds > 0) {
      updatedAt = new Date(tsSeconds * 1000).toISOString();
    }
  }
  if (!updatedOn) {
    const htmlDateMatch = html.match(HTML_UPDATE_DATE_PATTERN);
    if (htmlDateMatch && htmlDateMatch[1]) {
      updatedOn = htmlDateMatch[1].trim();
      const parsed = Date.parse(updatedOn);
      if (!isNaN(parsed)) {
        updatedAt = new Date(parsed).toISOString();
      }
    }
  }

  // 3. Version
  let playVersion: string | null = null;
  const primaryMatch = html.match(PRIMARY_VERSION_PATTERN);
  if (primaryMatch && primaryMatch[1]) {
    playVersion = primaryMatch[1].trim();
  } else {
    const secondaryMatch = html.match(SECONDARY_VERSION_PATTERN);
    if (secondaryMatch && secondaryMatch[1]) {
      playVersion = secondaryMatch[1].trim();
    }
  }

  const metadata: Partial<PlayStoreMetadata> = {
    permissionGroups: [],
    screenshots: [],
    featuredReviews: [],
  };

  // 4. Parse Schema.org JSON-LD
  const jsonLdMatches = html.matchAll(
    /<script\s+type="application\/ld\+json"[^>]*>([\s\S]*?)<\/script>/gi,
  );
  for (const match of jsonLdMatches) {
    if (!match[1]) continue;
    try {
      const data = JSON.parse(match[1]);
      if (
        data['@type'] === 'SoftwareApplication' ||
        data['@type'] === 'MobileApplication'
      ) {
        if (data.aggregateRating) {
          const val = parseFloat(data.aggregateRating.ratingValue);
          if (!isNaN(val) && val >= 0 && val <= 5) {
            metadata.score = Math.round(val * 10) / 10;
          }
          const rCount = parseInt(data.aggregateRating.ratingCount, 10);
          if (!isNaN(rCount) && rCount >= 0) {
            metadata.ratingsCount = rCount;
          }
          const revCount = parseInt(data.aggregateRating.reviewCount, 10);
          if (!isNaN(revCount) && revCount >= 0) {
            metadata.reviewsCount = revCount;
          }
        }
        if (
          typeof data.operatingSystem === 'string' &&
          data.operatingSystem.trim()
        ) {
          const os = data.operatingSystem.trim();
          // Only accept if it provides meaningful version details, not just bare "ANDROID"
          if (os.toUpperCase() !== 'ANDROID' && os.length <= 100) {
            metadata.minAndroid = os;
          }
        }
        if (
          typeof data.datePublished === 'string' &&
          data.datePublished.trim()
        ) {
          metadata.releasedOn = data.datePublished.trim().slice(0, 100);
        }
      }
    } catch {
      // ignore JSON parse errors in JSON-LD
    }
  }

  // 5. Parse ds:5 payload
  const ds5Match = html.match(
    /AF_initDataCallback\(\{key:\s*['"]ds:5['"][^}]*data:\s*([\s\S]*?),\s*sideChannel/i,
  );
  let ds5Data: unknown = null;
  if (ds5Match && ds5Match[1]) {
    try {
      ds5Data = JSON.parse(ds5Match[1]);
    } catch {
      ds5Data = null;
    }
  }

  if (ds5Data) {
    // Description HTML: [1,2,72,0,1]
    const rawDesc = getNested(ds5Data, [1, 2, 72, 0, 1]);
    if (typeof rawDesc === 'string') {
      metadata.description = htmlToPlainText(rawDesc).slice(0, 6000);
    } else if (rawDesc === null) {
      metadata.description = null;
    }

    // Developer: [1,2,68,0]
    const rawDev = getNested(ds5Data, [1, 2, 68, 0]);
    if (typeof rawDev === 'string') {
      metadata.developer = htmlToPlainText(rawDev).slice(0, 200);
    } else if (rawDev === null) {
      metadata.developer = null;
    }

    // Summary: [1,2,73,0,1]
    const rawSummary = getNested(ds5Data, [1, 2, 73, 0, 1]);
    if (typeof rawSummary === 'string') {
      metadata.summary = htmlToPlainText(rawSummary).slice(0, 500);
    } else if (rawSummary === null) {
      metadata.summary = null;
    }

    // Category Label & ID: [1,2,79,0,0,0] & [1,2,79,0,0,2]
    const rawCatLabel = getNested(ds5Data, [1, 2, 79, 0, 0, 0]);
    if (typeof rawCatLabel === 'string') {
      metadata.category = htmlToPlainText(rawCatLabel).slice(0, 100);
    } else if (rawCatLabel === null) {
      metadata.category = null;
    }

    const rawCatId = getNested(ds5Data, [1, 2, 79, 0, 0, 2]);
    if (typeof rawCatId === 'string') {
      metadata.categoryId = rawCatId.trim().slice(0, 100);
    } else if (rawCatId === null) {
      metadata.categoryId = null;
    }

    // Release notes / What's new: [1,2,144,1,1]
    const rawNotes = getNested(ds5Data, [1, 2, 144, 1, 1]);
    if (typeof rawNotes === 'string') {
      metadata.releaseNotes = htmlToPlainText(rawNotes).slice(0, 2000);
    } else if (rawNotes === null) {
      metadata.releaseNotes = null;
    }
  }

  // Fallbacks if not extracted via ds:5
  if (metadata.summary === undefined) {
    const descMeta = html.match(OG_DESC_PATTERN);
    if (descMeta && descMeta[1]) {
      metadata.summary = htmlToPlainText(descMeta[1]).slice(0, 500);
    }
  }

  if (metadata.description === undefined) {
    const descHtmlMatch = html.match(HTML_DESCRIPTION_PATTERN);
    if (descHtmlMatch && descHtmlMatch[1]) {
      metadata.description = htmlToPlainText(descHtmlMatch[1]).slice(0, 6000);
    }
  }

  if (metadata.categoryId === undefined || metadata.category === undefined) {
    const catMatch = html.match(HTML_CATEGORY_PATTERN);
    if (catMatch && catMatch[1] && catMatch[1] !== 'FAMILY') {
      if (metadata.categoryId === undefined) {
        metadata.categoryId = catMatch[1].trim().slice(0, 100);
      }
      if (metadata.category === undefined && catMatch[2]) {
        const plain = htmlToPlainText(catMatch[2]).slice(0, 100);
        if (plain) metadata.category = plain;
      }
    }
  }

  if (metadata.releaseNotes === undefined) {
    const notesMatch = html.match(HTML_WHATSNEW_PATTERN);
    if (notesMatch && notesMatch[1]) {
      metadata.releaseNotes = htmlToPlainText(notesMatch[1]).slice(0, 2000);
    }
  }

  // 6. Installs / Downloads label
  const downloadsMatch = html.match(
    /class="ClM7O">([^<]+)<\/div>\s*<div[^>]*>Downloads<\/div>/i,
  );
  if (downloadsMatch && downloadsMatch[1]) {
    metadata.installs = downloadsMatch[1].trim();
  }

  // 7. Content Rating
  const contentRatingBlockMatch = html.match(
    /class="J1DxQM"[^>]*>([\s\S]*?)<\/div>/i,
  );
  if (contentRatingBlockMatch && contentRatingBlockMatch[1]) {
    const block = contentRatingBlockMatch[1];
    const labelMatch = block.match(/<span>([^<]+)<\/span>/i);
    if (labelMatch && labelMatch[1]) {
      metadata.contentRating = labelMatch[1].trim().slice(0, 100);
    }
    const iconMatch = block.match(/<img[^>]+src="([^"]+)"/i);
    if (iconMatch && iconMatch[1]) {
      const asset = buildMediaAsset(iconMatch[1], null, null, 'Content rating');
      if (asset) {
        metadata.contentRatingIcon = asset;
      }
    }
  }

  // 8. In-App Purchases text/flag
  const iapMatch = html.match(/class="rxicEc">([^<]+)<\/div>/i);
  if (
    iapMatch &&
    iapMatch[1] &&
    iapMatch[1].toLowerCase().includes('in-app purchase')
  ) {
    metadata.inAppPurchases = iapMatch[1].trim().slice(0, 100);
  }

  // 9. Permissions (up to 30)
  const permissionMatches = html.matchAll(/class="b3x81">([^<]+)<\/div>/gi);
  const seenPermissions = new Set<string>();
  const permissionGroups: string[] = [];
  for (const pMatch of permissionMatches) {
    if (pMatch[1]) {
      const p = pMatch[1].trim();
      if (p && !seenPermissions.has(p)) {
        seenPermissions.add(p);
        permissionGroups.push(p);
        if (permissionGroups.length >= 30) break;
      }
    }
  }
  metadata.permissionGroups = permissionGroups;

  // 10. Screenshots (up to 12)
  const screenshots: PlayMediaAsset[] = [];
  const seenScreenshotUrls = new Set<string>();

  // Look for screenshot image URLs from ds callbacks or page markup
  const screenshotImgMatches = html.matchAll(
    /\[(?:null,\s*){1,2}\["(https:\/\/play-lh\.googleusercontent\.com\/[^"]+)",\s*(\d+),\s*(\d+)\]\]/gi,
  );
  for (const sMatch of screenshotImgMatches) {
    const rawUrl = sMatch[1];
    const width = parseInt(sMatch[2], 10) || null;
    const height = parseInt(sMatch[3], 10) || null;
    const asset = buildMediaAsset(rawUrl, width, height, 'Screenshot');
    if (asset && !seenScreenshotUrls.has(asset.sourceUrl)) {
      seenScreenshotUrls.add(asset.sourceUrl);
      screenshots.push(asset);
      if (screenshots.length >= 12) break;
    }
  }

  // Also check standard img tags with alt="Screenshot image"
  if (screenshots.length < 12) {
    const tagMatches = html.matchAll(
      /<img[^>]+src="(https:\/\/play-lh\.googleusercontent\.com\/[^"]+)"[^>]*alt="Screenshot[^"]*"/gi,
    );
    for (const tMatch of tagMatches) {
      const asset = buildMediaAsset(tMatch[1], null, null, 'Screenshot');
      if (asset && !seenScreenshotUrls.has(asset.sourceUrl)) {
        seenScreenshotUrls.add(asset.sourceUrl);
        screenshots.push(asset);
        if (screenshots.length >= 12) break;
      }
    }
  }
  metadata.screenshots = screenshots;

  // 11. Featured Reviews (first 3 reviews)
  const reviews: PlayReview[] = [];
  const reviewBlocks = html.matchAll(
    /<div[^>]+data-review-id="([^"]+)"[^>]*>([\s\S]*?)<\/div>\s*(?=<div[^>]+data-review-id=|<\/body>|$)/gi,
  );
  for (const rBlock of reviewBlocks) {
    if (reviews.length >= 3) break;
    const blockContent = rBlock[2];

    // Author
    const authorMatch = blockContent.match(/class="X5PpBb">([^<]+)<\/div>/i);
    const author =
      authorMatch && authorMatch[1]
        ? authorMatch[1].trim().slice(0, 100)
        : 'Google Play User';

    // Rating (1-5)
    let rating = 5;
    const ratingMatch = blockContent.match(
      /role="img"\s+aria-label="Rated\s+(\d+)\s+stars/i,
    );
    if (ratingMatch && ratingMatch[1]) {
      const parsedR = parseInt(ratingMatch[1], 10);
      if (parsedR >= 1 && parsedR <= 5) {
        rating = parsedR;
      }
    }

    // Date
    let date: string | null = null;
    const dateMatch = blockContent.match(/class="bp9Aid">([^<]+)<\/span>/i);
    if (dateMatch && dateMatch[1]) {
      date = dateMatch[1].trim().slice(0, 50);
    }

    // Review Text
    let text = '';
    const textMatch = blockContent.match(/class="h3YV2d">([\s\S]*?)<\/div>/i);
    if (textMatch && textMatch[1]) {
      text = htmlToPlainText(textMatch[1]).slice(0, 1500);
    }

    // Helpful count
    let helpfulCount = 0;
    const helpfulMatch = blockContent.match(
      /class="AJXJfe">(\d+)\s+people\s+found\s+this\s+helpful/i,
    );
    if (helpfulMatch && helpfulMatch[1]) {
      helpfulCount = parseInt(helpfulMatch[1], 10) || 0;
    }

    // Avatar
    let avatar: PlayMediaAsset | null = null;
    const avatarMatch = blockContent.match(
      /<header[^>]*>[\s\S]*?<img[^>]+src="([^"]+)"/i,
    );
    if (avatarMatch && avatarMatch[1]) {
      avatar = buildMediaAsset(avatarMatch[1], null, null, `${author} avatar`);
    }

    if (text) {
      reviews.push({
        author,
        rating,
        date,
        text,
        helpfulCount,
        avatar,
      });
    }
  }
  metadata.featuredReviews = reviews;

  return {
    playVersion,
    iconUrl,
    updatedAt,
    updatedOn,
    metadata,
  };
}

export async function fetchGooglePlayApp(
  packageName: string,
): Promise<ScrapedAppInfo> {
  const url = `https://play.google.com/store/apps/details?id=${encodeURIComponent(
    packageName,
  )}&hl=en&gl=US`;

  try {
    const res = await fetch(url, {
      headers: {
        'User-Agent': USER_AGENT,
        Accept:
          'text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8',
        'Accept-Language': 'en-US,en;q=0.9',
      },
    });

    if (res.status === 404) {
      return {
        playVersion: null,
        iconUrl: null,
        updatedAt: null,
        updatedOn: null,
        isNotFound: true,
      };
    }

    if (!res.ok) {
      return {
        playVersion: null,
        iconUrl: null,
        updatedAt: null,
        updatedOn: null,
        isNotFound: false,
        rawError: `HTTP ${res.status} ${res.statusText}`,
      };
    }

    const html = await res.text();
    const parsed = parseGooglePlayHtml(html);

    return {
      playVersion: parsed.playVersion,
      iconUrl: parsed.iconUrl,
      updatedAt: parsed.updatedAt,
      updatedOn: parsed.updatedOn,
      isNotFound: false,
      metadata: parsed.metadata,
    };
  } catch (err: unknown) {
    return {
      playVersion: null,
      iconUrl: null,
      updatedAt: null,
      updatedOn: null,
      isNotFound: false,
      rawError: err instanceof Error ? err.message : String(err),
    };
  }
}
