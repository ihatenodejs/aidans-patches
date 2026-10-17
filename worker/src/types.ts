export type FreshnessStatus =
  | 'up-to-date'
  | 'newer-available'
  | 'check-failed'
  | 'not-on-play-store'
  | 'unknown';

export type PatchCompatibilityStatus =
  'queued' | 'running' | 'compatible' | 'incompatible' | 'error' | 'not-tested';

export type FinalPatchCompatibilityStatus =
  'compatible' | 'incompatible' | 'error';
export type FailureStage =
  | 'acquisition'
  | 'r2-lookup'
  | 'r2-upload'
  | 'r2-download'
  | 'compatibility-check'
  | 'pipeline';

export interface CompatibilityRecord {
  requestId: string;
  role: 'target';
  versionName: string;
  versionCode: number;
  patchBundleVersion: string;
  gitRevision: string;
  testedAt: string;
  passedCount: number;
  failedCount: number;
  status: PatchCompatibilityStatus;
  workflowRunUrl?: string | null;
  failureReason?: string | null;
  failureStage?: FailureStage | null;
}

export interface OutstandingCompatibilityRequest {
  requestId: string;
  targetVersion: string;
  playVersion: string | null;
  expectedRoles: 'target'[];
  dispatchedAt: string;
  gitRevision: string;
}

export interface CompatibilityRunStart {
  requestId: string;
  packageName: string;
  appName: string;
  targetVersion: string;
  supportedVersions: string[];
  gitRevision: string;
}
export interface PlayMediaAsset {
  sourceUrl: string;
  fallbackPath: string | null;
  width: number | null;
  height: number | null;
  alt: string | null;
}

export interface PlayReview {
  author: string;
  rating: number;
  date: string | null;
  text: string;
  helpfulCount: number;
  avatar: PlayMediaAsset | null;
}

export interface PlayStoreMetadata {
  summary: string | null;
  description: string | null;
  developer: string | null;
  category: string | null;
  categoryId: string | null;
  releaseNotes: string | null;
  installs: string | null;
  score: number | null;
  ratingsCount: number | null;
  reviewsCount: number | null;
  contentRating: string | null;
  contentRatingIcon: PlayMediaAsset | null;
  releasedOn: string | null;
  minAndroid: string | null;
  inAppPurchases: string | null;
  permissionGroups: string[];
  screenshots: PlayMediaAsset[];
  featuredReviews: PlayReview[];
}

export type PlayVersionSource = 'web' | 'authenticated' | null;

export interface AppVersionRecord {
  appName?: string;
  playVersion: string | null;
  playVersionCode?: number | null;
  playVersionSource?: PlayVersionSource;
  playVersionVerifiedForUpdatedAt?: string | null;
  needsAuthenticatedVersion?: boolean;
  iconUrl?: string | null;
  iconAsset?: PlayMediaAsset | null;
  updatedAt?: string | null;
  updatedOn?: string | null;
  checkedAt: string;
  status: FreshnessStatus;
  supportedVersions: string[];
  latestSupportedVersion: string;
  playVersionReleaseUpdatedAt?: string | null;
  targetCompatibility?: CompatibilityRecord | null;
  outstandingRequest?: OutstandingCompatibilityRequest | null;
  outstandingRequestId?: string | null;
  playMetadata?: PlayStoreMetadata | null;
}

export interface LatestReleaseSummary {
  packageName: string;
  appName: string;
  playVersion: string | null;
  iconUrl: string | null;
  updatedAt: string | null;
  updatedOn: string | null;
}

export interface KVVersionPayload {
  updatedAt: string;
  apps: Record<string, AppVersionRecord>;
  latestRelease?: LatestReleaseSummary | null;
}

export interface TargetAppConfig {
  packageName: string;
  name: string;
  apkFileType: string;
  signatures: string[];
  patchNames: string[];
  latestSupportedVersion: string;
  supportedVersions: string[];
}

export interface CompatibilityResultInput {
  role: 'target';
  versionName: string;
  versionCode: number;
  patchBundleVersion: string;
  gitRevision: string;
  status: FinalPatchCompatibilityStatus;
  passedCount: number;
  failedCount: number;
  workflowRunUrl?: string | null;
  failureReason?: string | null;
  failureStage?: FailureStage | null;
}

export interface CompatibilityResultSubmission {
  requestId: string;
  packageName: string;
  acquiredPlayVersion?: string | null;
  acquiredPlayVersionCode?: number | null;
  results: CompatibilityResultInput[];
}

export interface PlayMetadataResultItem {
  packageName: string;
  versionName: string;
  versionCode: number;
  listingUpdatedAt: string | null;
}

export interface PlayMetadataSubmission {
  results: PlayMetadataResultItem[];
}

export interface WorkerEnv extends Env {
  REFRESH_SECRET?: string;
  COMPATIBILITY_STATUS_SECRET?: string;
  GITHUB_TOKEN?: string;
}

export interface DownloadAssetRecord {
  releaseTag: string;
  downloadCount: number;
}

export interface DownloadSnapshot {
  date: string;
  totalDownloads: number;
}

export interface DownloadStatistics {
  updatedAt: string;
  assets: Record<string, DownloadAssetRecord>;
  history: DownloadSnapshot[];
}

export interface ReleaseLineDownloads {
  label: string;
  totalDownloads: number;
  major: number;
  minor: number;
}
