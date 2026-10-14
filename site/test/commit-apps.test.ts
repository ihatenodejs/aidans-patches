import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildScopeIndex, resolveAppForScope } from '../src/data/commit-apps';
import type { App } from '../src/data/types';

const MOCK_APPS: App[] = [
  {
    id: '365-pay',
    name: '365 Pay',
    packageName: 'com.avinside',
    apkFileType: 'APKM',
    appIconColor: '#00A859',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 26,
    signatures: null,
    patchCount: 3,
    playStoreUrl: 'https://play.google.com/store/apps/details?id=com.avinside',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/365-pay',
  },
  {
    id: 'adobe-scan',
    name: 'Adobe Scan',
    packageName: 'com.adobe.scan.android',
    apkFileType: 'APKM',
    appIconColor: '#1473E6',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 26,
    signatures: null,
    patchCount: 5,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.adobe.scan.android',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/adobe-scan',
  },
  {
    id: 'aftership',
    name: 'AfterShip',
    packageName: 'com.aftership.AfterShip',
    apkFileType: 'APK',
    appIconColor: '#FD5B26',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 23,
    signatures: null,
    patchCount: 8,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.aftership.AfterShip',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/aftership',
  },
  {
    id: 'blackjack',
    name: 'Blackjack',
    packageName: 'com.tripledot.blackjack',
    apkFileType: 'APKM',
    appIconColor: '#205B1F',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 28,
    signatures: null,
    patchCount: 6,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.tripledot.blackjack',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/blackjack',
  },
  {
    id: 'canvas-student',
    name: 'Canvas Student',
    packageName: 'com.instructure.candroid',
    apkFileType: 'APKM',
    appIconColor: '#E62725',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 28,
    signatures: null,
    patchCount: 1,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.instructure.candroid',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/canvas-student',
  },
  {
    id: 'fizz',
    name: 'Fizz',
    packageName: 'com.ashtoncofer.Buzz',
    apkFileType: 'APKM',
    appIconColor: '#7F00FF',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 26,
    signatures: null,
    patchCount: 4,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.ashtoncofer.Buzz',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/fizz',
  },
  {
    id: 'geocaching',
    name: 'Geocaching',
    packageName: 'com.groundspeak.geocaching.intro',
    apkFileType: 'APKM',
    appIconColor: '#007D46',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 28,
    signatures: null,
    patchCount: 8,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.groundspeak.geocaching.intro',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/geocaching',
  },
  {
    id: 'navigate360-student',
    name: 'Navigate360 Student',
    packageName: 'com.eab.se',
    apkFileType: 'APKM',
    appIconColor: '#0071CE',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 26,
    signatures: null,
    patchCount: 2,
    playStoreUrl: 'https://play.google.com/store/apps/details?id=com.eab.se',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/navigate360-student',
  },
  {
    id: 'nhl',
    name: 'NHL',
    packageName: 'com.nhl.gc1112.free',
    apkFileType: 'APKM',
    appIconColor: '#000000',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 28,
    signatures: null,
    patchCount: 3,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.nhl.gc1112.free',
    docsUrl: 'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/nhl',
  },
  {
    id: 'sezzle',
    name: 'Sezzle',
    packageName: 'com.sezzle.sezzlemobile',
    apkFileType: 'APKM',
    appIconColor: '#5C2C94',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 26,
    signatures: null,
    patchCount: 14,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.sezzle.sezzlemobile',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/sezzle',
  },
  {
    id: 'sidelineswap',
    name: 'SidelineSwap',
    packageName: 'com.sidelineswap.android',
    apkFileType: 'XAPK',
    appIconColor: '#02C874',
    targets: [],
    supportedVersions: ['1.0.0'],
    latestSupportedVersion: '1.0.0',
    minSdk: 28,
    signatures: null,
    patchCount: 4,
    playStoreUrl:
      'https://play.google.com/store/apps/details?id=com.sidelineswap.android',
    docsUrl:
      'https://github.com/ihatenodejs/aidans-patches/tree/main/docs/sidelineswap',
  },
];

describe('resolveAppForScope', () => {
  it('resolves all 25 historical commit scopes correctly', () => {
    const historicalCases: Array<[string, string]> = [
      ['aftership', 'aftership'],
      ['blackjack', 'blackjack'],
      ['tripledot', 'blackjack'],
      ['sidelineswap', 'sidelineswap'],
      ['sideline', 'sidelineswap'],
      ['sezzle', 'sezzle'],
      ['sezzlemobile', 'sezzle'],
      ['fizz', 'fizz'],
      ['buzz', 'fizz'],
      ['adobe-scan', 'adobe-scan'],
      ['adobescan', 'adobe-scan'],
      ['adobe', 'adobe-scan'],
      ['scan', 'adobe-scan'],
      ['canvas', 'canvas-student'],
      ['canvas-student', 'canvas-student'],
      ['candroid', 'canvas-student'],
      ['navigate360', 'navigate360-student'],
      ['navigate360-student', 'navigate360-student'],
      ['navigate', 'navigate360-student'],
      ['eab', 'navigate360-student'],
      ['geocaching', 'geocaching'],
      ['groundspeak', 'geocaching'],
      ['365pay', '365-pay'],
      ['pay365', '365-pay'],
      ['nhl', 'nhl'],
    ];

    for (const [scope, expectedAppId] of historicalCases) {
      const app = resolveAppForScope(scope, MOCK_APPS);
      assert.ok(app !== null, `Expected scope "${scope}" to resolve to an app`);
      assert.equal(
        app?.id,
        expectedAppId,
        `Scope "${scope}" resolved to "${app?.id}" instead of "${expectedAppId}"`,
      );
    }
  });

  it('handles case-insensitivity, leading/trailing whitespace, and punctuation', () => {
    assert.equal(
      resolveAppForScope('  ADOBE-SCAN  ', MOCK_APPS)?.id,
      'adobe-scan',
    );
    assert.equal(
      resolveAppForScope('EAB!', MOCK_APPS)?.id,
      'navigate360-student',
    );
    assert.equal(resolveAppForScope('365_pay', MOCK_APPS)?.id, '365-pay');
    assert.equal(resolveAppForScope('TripleDot', MOCK_APPS)?.id, 'blackjack');
  });

  it('returns null for unknown scopes or empty inputs', () => {
    assert.equal(resolveAppForScope('', MOCK_APPS), null);
    assert.equal(resolveAppForScope(null, MOCK_APPS), null);
    assert.equal(resolveAppForScope('unknown-scope', MOCK_APPS), null);
    assert.equal(resolveAppForScope('tooling', MOCK_APPS), null);
    assert.equal(resolveAppForScope('site', MOCK_APPS), null);
    assert.equal(resolveAppForScope('worker', MOCK_APPS), null);
  });

  it('ensures no token collision exists among distinct apps', () => {
    const index = buildScopeIndex(MOCK_APPS);
    assert.ok(index.size > 0);

    // Verify key tokens resolve to unambiguous apps
    assert.equal(index.get('eab')?.id, 'navigate360-student');
    assert.equal(index.get('candroid')?.id, 'canvas-student');
    assert.equal(index.get('buzz')?.id, 'fizz');
    assert.equal(index.get('tripledot')?.id, 'blackjack');
    assert.equal(index.get('groundspeak')?.id, 'geocaching');
    assert.equal(index.get('365pay')?.id, '365-pay');
  });
});
