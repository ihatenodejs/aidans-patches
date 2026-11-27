import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { semver } from 'bun';
import type { DependencyItem } from './model';
export type FetchFn = (url: string, init?: RequestInit) => Promise<Response>;

export interface SourceResolverOptions {
  fetch?: FetchFn;
  githubToken?: string;
  githubActor?: string;
  npmRegistry?: string;
}

export interface GitHubReleaseAsset {
  name: string;
  url: string;
  sha256?: string;
}

export interface GitHubReleaseInfo {
  version: string;
  tag: string;
  sourceUrl: string;
  assets: GitHubReleaseAsset[];
}

/** Maps an Android NDK revision to its rXXx release label. */
export function ndkLabelFromRevision(revision: string): string {
  const [major, minor] = revision.split('.').map((part) => Number(part));
  if (!Number.isInteger(major) || major <= 0) return '';
  if (!Number.isInteger(minor) || minor <= 0) return `r${major}`;
  // NDK labels skip the "a" suffix: 26.1 -> r26b, 28.2 -> r28c.
  return `r${major}${String.fromCharCode(97 + minor)}`;
}

/** Replaces both a raw and regex-escaped version string in text. */
export function replaceVersionText(
  text: string,
  from: string,
  to: string,
): string {
  if (!from) return text;
  const escape = (value: string) =>
    value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return text.split(escape(from)).join(escape(to)).split(from).join(to);
}

function sha256FromDigest(digest: string | undefined): string | undefined {
  if (!digest) return undefined;
  const match = digest.match(/(?:sha256:)?([a-f0-9]{64})/i);
  return match?.[1];
}

const APK_TOOL_SOURCES: Record<
  string,
  { owner: string; repo: string; tagPrefix?: string }
> = {
  morphe: { owner: 'MorpheApp', repo: 'morphe-desktop', tagPrefix: 'v' },
  jadx: { owner: 'skylot', repo: 'jadx', tagPrefix: 'v' },
  apktool: { owner: 'iBotPeaches', repo: 'Apktool', tagPrefix: 'v' },
  baksmali: { owner: 'baksmali', repo: 'smali' },
  smali: { owner: 'baksmali', repo: 'smali' },
  apkeep: { owner: 'EFForg', repo: 'apkeep' },
};

/**
 * Normalizes package-manager version strings to a full semver triple.
 *
 * `semver.order('7.0.1', '7')` treats `7` as greater than `7.0.1`, which made
 * floating major action pins and Cargo declarations such as `1.0` look
 * up-to-date or ahead. Pad omitted components before comparing instead.
 */
export function normalizeSemver(value: string): string | null {
  const clean = value.replace(/^[v^~=><\s]+/, '').trim();
  const match = clean.match(
    /^(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?/,
  );
  if (!match) return null;
  const [, major, minor = '0', patch = '0', prerelease] = match;
  return `${major}.${minor}.${patch}${prerelease ? `-${prerelease}` : ''}`;
}

export function compareSemver(vA: string, vB: string): number {
  const cleanA = vA.replace(/^[v^~=>< ]+/, '').trim();
  const cleanB = vB.replace(/^[v^~=>< ]+/, '').trim();
  const normalizedA = normalizeSemver(cleanA);
  const normalizedB = normalizeSemver(cleanB);
  if (normalizedA && normalizedB) {
    try {
      return semver.order(normalizedA, normalizedB);
    } catch {
      // Fall through to a numeric-aware string comparison.
    }
  }
  return cleanA.localeCompare(cleanB, undefined, { numeric: true });
}

export function loadGradleCredentials(): { user?: string; key?: string } {
  const candidates = [
    path.join(process.cwd(), 'local.properties'),
    path.join(process.cwd(), 'gradle.properties'),
    path.join(os.homedir(), '.gradle', 'gradle.properties'),
  ];
  let user: string | undefined;
  let key: string | undefined;
  for (const c of candidates) {
    if (fs.existsSync(c)) {
      try {
        const text = fs.readFileSync(c, 'utf8');
        for (const line of text.split('\n')) {
          if (line.startsWith('gpr.user='))
            user = user || line.split('=')[1].trim();
          if (line.startsWith('gpr.key='))
            key = key || line.split('=')[1].trim();
        }
      } catch {
        // Ignore file read error
      }
    }
  }
  return { user, key };
}

export class SourceManager {
  private fetch: FetchFn;
  private githubToken?: string;
  private githubActor?: string;
  private npmRegistry: string;
  private cache = new Map<string, Promise<unknown>>();
  private inFlightCount = 0;
  private queue: Array<() => void> = [];
  private static MAX_CONCURRENCY = 6;

  constructor(opts: SourceResolverOptions = {}) {
    this.fetch = opts.fetch || globalThis.fetch;
    const gpr = loadGradleCredentials();
    this.githubToken = opts.githubToken || process.env.GITHUB_TOKEN || gpr.key;
    this.githubActor =
      opts.githubActor ||
      process.env.GITHUB_ACTOR ||
      process.env.GPR_USER ||
      gpr.user ||
      'ihatenodejs';
    this.npmRegistry = (
      opts.npmRegistry ||
      process.env.npm_config_registry ||
      'https://registry.npmjs.org'
    ).replace(/\/$/, '');
  }

  private async schedule<T>(task: () => Promise<T>): Promise<T> {
    if (this.inFlightCount >= SourceManager.MAX_CONCURRENCY) {
      const { promise, resolve } = Promise.withResolvers<void>();
      this.queue.push(resolve);
      await promise;
    }
    this.inFlightCount++;
    try {
      return await task();
    } finally {
      this.inFlightCount--;
      const next = this.queue.shift();
      if (next) next();
    }
  }

  private fetchJson<T>(
    url: string,
    headers: Record<string, string> = {},
  ): Promise<T> {
    const cacheKey = `json:${url}`;
    if (this.cache.has(cacheKey)) {
      return this.cache.get(cacheKey) as Promise<T>;
    }
    const prom = this.schedule(async () => {
      const res = await this.fetch(url, {
        headers: {
          'User-Agent': 'aidans-patches-deps/1.0',
          ...headers,
        },
        signal: AbortSignal.timeout(20000),
      });
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} ${res.statusText} fetching ${url}`);
      }
      return (await res.json()) as T;
    });
    this.cache.set(cacheKey, prom);
    return prom;
  }

  private fetchText(
    url: string,
    headers: Record<string, string> = {},
  ): Promise<string> {
    const cacheKey = `text:${url}`;
    if (this.cache.has(cacheKey)) {
      return this.cache.get(cacheKey) as Promise<string>;
    }
    const prom = this.schedule(async () => {
      const res = await this.fetch(url, {
        headers: {
          'User-Agent': 'aidans-patches-deps/1.0',
          ...headers,
        },
        signal: AbortSignal.timeout(20000),
      });
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} ${res.statusText} fetching ${url}`);
      }
      return await res.text();
    });
    this.cache.set(cacheKey, prom);
    return prom;
  }

  // --- NPM ---
  async resolveNpm(
    name: string,
  ): Promise<{ latest: string; sourceUrl: string }> {
    const encoded = name.startsWith('@')
      ? `@${encodeURIComponent(name.slice(1))}`
      : encodeURIComponent(name);
    const url = `${this.npmRegistry}/${encoded}`;
    const data = await this.fetchJson<{
      'dist-tags'?: Record<string, string>;
      versions?: Record<string, unknown>;
    }>(url);
    const distTags = data['dist-tags'] || {};
    let latest = distTags.latest;
    if (!latest || latest.includes('-')) {
      const versions = Object.keys(data.versions || {}).filter(
        (v) => !v.includes('-'),
      );
      if (versions.length > 0) {
        versions.sort(compareSemver);
        latest = versions[versions.length - 1];
      }
    }
    if (!latest) {
      throw new Error(`No stable release found for npm package ${name}`);
    }
    return { latest, sourceUrl: `https://www.npmjs.com/package/${name}` };
  }

  // --- GitHub Branch (e.g. @MorpheApp/changelog #bundle) ---
  async resolveGitBranch(
    owner: string,
    repo: string,
    branch: string,
  ): Promise<{ latest: string; sourceUrl: string }> {
    const url = `https://api.github.com/repos/${owner}/${repo}/commits/${branch}`;
    const headers: Record<string, string> = {
      Accept: 'application/vnd.github.v3+json',
    };
    if (this.githubToken) headers.Authorization = `Bearer ${this.githubToken}`;
    const data = await this.fetchJson<{ sha?: string }>(url, headers);
    const sha = data.sha;
    if (!sha)
      throw new Error(
        `Could not resolve commit for branch ${branch} on ${owner}/${repo}`,
      );
    return {
      latest: sha,
      sourceUrl: `https://github.com/${owner}/${repo}/tree/${branch}`,
    };
  }

  // --- Crates.io ---
  async resolveCrates(
    name: string,
  ): Promise<{ latest: string; sourceUrl: string }> {
    const url = `https://crates.io/api/v1/crates/${name}`;
    const data = await this.fetchJson<{
      versions?: Array<{ num: string; yanked: boolean }>;
    }>(url);
    const versions = data.versions || [];
    const stables = versions
      .filter((v) => !v.yanked && !v.num.includes('-'))
      .map((v) => v.num);
    if (stables.length === 0) {
      throw new Error(`No non-yanked stable releases found for crate ${name}`);
    }
    stables.sort(compareSemver);
    return {
      latest: stables[stables.length - 1],
      sourceUrl: `https://crates.io/crates/${name}`,
    };
  }

  // --- Maven (Central or GitHub Packages) ---
  async resolveMaven(
    group: string,
    artifact: string,
    repoUrl?: string,
  ): Promise<{ latest: string; sourceUrl: string }> {
    const groupPath = group.replace(/\./g, '/');
    let metadataUrl: string;
    const headers: Record<string, string> = {};

    if (repoUrl && repoUrl.includes('maven.pkg.github.com')) {
      if (!this.githubToken) {
        throw new Error(
          `GitHub Packages authentication required for ${group}:${artifact} (GITHUB_TOKEN/gpr.key not set)`,
        );
      }
      const token = Buffer.from(
        `${this.githubActor || 'token'}:${this.githubToken}`,
      ).toString('base64');
      headers.Authorization = `Basic ${token}`;
      metadataUrl = `${repoUrl.replace(/\/$/, '')}/${groupPath}/${artifact}/maven-metadata.xml`;
    } else {
      metadataUrl = `https://repo1.maven.org/maven2/${groupPath}/${artifact}/maven-metadata.xml`;
    }

    const xml = await this.fetchText(metadataUrl, headers);
    const releaseMatch = xml.match(/<release>([^<]+)<\/release>/);
    let chosen = releaseMatch ? releaseMatch[1] : null;

    if (
      !chosen ||
      chosen.includes('-SNAPSHOT') ||
      chosen.includes('-alpha') ||
      chosen.includes('-beta')
    ) {
      const versionMatches = [
        ...xml.matchAll(/<version>([^<]+)<\/version>/g),
      ].map((m) => m[1]);
      const stables = versionMatches.filter(
        (v) =>
          !v.includes('SNAPSHOT') &&
          !v.includes('alpha') &&
          !v.includes('beta') &&
          !v.includes('rc'),
      );
      if (stables.length > 0) {
        stables.sort(compareSemver);
        chosen = stables[stables.length - 1];
      }
    }

    if (!chosen) {
      throw new Error(
        `Could not parse stable release from maven-metadata.xml at ${metadataUrl}`,
      );
    }
    return { latest: chosen, sourceUrl: metadataUrl };
  }

  // --- GitHub Releases / Tags ---
  async resolveGitHubRelease(
    owner: string,
    repo: string,
    tagPrefix = '',
  ): Promise<{ latest: string; sourceUrl: string }> {
    const url = `https://api.github.com/repos/${owner}/${repo}/releases`;
    const headers: Record<string, string> = {
      Accept: 'application/vnd.github.v3+json',
    };
    if (this.githubToken) headers.Authorization = `Bearer ${this.githubToken}`;

    try {
      const releases = await this.fetchJson<
        Array<{ tag_name: string; draft: boolean; prerelease: boolean }>
      >(url, headers);
      const stableReleases = releases.filter(
        (r) =>
          !r.draft &&
          !r.prerelease &&
          (!tagPrefix || r.tag_name.startsWith(tagPrefix)),
      );
      if (stableReleases.length > 0) {
        const tag = stableReleases[0].tag_name;
        const ver = tag
          .replace(new RegExp(`^${tagPrefix}`), '')
          .replace(/^v/, '');
        return {
          latest: ver,
          sourceUrl: `https://github.com/${owner}/${repo}/releases/tag/${tag}`,
        };
      }
    } catch {
      // Fallback to tags
    }

    // Fallback: list tags
    const tagsUrl = `https://api.github.com/repos/${owner}/${repo}/tags`;
    const tags = await this.fetchJson<Array<{ name: string }>>(
      tagsUrl,
      headers,
    );
    const validTags = tags
      .map((t) => t.name as string)
      .filter(
        (t) =>
          !t.includes('alpha') &&
          !t.includes('beta') &&
          !t.includes('rc') &&
          (!tagPrefix || t.startsWith(tagPrefix)),
      );
    if (validTags.length === 0) {
      throw new Error(`No stable tags found for ${owner}/${repo}`);
    }
    validTags.sort(compareSemver);
    const newestTag = validTags[validTags.length - 1];
    const ver = newestTag
      .replace(new RegExp(`^${tagPrefix}`), '')
      .replace(/^v/, '');
    return {
      latest: ver,
      sourceUrl: `https://github.com/${owner}/${repo}/releases/tag/${newestTag}`,
    };
  }

  /** Stable GitHub release including downloadable assets and digests. */
  async resolveGitHubReleaseInfo(
    owner: string,
    repo: string,
    tagPrefix = '',
  ): Promise<GitHubReleaseInfo> {
    const url = `https://api.github.com/repos/${owner}/${repo}/releases`;
    const headers: Record<string, string> = {
      Accept: 'application/vnd.github.v3+json',
    };
    if (this.githubToken) headers.Authorization = `Bearer ${this.githubToken}`;
    const releases = await this.fetchJson<
      Array<{
        tag_name: string;
        draft: boolean;
        prerelease: boolean;
        assets?: Array<{
          name: string;
          browser_download_url: string;
          digest?: string | null;
        }>;
      }>
    >(url, headers);
    const release = releases.find(
      (r) =>
        !r.draft &&
        !r.prerelease &&
        (!tagPrefix || r.tag_name.startsWith(tagPrefix)),
    );
    if (!release) {
      throw new Error(
        `No stable GitHub release found for ${owner}/${repo} with prefix '${tagPrefix}'`,
      );
    }
    const version = release.tag_name
      .replace(new RegExp(`^${tagPrefix}`), '')
      .replace(/^v/, '');
    return {
      version,
      tag: release.tag_name,
      sourceUrl: `https://github.com/${owner}/${repo}/releases/tag/${release.tag_name}`,
      assets: (release.assets ?? []).map((asset) => ({
        name: asset.name,
        url: asset.browser_download_url,
        sha256: sha256FromDigest(asset.digest ?? undefined),
      })),
    };
  }

  // --- Gradle Wrapper ---
  async resolveGradleWrapper(): Promise<{
    latest: string;
    sha256: string;
    sourceUrl: string;
  }> {
    const url = 'https://services.gradle.org/versions/current';
    const data = await this.fetchJson<{ version: string }>(url);
    const version = data.version;
    const shaUrl = `https://services.gradle.org/distributions/gradle-${version}-bin.zip.sha256`;
    const sha256 = (await this.fetchText(shaUrl)).trim();
    return {
      latest: version,
      sha256,
      sourceUrl: `https://gradle.org/releases/`,
    };
  }

  // --- Rust Toolchain ---
  async resolveRustStable(): Promise<{ latest: string; sourceUrl: string }> {
    const url = 'https://static.rust-lang.org/dist/channel-rust-stable.toml';
    const text = await this.fetchText(url);
    const m =
      text.match(/#\s*Rust\s*([0-9.]+)/) ||
      text.match(/pkg\.rust\.version\s*=\s*"([0-9.]+)/);
    const ver = m ? m[1] : '1.85.0';
    return {
      latest: ver,
      sourceUrl: 'https://github.com/rust-lang/rust/releases',
    };
  }

  // --- Adoptium Java ---
  async resolveJavaAdoptium(): Promise<{ latest: string; sourceUrl: string }> {
    const url = 'https://api.adoptium.net/v3/info/available_releases';
    const data = await this.fetchJson<{ available_lts_releases?: number[] }>(
      url,
    );
    const ga = data.available_lts_releases || [21, 25];
    const latestLts = String(Math.max(...ga));
    return {
      latest: latestLts,
      sourceUrl: 'https://adoptium.net/temurin/releases/',
    };
  }

  // --- Node LTS ---
  async resolveNodeLts(): Promise<{ latest: string; sourceUrl: string }> {
    const url = 'https://nodejs.org/dist/index.json';
    const releases =
      await this.fetchJson<Array<{ version?: string; lts: boolean | string }>>(
        url,
      );
    const ltsReleases = releases.filter((r) => Boolean(r.lts));
    const latestLts = ltsReleases[0]?.version?.replace(/^v/, '') || '22.14.0';
    return {
      latest: latestLts,
      sourceUrl: 'https://nodejs.org/en/about/previous-releases',
    };
  }

  // --- Bun Latest ---
  async resolveBunLatest(): Promise<{ latest: string; sourceUrl: string }> {
    const rel = await this.resolveGitHubRelease('oven-sh', 'bun', 'bun-v');
    return { latest: rel.latest, sourceUrl: rel.sourceUrl };
  }

  // --- ktlint ---
  async resolveKtlint(): Promise<{
    latest: string;
    sha256?: string;
    assetUrl?: string;
    sourceUrl: string;
  }> {
    const release = await this.resolveGitHubReleaseInfo('pinterest', 'ktlint');
    const asset = release.assets.find(
      (candidate) => candidate.name === 'ktlint',
    );
    if (!asset) {
      throw new Error(
        `ktlint release ${release.tag} does not publish the expected 'ktlint' asset`,
      );
    }
    return {
      latest: release.version,
      sha256: asset.sha256,
      assetUrl: asset.url,
      sourceUrl: release.sourceUrl,
    };
  }

  private async resolveAndroidRepositoryPackages(): Promise<
    Array<{ path: string; revision: string }>
  > {
    const xml = await this.fetchText(
      'https://dl.google.com/android/repository/repository2-3.xml',
    );
    const packages: Array<{ path: string; revision: string }> = [];
    const packageRegex =
      /<remotePackage path="([^"]+)">([\s\S]*?)<\/remotePackage>/g;
    for (const match of xml.matchAll(packageRegex)) {
      const [, packagePath, body] = match;
      // channel-0 is the stable channel; previews live in later channels.
      if (!body.includes('channelRef ref="channel-0"')) continue;
      const revision = body.match(
        /<revision>\s*<major>(\d+)<\/major>\s*<minor>(\d+)<\/minor>\s*<micro>(\d+)<\/micro>/,
      );
      if (!revision) continue;
      packages.push({
        path: packagePath,
        revision: `${revision[1]}.${revision[2]}.${revision[3]}`,
      });
    }
    return packages;
  }

  // --- NDK Latest ---
  async resolveNdk(): Promise<{
    label: string;
    revision: string;
    sourceUrl: string;
  }> {
    const packages = (await this.resolveAndroidRepositoryPackages())
      .filter((entry) => entry.path.startsWith('ndk;'))
      .sort((a, b) => compareSemver(a.revision, b.revision));
    const latest = packages.at(-1);
    if (!latest) {
      throw new Error(
        'Could not find a stable NDK release in the SDK repository',
      );
    }
    return {
      label: ndkLabelFromRevision(latest.revision),
      revision: latest.revision,
      sourceUrl: 'https://developer.android.com/ndk/downloads',
    };
  }

  // --- Android Build Tools ---
  async resolveAndroidBuildTools(): Promise<{
    latest: string;
    sourceUrl: string;
  }> {
    const packages = (await this.resolveAndroidRepositoryPackages())
      .filter((entry) => entry.path.startsWith('build-tools;'))
      .sort((a, b) => compareSemver(a.revision, b.revision));
    const latest = packages.at(-1);
    if (!latest) {
      throw new Error(
        'Could not find a stable Android build-tools release in the SDK repository',
      );
    }
    return {
      latest: latest.revision,
      sourceUrl: 'https://developer.android.com/tools/releases/build-tools',
    };
  }
}

export async function resolveDependencyItem(
  item: DependencyItem,
  sources: SourceManager,
): Promise<void> {
  const start = performance.now();
  try {
    // 1. JavaScript (NPM)
    if (item.ecosystem === 'npm') {
      if (item.declared.startsWith('git+')) {
        const branchMatch = item.declared.match(/#([a-zA-Z0-9_-]+)$/);
        const branch = branchMatch ? branchMatch[1] : 'bundle';
        const res = await sources.resolveGitBranch(
          'MorpheApp',
          'changelog',
          branch,
        );
        item.latest = res.latest;
        item.latestDisplay = `commit ${res.latest.slice(0, 8)}`;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status = 'moving';
          item.reason = `Tracks branch '${branch}'; refresh the lockfile with the package manager after reviewing the new commit`;
        }
      } else if (item.declared === 'latest') {
        const res = await sources.resolveNpm(item.name);
        item.latest = res.latest;
        item.latestDisplay = `latest (${res.latest})`;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') item.status = 'moving';
      } else {
        const res = await sources.resolveNpm(item.name);
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          const compDeclared = compareSemver(res.latest, item.declared);
          const compResolved = item.resolved
            ? compareSemver(res.latest, item.resolved)
            : compDeclared;
          if (compDeclared > 0 || compResolved > 0) {
            item.status = 'update';
          } else if (compDeclared < 0) {
            item.status = 'ahead';
          } else {
            item.status = 'current';
          }
        }
      }
    }

    // 2. Rust (crates.io)
    else if (item.ecosystem === 'crates') {
      const res = await sources.resolveCrates(item.name);
      item.latest = res.latest;
      item.sourceUrl = res.sourceUrl;
      if (item.status !== 'held') {
        const comp = compareSemver(res.latest, item.declared);
        if (comp > 0) {
          item.status = 'update';
        } else if (comp < 0) {
          item.status = 'ahead';
        } else {
          item.status = 'current';
        }
      }
    }

    // 3. Gradle (Morphe repo or Maven Central)
    else if (item.ecosystem === 'gradle') {
      if (item.status === 'unused') {
        // Leave unused entries as is
      } else if (item.name.includes('morphe')) {
        const res = await sources.resolveMaven(
          item.name === 'app.morphe.patches'
            ? 'app.morphe.patches'
            : 'app.morphe',
          item.name === 'app.morphe.patches'
            ? 'app.morphe.patches.gradle.plugin'
            : 'morphe-patcher',
          'https://maven.pkg.github.com/MorpheApp/registry',
        );
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
        }
      } else if (item.name.includes(':')) {
        const [group, artifact] = item.name.split(':');
        const res = await sources.resolveMaven(group, artifact);
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          const ord = compareSemver(res.latest, item.declared);
          item.status = ord > 0 ? 'update' : ord < 0 ? 'ahead' : 'current';
        }
      }
    }

    // 4. GitHub Actions
    else if (item.ecosystem === 'action') {
      const [owner, repo] = item.name.split('/');
      const res = await sources.resolveGitHubRelease(owner, repo);
      item.latest = res.latest;
      item.sourceUrl = res.sourceUrl;
      if (item.status !== 'held') {
        const cleanDeclared = item.declared.replace(/^v/, '');
        const cleanLatest = res.latest.replace(/^v/, '');
        item.status =
          compareSemver(cleanLatest, cleanDeclared) > 0 ? 'update' : 'current';
      }
    }

    // 5. Tools & Toolchains
    else if (item.ecosystem === 'tool') {
      if (item.id === 'tool:gradle') {
        const res = await sources.resolveGradleWrapper();
        item.latest = res.latest;
        item.latestDisplay = `${res.latest} (${res.sha256.slice(0, 8)}...)`;
        item.latestAsset = res.sha256;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
        }
      } else if (item.id === 'tool:ktlint') {
        const res = await sources.resolveKtlint();
        item.latest = res.latest;
        item.latestAsset = res.sha256;
        item.latestUrl = res.assetUrl;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
        }
      } else if (item.id.startsWith('tool:apk-lab/')) {
        const toolName = item.name;
        const target = APK_TOOL_SOURCES[toolName];
        if (!target) {
          throw new Error(
            `No GitHub release source registered for APK Lab tool '${toolName}'`,
          );
        }
        const res = await sources.resolveGitHubReleaseInfo(
          target.owner,
          target.repo,
          target.tagPrefix ?? '',
        );
        item.latest = res.version;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          if (compareSemver(res.version, item.declared) > 0) {
            const candidateUrl = replaceVersionText(
              item.latestUrl ?? '',
              item.declared,
              res.version,
            );
            const candidateName = candidateUrl.split('/').pop() ?? '';
            const asset =
              res.assets.find((entry) => entry.url === candidateUrl) ??
              res.assets.find((entry) => entry.name === candidateName);
            if (!asset) {
              throw new Error(
                `Release ${res.tag} for ${toolName} has no asset matching '${candidateName || candidateUrl}'`,
              );
            }
            item.latestUrl = asset.url;
            item.latestAsset = asset.sha256;
            item.latestExecutablePath = replaceVersionText(
              item.latestExecutablePath ?? '',
              item.declared,
              res.version,
            );
            item.latestProbePattern = replaceVersionText(
              item.latestProbePattern ?? '',
              item.declared,
              res.version,
            );
            item.status = 'update';
          } else {
            item.status = 'current';
          }
        }
      } else if (item.id === 'tool:rust-native') {
        const res = await sources.resolveRustStable();
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
        }
      } else if (item.id === 'tool:java') {
        const res = await sources.resolveJavaAdoptium();
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            parseInt(res.latest, 10) > parseInt(item.declared, 10)
              ? 'update'
              : 'current';
        }
      } else if (item.id === 'tool:bun') {
        const res = await sources.resolveBunLatest();
        item.latest = res.latest;
        item.latestDisplay = `latest (${res.latest})`;
        item.sourceUrl = res.sourceUrl;
        item.status = 'moving';
      } else if (item.id === 'tool:node') {
        const res = await sources.resolveNodeLts();
        item.latest = res.latest.replace(/^lts\/\* \(([^)]+)\)$/, '$1');
        item.latestDisplay = res.latest.startsWith('lts/*')
          ? res.latest
          : `lts/* (${res.latest})`;
        item.sourceUrl = res.sourceUrl;
        item.status =
          res.latest === item.declared ||
          res.latest.startsWith(`${item.declared}.`)
            ? 'current'
            : 'moving';
      } else if (item.id === 'tool:ndk') {
        const res = await sources.resolveNdk();
        item.latest = res.revision;
        item.latestDisplay = `${res.label} (${res.revision})`;
        item.latestLabel = res.label;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          const declaredRevision =
            item.declared.match(/(\d+\.\d+\.\d+)/)?.[1] ?? item.declared;
          const declaredLabel = item.declared.match(/r\d+[a-z]?/)?.[0];
          item.status =
            declaredRevision === res.revision && declaredLabel === res.label
              ? 'current'
              : 'update';
        }
      } else if (item.id === 'tool:android-build-tools') {
        const res = await sources.resolveAndroidBuildTools();
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          item.status =
            compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
        }
      }
    }

    // 6. Embedded Leaflet
    else if (item.ecosystem === 'embedded' && item.name === 'leaflet') {
      const res = await sources.resolveNpm('leaflet');
      item.latest = res.latest;
      item.sourceUrl = res.sourceUrl;
      if (item.status !== 'held') {
        item.status =
          compareSemver(res.latest, item.declared) > 0 ? 'update' : 'current';
      }
    }

    // 7. C++ CMake FetchContent
    else if (item.ecosystem === 'cmake') {
      let res: { latest: string; sourceUrl: string } | null = null;
      if (item.name === 'libjpeg-turbo') {
        res = await sources.resolveGitHubRelease(
          'libjpeg-turbo',
          'libjpeg-turbo',
        );
      } else if (item.name === 'qpdf') {
        res = await sources.resolveGitHubRelease('qpdf', 'qpdf', 'v');
      } else if (item.name === 'leptonica') {
        res = await sources.resolveGitHubRelease('DanBloomberg', 'leptonica');
      } else if (item.name === 'tesseract') {
        res = await sources.resolveGitHubRelease('tesseract-ocr', 'tesseract');
      } else if (item.sourceUrl) {
        const ghMatch = item.sourceUrl.match(/github\.com\/([^/]+)\/([^/]+)/);
        if (ghMatch) {
          const owner = ghMatch[1];
          const repo = ghMatch[2];
          const hasVPrefix = /\/v[0-9]/.test(item.sourceUrl);
          res = await sources.resolveGitHubRelease(
            owner,
            repo,
            hasVPrefix ? 'v' : '',
          );
        }
      }
      if (res) {
        item.latest = res.latest;
        item.sourceUrl = res.sourceUrl;
        if (item.status !== 'held') {
          const ord = compareSemver(res.latest, item.declared);
          item.status = ord > 0 ? 'update' : ord < 0 ? 'ahead' : 'current';
        }
      }
    }
  } catch (err) {
    item.status = 'error';
    item.reason = (err as Error).message;
  } finally {
    item.durationMs = Math.round(performance.now() - start);
  }
}
