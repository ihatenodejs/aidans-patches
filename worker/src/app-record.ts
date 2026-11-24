import { compareAppVersions } from './comparator';
import { buildMediaAsset } from './scraper';
import type { ScrapedAppInfo } from './scraper';
import type {
  AppVersionRecord,
  FreshnessStatus,
  PlayMediaAsset,
  PlayReview,
  PlayStoreMetadata,
  PlayVersionSource,
  TargetAppConfig,
} from './types';

export const OUTSTANDING_REQUEST_TTL_MS = 36 * 60 * 60 * 1000; // 36 hours TTL (exceeds max 24h queue + 6h execution)

export function reconcileMediaAsset(
  newAsset: PlayMediaAsset | null | undefined,
  priorAsset: PlayMediaAsset | null | undefined,
): PlayMediaAsset | null {
  if (!newAsset) {
    return null;
  }
  if (
    priorAsset &&
    priorAsset.fallbackPath &&
    newAsset.sourceUrl === priorAsset.sourceUrl
  ) {
    return {
      ...newAsset,
      fallbackPath: newAsset.fallbackPath ?? priorAsset.fallbackPath,
    };
  }
  return newAsset;
}

export function reconcileMediaAssetsList(
  newAssets: PlayMediaAsset[],
  priorAssets: PlayMediaAsset[] | null | undefined,
): PlayMediaAsset[] {
  if (!priorAssets || priorAssets.length === 0) {
    return newAssets;
  }
  const priorMap = new Map<string, PlayMediaAsset>();
  for (const asset of priorAssets) {
    if (asset?.sourceUrl) {
      priorMap.set(asset.sourceUrl, asset);
    }
  }
  return newAssets.map((asset) => {
    const prior = priorMap.get(asset.sourceUrl);
    return reconcileMediaAsset(asset, prior) ?? asset;
  });
}

export function reconcileReviews(
  newReviews: PlayReview[],
  priorReviews: PlayReview[] | null | undefined,
): PlayReview[] {
  if (!priorReviews || priorReviews.length === 0) {
    return newReviews;
  }
  const priorAvatarMap = new Map<string, PlayMediaAsset>();
  for (const review of priorReviews) {
    if (review?.avatar?.sourceUrl) {
      priorAvatarMap.set(review.avatar.sourceUrl, review.avatar);
    }
  }
  return newReviews.map((review) => {
    if (!review.avatar) {
      return review;
    }
    const priorAvatar = priorAvatarMap.get(review.avatar.sourceUrl);
    return {
      ...review,
      avatar: reconcileMediaAsset(review.avatar, priorAvatar),
    };
  });
}

function applyMirroredFallback(
  asset: PlayMediaAsset | null | undefined,
  fallbackPaths: Record<string, string>,
): PlayMediaAsset | null {
  if (!asset) return null;
  const fallbackPath = asset.fallbackPath ?? fallbackPaths[asset.sourceUrl];
  return fallbackPath ? { ...asset, fallbackPath } : asset;
}

/**
 * Applies a scrape result to the current authoritative record.
 *
 * Only scrape-owned fields are replaced. Fields owned by compatibility
 * run-start/result callbacks (targetCompatibility, outstandingRequest and the
 * authenticated playVersion acquisition) are read from `prior` and preserved
 * unless the scrape itself proves the target version changed.
 */
export function mergeScrapedRecord(
  prior: AppVersionRecord | null,
  scraped: ScrapedAppInfo,
  app: TargetAppConfig,
  now: string,
  fallbackPaths: Record<string, string> = {},
): AppVersionRecord {
  let playVersion: string | null = null;
  let playVersionCode: number | null = prior?.playVersionCode ?? null;
  let playVersionSource: PlayVersionSource = prior?.playVersionSource ?? null;
  let playVersionVerifiedForUpdatedAt: string | null =
    prior?.playVersionVerifiedForUpdatedAt ?? null;
  let needsAuthenticatedVersion = false;

  let iconUrl = scraped.iconUrl || prior?.iconUrl || null;
  let iconAsset: PlayMediaAsset | null = scraped.iconUrl
    ? buildMediaAsset(scraped.iconUrl, null, null, `${app.name} icon`)
    : (prior?.iconAsset ?? null);
  iconAsset = reconcileMediaAsset(iconAsset, prior?.iconAsset);
  let updatedAt = scraped.updatedAt || prior?.updatedAt || null;
  let updatedOn = scraped.updatedOn || prior?.updatedOn || null;
  let status: FreshnessStatus;
  let playVersionReleaseUpdatedAt: string | null =
    prior?.playVersionReleaseUpdatedAt ?? null;

  if (scraped.isNotFound) {
    status = 'not-on-play-store';
    playVersion = null;
    playVersionCode = null;
    playVersionSource = null;
    playVersionReleaseUpdatedAt = null;
    playVersionVerifiedForUpdatedAt = null;
    needsAuthenticatedVersion = false;
  } else if (scraped.rawError) {
    status = 'check-failed';
    playVersion = prior?.playVersion ?? null;
    needsAuthenticatedVersion = prior?.needsAuthenticatedVersion ?? false;
  } else if (scraped.playVersion) {
    playVersion = scraped.playVersion;
    playVersionSource = 'web';
    status = compareAppVersions(app.latestSupportedVersion, playVersion);
    playVersionReleaseUpdatedAt = scraped.updatedAt ?? null;
    playVersionVerifiedForUpdatedAt = null;
    needsAuthenticatedVersion = false;
  } else {
    // App exists on Google Play but public HTML omits version
    const isVerifiedForCurrentUpdate =
      Boolean(scraped.updatedAt) &&
      prior?.playVersionVerifiedForUpdatedAt === scraped.updatedAt &&
      prior?.playVersionSource === 'authenticated';

    if (isVerifiedForCurrentUpdate && prior?.playVersion) {
      playVersion = prior.playVersion;
      playVersionCode = prior.playVersionCode ?? null;
      playVersionSource = 'authenticated';
      status = compareAppVersions(app.latestSupportedVersion, playVersion);
      needsAuthenticatedVersion = false;
    } else {
      // Retain prior version for display but mark freshness unknown and request authentication
      playVersion = prior?.playVersion ?? null;
      playVersionCode = prior?.playVersionCode ?? null;
      status = 'unknown';
      needsAuthenticatedVersion = true;
    }
  }

  let targetCompat = prior?.targetCompatibility ?? null;
  const targetVersionChanged = Boolean(
    prior && prior.latestSupportedVersion !== app.latestSupportedVersion,
  );
  if (targetVersionChanged) {
    targetCompat = null;
  }

  let outstandingRequest = prior?.outstandingRequest ?? null;
  if (
    outstandingRequest &&
    outstandingRequest.targetVersion !== app.latestSupportedVersion
  ) {
    outstandingRequest = null;
  }
  if (outstandingRequest) {
    const dispatchedTime = new Date(outstandingRequest.dispatchedAt).getTime();
    const isStaleOrInvalid =
      Number.isNaN(dispatchedTime) ||
      Date.now() - dispatchedTime > OUTSTANDING_REQUEST_TTL_MS;
    if (isStaleOrInvalid) {
      outstandingRequest = null;
    }
  }

  let mergedMetadata: PlayStoreMetadata | null = null;
  const priorMeta = prior?.playMetadata;

  if (scraped.isNotFound || scraped.rawError) {
    mergedMetadata = priorMeta ?? null;
  } else {
    const sMeta = scraped.metadata;
    mergedMetadata = {
      summary:
        sMeta?.summary !== undefined
          ? sMeta.summary
          : (priorMeta?.summary ?? null),
      description:
        sMeta?.description !== undefined
          ? sMeta.description
          : (priorMeta?.description ?? null),
      developer:
        sMeta?.developer !== undefined
          ? sMeta.developer
          : (priorMeta?.developer ?? null),
      category:
        sMeta?.category !== undefined
          ? sMeta.category
          : (priorMeta?.category ?? null),
      categoryId:
        sMeta?.categoryId !== undefined
          ? sMeta.categoryId
          : (priorMeta?.categoryId ?? null),
      releaseNotes:
        sMeta?.releaseNotes !== undefined
          ? sMeta.releaseNotes
          : (priorMeta?.releaseNotes ?? null),
      installs:
        sMeta?.installs !== undefined
          ? sMeta.installs
          : (priorMeta?.installs ?? null),
      score:
        sMeta?.score !== undefined ? sMeta.score : (priorMeta?.score ?? null),
      ratingsCount:
        sMeta?.ratingsCount !== undefined
          ? sMeta.ratingsCount
          : (priorMeta?.ratingsCount ?? null),
      reviewsCount:
        sMeta?.reviewsCount !== undefined
          ? sMeta.reviewsCount
          : (priorMeta?.reviewsCount ?? null),
      contentRating:
        sMeta?.contentRating !== undefined
          ? sMeta.contentRating
          : (priorMeta?.contentRating ?? null),
      contentRatingIcon:
        sMeta?.contentRatingIcon !== undefined
          ? reconcileMediaAsset(
              sMeta.contentRatingIcon,
              priorMeta?.contentRatingIcon,
            )
          : (priorMeta?.contentRatingIcon ?? null),
      releasedOn:
        sMeta?.releasedOn !== undefined
          ? sMeta.releasedOn
          : (priorMeta?.releasedOn ?? null),
      minAndroid:
        sMeta?.minAndroid !== undefined
          ? sMeta.minAndroid
          : (priorMeta?.minAndroid ?? null),
      inAppPurchases:
        sMeta?.inAppPurchases !== undefined
          ? sMeta.inAppPurchases
          : (priorMeta?.inAppPurchases ?? null),
      permissionGroups:
        sMeta?.permissionGroups && sMeta.permissionGroups.length > 0
          ? sMeta.permissionGroups
          : (priorMeta?.permissionGroups ?? []),
      screenshots:
        sMeta?.screenshots && sMeta.screenshots.length > 0
          ? reconcileMediaAssetsList(sMeta.screenshots, priorMeta?.screenshots)
          : (priorMeta?.screenshots ?? []),
      featuredReviews:
        sMeta?.featuredReviews && sMeta.featuredReviews.length > 0
          ? reconcileReviews(sMeta.featuredReviews, priorMeta?.featuredReviews)
          : (priorMeta?.featuredReviews ?? []),
    };
  }

  iconAsset = applyMirroredFallback(iconAsset, fallbackPaths);
  if (mergedMetadata) {
    mergedMetadata = {
      ...mergedMetadata,
      contentRatingIcon: applyMirroredFallback(
        mergedMetadata.contentRatingIcon,
        fallbackPaths,
      ),
      screenshots: mergedMetadata.screenshots.map(
        (asset) => applyMirroredFallback(asset, fallbackPaths) ?? asset,
      ),
      featuredReviews: mergedMetadata.featuredReviews.map((review) =>
        review.avatar
          ? {
              ...review,
              avatar:
                applyMirroredFallback(review.avatar, fallbackPaths) ??
                review.avatar,
            }
          : review,
      ),
    };
  }

  return {
    appName: app.name,
    playVersion,
    playVersionCode,
    playVersionSource,
    playVersionVerifiedForUpdatedAt,
    needsAuthenticatedVersion,
    iconUrl,
    iconAsset,
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
    playMetadata: mergedMetadata,
  };
}
