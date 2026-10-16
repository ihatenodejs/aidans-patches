import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import { getApps, getPatches } from '../src/data/repo';

describe('repo-app-catalog contract', () => {
  const currentDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(currentDir, '../../');
  const patchesListPath = path.join(repoRoot, 'patches-list.json');

  const rawData = JSON.parse(fs.readFileSync(patchesListPath, 'utf-8'));
  const catalogApps = rawData.apps as Array<{
    packageName: string;
    name: string;
    description: string | null;
    apkFileType: string;
    appIconColor: string | null;
    signatures: string[];
    targets: Array<{
      version: string;
      versionCodes?: Record<string, number> | null;
      isExperimental: boolean;
      minSdk?: number | null;
      description?: string | null;
    }>;
    patchNames: string[];
    supportedVersions: string[];
    latestSupportedVersion: string;
  }>;

  const siteApps = getApps();
  const sitePatches = getPatches();

  it('builds one site app per generated catalog package', () => {
    assert.equal(
      siteApps.length,
      catalogApps.length,
      'Site apps count must match catalog apps count',
    );

    const catalogPackages = new Set(catalogApps.map((a) => a.packageName));
    const sitePackages = new Set(siteApps.map((a) => a.packageName));
    assert.deepEqual(
      Array.from(sitePackages).sort(),
      Array.from(catalogPackages).sort(),
      'Site package names must match catalog package names',
    );
  });

  it('guarantees unique app IDs and unique package names', () => {
    const ids = siteApps.map((a) => a.id);
    assert.equal(
      new Set(ids).size,
      ids.length,
      'App IDs must be unique across all apps',
    );

    const pkgs = siteApps.map((a) => a.packageName);
    assert.equal(
      new Set(pkgs).size,
      pkgs.length,
      'Package names must be unique across all apps',
    );
  });

  it('matches supported versions, latest version, signatures, targets, and patch count with catalog', () => {
    const catalogMap = new Map(catalogApps.map((a) => [a.packageName, a]));

    for (const app of siteApps) {
      const catalogApp = catalogMap.get(app.packageName);
      assert.ok(
        catalogApp,
        `Missing catalog entry for package ${app.packageName}`,
      );

      assert.deepEqual(
        app.supportedVersions,
        catalogApp.supportedVersions,
        `supportedVersions mismatch for ${app.packageName}`,
      );
      assert.equal(
        app.latestSupportedVersion,
        catalogApp.latestSupportedVersion,
        `latestSupportedVersion mismatch for ${app.packageName}`,
      );

      const expectedSignatures =
        catalogApp.signatures.length > 0 ? catalogApp.signatures : null;
      assert.deepEqual(
        app.signatures,
        expectedSignatures,
        `signatures mismatch for ${app.packageName}`,
      );

      assert.equal(
        app.patchCount,
        catalogApp.patchNames.length,
        `patchCount mismatch for ${app.packageName}`,
      );

      assert.equal(
        app.targets.length,
        catalogApp.targets.length,
        `targets length mismatch for ${app.packageName}`,
      );

      for (let i = 0; i < app.targets.length; i++) {
        assert.equal(
          app.targets[i].version,
          catalogApp.targets[i].version,
          `target version mismatch for ${app.packageName} at index ${i}`,
        );
        assert.equal(
          app.targets[i].isExperimental,
          catalogApp.targets[i].isExperimental,
          `target isExperimental mismatch for ${app.packageName} at index ${i}`,
        );
        assert.equal(
          app.targets[i].minSdk,
          catalogApp.targets[i].minSdk ?? null,
          `target minSdk mismatch for ${app.packageName} at index ${i}`,
        );
        assert.equal(
          app.targets[i].description,
          catalogApp.targets[i].description ?? null,
          `target description mismatch for ${app.packageName} at index ${i}`,
        );
      }
    }
  });

  it('resolves every patch compatible app ID to exactly one site app', () => {
    const appsById = new Map(siteApps.map((a) => [a.id, a]));

    for (const patch of sitePatches) {
      for (const appId of patch.compatibleAppIds) {
        const resolved = appsById.get(appId);
        assert.ok(
          resolved,
          `Patch "${patch.name}" references unresolved app ID "${appId}"`,
        );
      }
    }
  });
});
