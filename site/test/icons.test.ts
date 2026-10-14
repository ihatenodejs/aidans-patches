import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  getCachedAppIconUrl,
  resolveGooglePlayIconUrl,
} from '../src/data/app-icons';
import type { PublicStatusPayload } from '../src/data/status-client';

describe('getCachedAppIconUrl', () => {
  it('returns valid encoded URL for existing application package', () => {
    assert.equal(
      getCachedAppIconUrl('com.adobe.scan.android'),
      '/icons/com.adobe.scan.android.png',
    );
    assert.equal(
      getCachedAppIconUrl('com.sidelineswap.android'),
      '/icons/com.sidelineswap.android.png',
    );
    assert.equal(
      getCachedAppIconUrl('com.sezzle.sezzlemobile'),
      '/icons/com.sezzle.sezzlemobile.png',
    );
  });

  it('encodes special characters in package names', () => {
    assert.equal(
      getCachedAppIconUrl('com.test.app+variant'),
      '/icons/com.test.app%2Bvariant.png',
    );
  });

  it('returns null for empty or invalid input', () => {
    assert.equal(getCachedAppIconUrl(''), null);
    assert.equal(getCachedAppIconUrl('   '), null);
    // @ts-expect-error testing null safety
    assert.equal(getCachedAppIconUrl(null), null);
    // @ts-expect-error testing undefined safety
    assert.equal(getCachedAppIconUrl(undefined), null);
  });
});

describe('resolveGooglePlayIconUrl', () => {
  const samplePayload: PublicStatusPayload = {
    updatedAt: '2025-01-01T00:00:00Z',
    apps: {
      'com.sezzle.sezzlemobile': {
        iconUrl: 'https://play-lh.googleusercontent.com/abc123XYZ=w240-h480-rw',
      },
      'com.insecure.app': {
        iconUrl: 'http://play-lh.googleusercontent.com/insecure',
      },
      'com.deceptive.app': {
        iconUrl: 'https://play-lh.googleusercontent.com.attacker.com/evil',
      },
      'com.otherhost.app': {
        iconUrl: 'https://lh3.googleusercontent.com/other',
      },
      'com.malformed.app': {
        iconUrl: 'not a valid url',
      },
      'com.emptyicon.app': {
        iconUrl: '',
      },
      'com.nullicon.app': {
        iconUrl: null,
      },
    },
  };

  it('accepts valid HTTPS URLs from play-lh.googleusercontent.com', () => {
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.sezzle.sezzlemobile'),
      'https://play-lh.googleusercontent.com/abc123XYZ=w240-h480-rw',
    );
  });

  it('rejects HTTP URLs', () => {
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.insecure.app'),
      null,
    );
  });

  it('rejects deceptive and unauthorized hostnames', () => {
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.deceptive.app'),
      null,
    );
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.otherhost.app'),
      null,
    );
  });

  it('rejects malformed, empty, or null URLs', () => {
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.malformed.app'),
      null,
    );
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.emptyicon.app'),
      null,
    );
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.nullicon.app'),
      null,
    );
  });

  it('returns null for missing package or missing status payload', () => {
    assert.equal(
      resolveGooglePlayIconUrl(samplePayload, 'com.nonexistent.app'),
      null,
    );
    assert.equal(
      resolveGooglePlayIconUrl(null, 'com.sezzle.sezzlemobile'),
      null,
    );
    assert.equal(
      resolveGooglePlayIconUrl(undefined, 'com.sezzle.sezzlemobile'),
      null,
    );
    assert.equal(resolveGooglePlayIconUrl(samplePayload, ''), null);
  });
});
