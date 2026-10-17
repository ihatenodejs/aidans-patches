import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  getApps,
  getInitialVersionStatuses,
  getLatestPlayRelease,
} from '../src/data/repo';

describe('Repository status baseline fallback contract', () => {
  it('assigns every app exactly one initial status record', () => {
    const apps = getApps();
    const statuses = getInitialVersionStatuses();

    assert.ok(apps.length > 0, 'Apps list must not be empty');
    assert.equal(
      statuses.length,
      apps.length,
      'Initial version statuses count must match apps count',
    );

    const appIds = new Set(apps.map((app) => app.id));
    const statusAppIds = new Set(statuses.map((s) => s.appId));
    assert.deepEqual(
      statusAppIds,
      appIds,
      'Every app must be represented in initial statuses by appId',
    );

    const packageNames = new Set(apps.map((app) => app.packageName));
    const statusPackageNames = new Set(statuses.map((s) => s.packageName));
    assert.deepEqual(
      statusPackageNames,
      packageNames,
      'Every app must be represented in initial statuses by packageName',
    );
  });

  it('marks every initial status as explicitly unknown with null Play dates', () => {
    const statuses = getInitialVersionStatuses();
    assert.ok(statuses.length > 0, 'Statuses list must not be empty');

    for (const status of statuses) {
      assert.equal(
        status.status,
        'unknown',
        `App '${status.appId}' status must be 'unknown'`,
      );
      assert.equal(
        status.updatedAt,
        null,
        `App '${status.appId}' updatedAt must be null`,
      );
      assert.equal(
        status.updatedOn,
        null,
        `App '${status.appId}' updatedOn must be null`,
      );
      assert.equal(
        status.playVersion,
        null,
        `App '${status.appId}' playVersion must be null`,
      );
      assert.equal(
        status.checkedAt,
        null,
        `App '${status.appId}' checkedAt must be null`,
      );
    }
  });

  it('returns null from getLatestPlayRelease() when no build-time status has a date', () => {
    const latest = getLatestPlayRelease();
    assert.equal(
      latest,
      null,
      'getLatestPlayRelease must return null when no build-time statuses are dated',
    );
  });

  it('preserves canonical app identity and supported versions in initial statuses', () => {
    const apps = getApps();
    const statuses = getInitialVersionStatuses();
    const statusByPkg = new Map(statuses.map((s) => [s.packageName, s]));

    for (const app of apps) {
      const status = statusByPkg.get(app.packageName);
      assert.ok(
        status,
        `Status record must exist for package '${app.packageName}'`,
      );
      assert.equal(
        status.appName,
        app.name,
        `Status appName must match canonical app.name for '${app.id}'`,
      );
      assert.equal(
        status.appId,
        app.id,
        `Status appId must match canonical app.id for '${app.id}'`,
      );
      assert.equal(
        status.appIconColor,
        app.appIconColor,
        `Status appIconColor must match canonical app.appIconColor for '${app.id}'`,
      );
      assert.deepEqual(
        status.supportedVersions,
        app.supportedVersions,
        `Status supportedVersions must match canonical app.supportedVersions for '${app.id}'`,
      );
      assert.equal(
        status.latestSupportedVersion,
        app.latestSupportedVersion,
        `Status latestSupportedVersion must match canonical app.latestSupportedVersion for '${app.id}'`,
      );
      assert.equal(
        status.playStoreUrl,
        app.playStoreUrl,
        `Status playStoreUrl must match canonical app.playStoreUrl for '${app.id}'`,
      );
    }
  });
});
