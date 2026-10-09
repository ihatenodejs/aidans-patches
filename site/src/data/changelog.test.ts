import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  extractLatestStableFromChangelog,
  formatDisplayDate,
  isPreRelease,
  resolveLatestReleaseSync,
  resolveLatestRelease,
  fetchLatestGitHubRelease,
} from './changelog';
import type { RawBundle } from './types';

describe('isPreRelease', () => {
  it('identifies pre-release versions with dev/alpha/beta/rc tags', () => {
    assert.equal(isPreRelease('1.4.0-dev.7'), true);
    assert.equal(isPreRelease('v1.4.0-dev.7'), true);
    assert.equal(isPreRelease('1.3.1-dev.1'), true);
    assert.equal(isPreRelease('2.0.0-rc.1'), true);
    assert.equal(isPreRelease('1.0.0-alpha.2'), true);
    assert.equal(isPreRelease('1.0.0-beta'), true);
  });

  it('identifies stable versions without tags', () => {
    assert.equal(isPreRelease('1.4.0'), false);
    assert.equal(isPreRelease('v1.4.0'), false);
    assert.equal(isPreRelease('1.3.1'), false);
    assert.equal(isPreRelease('1.0.0'), false);
    assert.equal(isPreRelease('0.9.5'), false);
  });

  it('treats empty or invalid versions as pre-release for safety', () => {
    assert.equal(isPreRelease(''), true);
    assert.equal(isPreRelease('unknown'), true);
    assert.equal(isPreRelease('custom-build'), true);
  });
});

describe('formatDisplayDate', () => {
  it('formats YYYY-MM-DD correctly', () => {
    assert.equal(formatDisplayDate('2026-10-08'), 'Oct 8, 2026');
    assert.equal(formatDisplayDate('2026-01-15'), 'Jan 15, 2026');
  });

  it('formats ISO timestamps correctly', () => {
    assert.equal(formatDisplayDate('2026-10-08T20:27:35Z'), 'Oct 8, 2026');
    assert.equal(formatDisplayDate('2026-05-04T12:00:00'), 'May 4, 2026');
  });

  it('handles empty or malformed dates gracefully', () => {
    assert.equal(formatDisplayDate(''), 'Recent');
    assert.equal(formatDisplayDate('invalid-date'), 'invalid-date');
  });
});

describe('extractLatestStableFromChangelog', () => {
  it('extracts stable release at top of changelog', () => {
    const markdown = `## [1.4.0](https://github.com/owner/repo/compare/v1.3.1...v1.4.0) (2026-10-08)

### 🐛 Bug Fixes

* **core:** fix bug ([abc1234](https://github.com/owner/repo/commit/abc1234))

## [1.3.1](https://github.com/owner/repo/compare/v1.3.0...v1.3.1) (2026-10-06)
`;
    const result = extractLatestStableFromChangelog(markdown);
    assert.ok(result !== null);
    assert.equal(result?.version, '1.4.0');
    assert.equal(result?.date, '2026-10-08');
    assert.ok(result?.body.includes('**core:** fix bug'));
    assert.ok(!result?.body.includes('1.3.1'));
  });

  it('skips multiple pre-releases at top of changelog to find the latest stable release', () => {
    const markdown = `## [1.4.0-dev.7](https://github.com/owner/repo/compare/v1.4.0-dev.6...v1.4.0-dev.7) (2026-10-08)

### 🐛 Bug Fixes

* **test:** dev fix ([1111111](https://github.com/owner/repo/commit/1111111))

## [1.4.0-dev.6](https://github.com/owner/repo/compare/v1.4.0-dev.5...v1.4.0-dev.6) (2026-10-08)

### ✨ New Features

* **test:** dev feat ([2222222](https://github.com/owner/repo/commit/2222222))

## [1.3.1](https://github.com/owner/repo/compare/v1.3.0...v1.3.1) (2026-10-06)

### 🐛 Bug Fixes

* **sezzle:** fixed issue ([3333333](https://github.com/owner/repo/commit/3333333))

## [1.3.0](https://github.com/owner/repo/compare/v1.2.0...v1.3.0) (2026-10-05)
`;
    const result = extractLatestStableFromChangelog(markdown);
    assert.ok(result !== null);
    assert.equal(result?.version, '1.3.1');
    assert.equal(result?.date, '2026-10-06');
    assert.ok(result?.body.includes('**sezzle:** fixed issue'));
    assert.ok(!result?.body.includes('dev fix'));
    assert.ok(!result?.body.includes('1.3.0'));
  });

  it('handles heading without brackets e.g. ## 1.0.0 (2026-09-20)', () => {
    const markdown = `## 1.0.0 (2026-09-20)

### Initial Release
`;
    const result = extractLatestStableFromChangelog(markdown);
    assert.ok(result !== null);
    assert.equal(result?.version, '1.0.0');
    assert.equal(result?.date, '2026-09-20');
  });
});

describe('resolveLatestReleaseSync', () => {
  const changelog = `## [1.4.0](https://github.com/owner/repo/compare/v1.3.1...v1.4.0) (2026-10-08)

### 🐛 Bug Fixes

* **apk-lab:** address review feedback ([9e1addc](https://github.com/owner/repo/commit/9e1addc))

## [1.3.1](https://github.com/owner/repo/compare/v1.3.0...v1.3.1) (2026-10-06)
`;

  it('uses stable bundleJson directly when bundleJson has stable version', () => {
    const bundle: RawBundle = {
      version: '1.4.0',
      created_at: '2026-10-08T20:26:34',
      description:
        '### 🐛 Bug Fixes\n\n* **apk-lab:** fix ([1234567](https://github.com/owner/repo/commit/1234567))',
      download_url:
        'https://github.com/owner/repo/releases/download/v1.4.0/patches-1.4.0.mpp',
    };

    const release = resolveLatestReleaseSync({
      bundleJson: bundle,
      changelogContent: changelog,
    });
    assert.equal(release.version, '1.4.0');
    assert.equal(release.downloadUrl, bundle.download_url);
    assert.equal(release.recentChanges.length, 1);
    assert.equal(release.recentChanges[0].scope, 'apk-lab');
  });

  it('rejects pre-release bundleJson and resolves latest stable from changelog', () => {
    const devBundle: RawBundle = {
      version: '1.4.0-dev.7',
      created_at: '2026-10-08T20:00:00',
      description:
        '### 🐛 Bug Fixes\n\n* **apk-lab:** pre-release commit ([9999999](https://github.com/owner/repo/commit/9999999))',
      download_url:
        'https://github.com/owner/repo/releases/download/v1.4.0-dev.7/patches-1.4.0-dev.7.mpp',
    };

    const release = resolveLatestReleaseSync({
      bundleJson: devBundle,
      changelogContent: changelog,
      repo: 'owner/repo',
    });

    assert.equal(release.version, '1.4.0');
    assert.equal(
      release.downloadUrl,
      'https://github.com/owner/repo/releases/download/v1.4.0/patches-1.4.0.mpp',
    );
    assert.equal(release.recentChanges.length, 1);
    assert.equal(release.recentChanges[0].hash, '9e1addc');
  });

  it('resolves stable version even when both bundleJson and top of changelog are pre-releases', () => {
    const devChangelog = `## [1.4.0-dev.7] (2026-10-08)

* dev commit

## [1.3.1](https://github.com/owner/repo/compare/v1.3.0...v1.3.1) (2026-10-06)

### 🐛 Bug Fixes

* **blackjack:** update to 2.22.09 ([34eca78](https://github.com/owner/repo/commit/34eca78))
`;
    const devBundle: RawBundle = {
      version: '1.4.0-dev.7',
      created_at: '2026-10-08T20:00:00',
      description: '* dev commit',
      download_url:
        'https://github.com/owner/repo/releases/download/v1.4.0-dev.7/patches-1.4.0-dev.7.mpp',
    };

    const release = resolveLatestReleaseSync({
      bundleJson: devBundle,
      changelogContent: devChangelog,
      repo: 'owner/repo',
    });

    assert.equal(release.version, '1.3.1');
    assert.equal(release.releaseDate, 'Oct 6, 2026');
    assert.equal(
      release.downloadUrl,
      'https://github.com/owner/repo/releases/download/v1.3.1/patches-1.3.1.mpp',
    );
    assert.equal(release.recentChanges[0].scope, 'blackjack');
  });
});

describe('resolveLatestRelease async', () => {
  it('falls back to local files when fetch fails', async () => {
    const changelog = `## [1.4.0] (2026-10-08)\n\n* **test:** change ([1111111](https://github.com/owner/repo/commit/1111111))`;
    const bundle: RawBundle = {
      version: '1.4.0',
      created_at: '2026-10-08T20:26:34',
      description:
        '* **test:** change ([1111111](https://github.com/owner/repo/commit/1111111))',
      download_url: 'https://example.com/patches.mpp',
    };

    const release = await resolveLatestRelease({
      bundleJson: bundle,
      changelogContent: changelog,
      repo: 'nonexistent-org/nonexistent-repo-12345',
    });

    assert.equal(release.version, '1.4.0');
    assert.equal(release.downloadUrl, 'https://example.com/patches.mpp');
    assert.equal(release.recentChanges.length, 1);
  });

  it('fetches actual latest tagged release from live GitHub repository', async () => {
    const release = await fetchLatestGitHubRelease(
      'ihatenodejs/aidans-patches',
    );
    assert.ok(release !== null);
    assert.ok(
      typeof release?.version === 'string' && release.version.length > 0,
    );
    assert.equal(isPreRelease(release?.version || ''), false);
    assert.ok(release?.downloadUrl.endsWith('.mpp'));
    assert.ok((release?.recentChanges.length ?? 0) > 0);
  });
});
