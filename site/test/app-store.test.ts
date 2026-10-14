import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { getApps } from '../src/data/repo';
import {
  APP_FALLBACK_CATEGORIES,
  APP_FALLBACK_SUMMARIES,
  DISCOVERY_SECTIONS,
  STORE_CATEGORIES,
  buildDiscoverySections,
  normalizePlayCategoryId,
  resolveStoreMetadata,
} from '../src/data/app-store';
import {
  resolveFallbackMediaUrl,
  type PublicStatusPayload,
} from '../src/data/status-client';

describe('App store taxonomy and fallback invariants', () => {
  it('assigns every supported app a valid fallback category and summary', () => {
    const apps = getApps();
    assert.ok(apps.length > 0, 'Apps list must not be empty');

    for (const app of apps) {
      const catId = APP_FALLBACK_CATEGORIES[app.id];
      assert.ok(
        catId && STORE_CATEGORIES[catId],
        `App '${app.id}' must map to a valid StoreCategoryId, got '${catId}'`,
      );

      const summary = APP_FALLBACK_SUMMARIES[app.id];
      assert.ok(
        summary && summary.trim().length > 10,
        `App '${app.id}' must have a non-empty fallback summary`,
      );
    }
  });

  it('guarantees every app appears in at least one discovery section', () => {
    const apps = getApps();
    const appIds = new Set(apps.map((a) => a.id));
    const representedIds = new Set<string>();

    for (const section of DISCOVERY_SECTIONS) {
      for (const id of section.appIds) {
        assert.ok(
          appIds.has(id),
          `Section '${section.id}' references unknown app '${id}'`,
        );
        representedIds.add(id);
      }
    }

    for (const id of appIds) {
      assert.ok(
        representedIds.has(id),
        `App '${id}' must appear in at least one discovery rail`,
      );
    }
  });

  it('buildDiscoverySections builds sections without duplicates within a rail', () => {
    const apps = getApps();
    const sections = buildDiscoverySections(apps);

    assert.equal(sections.length, 3, 'Must have 3 discovery sections');

    for (const section of sections) {
      assert.ok(section.apps.length > 0, `Section ${section.id} has apps`);
      const seen = new Set<string>();
      for (const app of section.apps) {
        assert.ok(
          !seen.has(app.id),
          `Duplicate app ${app.id} in ${section.id}`,
        );
        seen.add(app.id);
      }
    }
  });
});

describe('Play Store category normalization', () => {
  it('maps business and productivity to productivity', () => {
    assert.equal(normalizePlayCategoryId('BUSINESS'), 'productivity');
    assert.equal(normalizePlayCategoryId('PRODUCTIVITY'), 'productivity');
  });

  it('maps shopping and finance to shopping', () => {
    assert.equal(normalizePlayCategoryId('SHOPPING'), 'shopping');
    assert.equal(normalizePlayCategoryId('FINANCE'), 'shopping');
  });

  it('maps all GAME_* categories to games', () => {
    assert.equal(normalizePlayCategoryId('GAME_CARD'), 'games');
    assert.equal(normalizePlayCategoryId('GAME_CASUAL'), 'games');
    assert.equal(normalizePlayCategoryId('GAME_ACTION'), 'games');
    assert.equal(normalizePlayCategoryId('CASINO'), 'games');
  });

  it('falls back safely for unknown or absent category IDs', () => {
    assert.equal(normalizePlayCategoryId(null, 'education'), 'education');
    assert.equal(normalizePlayCategoryId('', 'sports'), 'sports');
    assert.equal(
      normalizePlayCategoryId('NON_EXISTENT_CATEGORY', 'other'),
      'other',
    );
  });
});

describe('Live store metadata resolution and boundary validation', () => {
  const apps = getApps();
  const testApp = apps[0];

  it('falls back to static defaults when status payload is null or empty', () => {
    const meta = resolveStoreMetadata(null, testApp);
    assert.equal(meta.category.id, APP_FALLBACK_CATEGORIES[testApp.id]);
    assert.equal(meta.summary, APP_FALLBACK_SUMMARIES[testApp.id]);
    assert.equal(meta.description, null);
    assert.equal(meta.developer, null);
    assert.equal(meta.releaseNotes, null);
  });

  it('applies live metadata and respects string lengths/bounds', () => {
    const payload: PublicStatusPayload = {
      updatedAt: '2026-10-14T00:00:00Z',
      apps: {
        [testApp.packageName]: {
          playMetadata: {
            summary: 'Live summary from Google Play.',
            description: 'Full live description for the app.',
            developer: 'Certified Software Inc.',
            category: 'Productivity & Office',
            categoryId: 'BUSINESS',
            releaseNotes: 'Fixed bugs and improved performance.',
          },
        },
      },
    };

    const resolved = resolveStoreMetadata(payload, testApp);
    assert.equal(resolved.summary, 'Live summary from Google Play.');
    assert.equal(resolved.description, 'Full live description for the app.');
    assert.equal(resolved.developer, 'Certified Software Inc.');
    assert.equal(resolved.category.id, 'productivity');
    assert.equal(resolved.categoryLabel, 'Productivity & Office');
    assert.equal(resolved.releaseNotes, 'Fixed bugs and improved performance.');
  });

  it('rejects oversized or empty strings in favor of fallbacks', () => {
    const oversizedSummary = 'a'.repeat(600); // Max allowed is 500
    const payload: PublicStatusPayload = {
      updatedAt: '2026-10-14T00:00:00Z',
      apps: {
        [testApp.packageName]: {
          playMetadata: {
            summary: oversizedSummary,
            description: '   ',
            developer: '',
            categoryId: null,
          },
        },
      },
    };

    const resolved = resolveStoreMetadata(payload, testApp);
    // Oversized summary rejected, fallback retained
    assert.equal(resolved.summary, APP_FALLBACK_SUMMARIES[testApp.id]);
    assert.equal(resolved.description, null);
    assert.equal(resolved.developer, null);
    assert.equal(resolved.category.id, APP_FALLBACK_CATEGORIES[testApp.id]);
  });

  it('resolves rich store metadata fields: installs, score, reviews, screenshots, permissions', () => {
    const payload: PublicStatusPayload = {
      updatedAt: '2026-10-14T00:00:00Z',
      apps: {
        [testApp.packageName]: {
          playMetadata: {
            installs: '10M+',
            score: 4.6,
            ratingsCount: 15000,
            reviewsCount: 3000,
            contentRating: 'Everyone',
            contentRatingIcon: {
              sourceUrl: 'https://play-lh.googleusercontent.com/icon',
              fallbackPath: '/media/play/pkg/abc.png',
            },
            releasedOn: 'Jan 15, 2020',
            minAndroid: 'Android 8.0 and up',
            inAppPurchases: 'In-app purchases',
            permissionGroups: ['Camera', 'Storage'],
            screenshots: [
              {
                sourceUrl: 'https://play-lh.googleusercontent.com/s1',
                fallbackPath: null,
              },
              {
                sourceUrl: 'https://play-lh.googleusercontent.com/s2',
                fallbackPath: '/media/play/pkg/s2.png',
              },
            ],
            featuredReviews: [
              {
                author: 'Jane Doe',
                rating: 5,
                date: 'March 1, 2026',
                text: 'Great app!',
                helpfulCount: 12,
                avatar: {
                  sourceUrl: 'https://play-lh.googleusercontent.com/avatar',
                  fallbackPath: null,
                },
              },
            ],
          },
        },
      },
    };

    const resolved = resolveStoreMetadata(payload, testApp);
    assert.equal(resolved.installs, '10M+');
    assert.equal(resolved.score, 4.6);
    assert.equal(resolved.ratingsCount, 15000);
    assert.equal(resolved.reviewsCount, 3000);
    assert.equal(resolved.contentRating, 'Everyone');
    assert.equal(
      resolved.contentRatingIcon?.sourceUrl,
      'https://play-lh.googleusercontent.com/icon',
    );
    assert.equal(resolved.releasedOn, 'Jan 15, 2020');
    assert.equal(resolved.minAndroid, 'Android 8.0 and up');
    assert.equal(resolved.inAppPurchases, 'In-app purchases');
    assert.deepEqual(resolved.permissionGroups, ['Camera', 'Storage']);
    assert.equal(resolved.screenshots.length, 2);
    assert.equal(resolved.featuredReviews.length, 1);
    assert.equal(resolved.featuredReviews[0].author, 'Jane Doe');
  });

  it('resolveFallbackMediaUrl safely validates paths and produces full URLs', () => {
    const valid = resolveFallbackMediaUrl(
      '/media/play/com.adobe.scan.android/abcdef0123456789.png',
      'https://worker.example.com',
    );
    assert.equal(
      valid,
      'https://worker.example.com/media/play/com.adobe.scan.android/abcdef0123456789.png',
    );

    // Rejection of invalid paths
    assert.equal(resolveFallbackMediaUrl(null), null);
    assert.equal(resolveFallbackMediaUrl('https://evil.com/img.png'), null);
    assert.equal(resolveFallbackMediaUrl('/other/path.png'), null);
  });
});
