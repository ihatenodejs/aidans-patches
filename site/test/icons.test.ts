import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { getAppIconUrl } from '../src/data/icons';

describe('getAppIconUrl', () => {
  it('returns valid URL for existing application icon', () => {
    assert.equal(
      getAppIconUrl('com.adobe.scan.android'),
      '/icons/com.adobe.scan.android.png',
    );
    assert.equal(
      getAppIconUrl('com.sidelineswap.android'),
      '/icons/com.sidelineswap.android.png',
    );
    assert.equal(
      getAppIconUrl('com.sezzle.sezzlemobile'),
      '/icons/com.sezzle.sezzlemobile.png',
    );
  });

  it('returns null for non-existent package icon', () => {
    assert.equal(getAppIconUrl('com.nonexistent.fakeapp'), null);
    assert.equal(getAppIconUrl('random-package'), null);
  });

  it('returns null for empty or invalid input', () => {
    assert.equal(getAppIconUrl(''), null);
    // @ts-expect-error testing null safety
    assert.equal(getAppIconUrl(null), null);
    // @ts-expect-error testing undefined safety
    assert.equal(getAppIconUrl(undefined), null);
  });
});
