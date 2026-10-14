import type { PlayStoreMetadata } from './types';

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

// Multi-tier regex patterns for Google Play web store HTML
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

export function htmlToPlainText(html: string): string {
  if (!html) return '';
  // 1. Decode entities first so encoded tags (e.g. &lt;b&gt;) decode to tags
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

  // 2. Strip tags, preserving paragraph/break boundaries
  text = text
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/p>/gi, '\n\n')
    .replace(/<\/li>/gi, '\n')
    .replace(/<li[^>]*>/gi, '• ')
    .replace(/<\/h[1-6]>/gi, '\n\n')
    .replace(/<[^>]+>/g, '');
  // Normalize multi-newlines and whitespace
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
    iconUrl = ogMatch[1].replace(/&amp;/g, '&');
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

  // 4. Metadata extraction (ds:5 payload with fallback)
  const metadata: Partial<PlayStoreMetadata> = {};

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
    const response = await fetch(url, {
      signal: AbortSignal.timeout(10000),
      headers: {
        'User-Agent': USER_AGENT,
        Accept:
          'text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8',
        'Accept-Language': 'en-US,en;q=0.9',
      },
    });

    if (response.status === 404) {
      return {
        playVersion: null,
        iconUrl: null,
        updatedAt: null,
        updatedOn: null,
        isNotFound: true,
      };
    }

    if (!response.ok) {
      return {
        playVersion: null,
        iconUrl: null,
        updatedAt: null,
        updatedOn: null,
        isNotFound: false,
        rawError: `HTTP ${response.status}`,
      };
    }

    const html = await response.text();
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
    const message = err instanceof Error ? err.message : String(err);
    return {
      playVersion: null,
      iconUrl: null,
      updatedAt: null,
      updatedOn: null,
      isNotFound: false,
      rawError: message,
    };
  }
}
