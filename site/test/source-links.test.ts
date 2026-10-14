import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  getPatchSourceMap,
  getPatchSourceUrl,
  GITHUB_BLOB_BASE,
} from '../src/data/source-links';

describe('source-links', () => {
  const currentDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(currentDir, '../../');

  it('scans and maps 100% of patches in patches-list.json to existing Kotlin files', () => {
    const patchesListPath = path.join(repoRoot, 'patches-list.json');
    assert.ok(
      fs.existsSync(patchesListPath),
      'patches-list.json should exist in repository root',
    );

    const content = JSON.parse(fs.readFileSync(patchesListPath, 'utf-8'));
    const sourceMap = getPatchSourceMap();

    assert.ok(
      Object.keys(sourceMap).length > 0,
      'Dynamic source map should not be empty',
    );

    function slugify(text: string): string {
      return text
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/(^-|-$)/g, '');
    }

    const missing: string[] = [];

    for (const patch of content.patches) {
      const patchName = patch.name;
      for (const cp of patch.compatiblePackages || []) {
        const appId = slugify(cp.name || cp.packageName);
        const scopedKey = `${appId}:${patchName}`;
        const relFile = sourceMap[scopedKey] || sourceMap[patchName];

        if (!relFile) {
          missing.push(`Unmapped: "${scopedKey}"`);
          continue;
        }

        const fullPath = path.resolve(repoRoot, relFile);
        if (!fs.existsSync(fullPath)) {
          missing.push(`File missing: "${scopedKey}" -> "${relFile}"`);
        }
      }
    }

    assert.deepEqual(
      missing,
      [],
      `Expected all patches to resolve to existing files, but got missing:\n${missing.join('\n')}`,
    );
  });

  it('generates valid GitHub source blob URLs for known patches', () => {
    const url = getPatchSourceUrl('Remove Ads and Tracking', 'adobe-scan');
    assert.ok(url.startsWith(GITHUB_BLOB_BASE));
    assert.ok(url.endsWith('RemoveAdsAndTrackingPatch.kt'));
  });

  it('falls back to patches directory URL when patch is not found', () => {
    const fallbackUrl = getPatchSourceUrl('Completely Nonexistent Patch Name');
    assert.equal(fallbackUrl, `${GITHUB_BLOB_BASE}/patches`);
  });
});
