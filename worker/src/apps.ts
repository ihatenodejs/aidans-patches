import type { TargetAppConfig } from './types';
import rawPatchesList from '../../patches-list.json';

export function compareUtf16(a: string, b: string): number {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

export function parseMonitoredApps(data: unknown): TargetAppConfig[] {
  if (!data || typeof data !== 'object') {
    throw new Error('Invalid patches list data: must be an object');
  }

  if (!('apps' in data) || !Array.isArray(data.apps)) {
    throw new Error('Invalid patches list data: missing "apps" array');
  }

  const seenPackages = new Set<string>();
  let lastPkg = '';
  const result: TargetAppConfig[] = [];

  for (const item of data.apps) {
    if (!item || typeof item !== 'object') {
      throw new Error('Invalid app entry: must be an object');
    }

    const {
      packageName,
      name,
      apkFileType,
      signatures,
      patchNames,
      supportedVersions,
      latestSupportedVersion,
    } = item as Record<string, unknown>;

    if (typeof packageName !== 'string' || !packageName.trim()) {
      throw new Error('Invalid app entry: missing packageName');
    }
    if (typeof name !== 'string' || !name.trim()) {
      throw new Error(`Invalid app entry for ${packageName}: missing name`);
    }
    if (typeof apkFileType !== 'string' || !apkFileType.trim()) {
      throw new Error(
        `Invalid app entry for ${packageName}: missing apkFileType`,
      );
    }
    if (
      typeof latestSupportedVersion !== 'string' ||
      !latestSupportedVersion.trim()
    ) {
      throw new Error(
        `Invalid app entry for ${packageName}: missing latestSupportedVersion`,
      );
    }

    if (seenPackages.has(packageName)) {
      throw new Error(`Duplicate package name: ${packageName}`);
    }
    seenPackages.add(packageName);

    if (compareUtf16(packageName, lastPkg) < 0) {
      throw new Error(
        `Apps catalog must be sorted by packageName: ${packageName} appeared after ${lastPkg}`,
      );
    }
    lastPkg = packageName;

    if (
      !Array.isArray(signatures) ||
      signatures.some((s) => typeof s !== 'string')
    ) {
      throw new Error(`Invalid signatures for package ${packageName}`);
    }
    for (let i = 1; i < signatures.length; i++) {
      if (compareUtf16(signatures[i], signatures[i - 1]) <= 0) {
        throw new Error(
          `Signatures for package ${packageName} must be uniquely sorted`,
        );
      }
    }

    if (
      !Array.isArray(patchNames) ||
      patchNames.length === 0 ||
      patchNames.some((p) => typeof p !== 'string' || !p.trim())
    ) {
      throw new Error(`Invalid patchNames for package ${packageName}`);
    }
    for (let i = 1; i < patchNames.length; i++) {
      if (compareUtf16(patchNames[i], patchNames[i - 1]) <= 0) {
        throw new Error(
          `Patch names for package ${packageName} must be uniquely sorted`,
        );
      }
    }

    if (
      !Array.isArray(supportedVersions) ||
      supportedVersions.length === 0 ||
      supportedVersions.some((v) => typeof v !== 'string' || !v.trim())
    ) {
      throw new Error(`Invalid supportedVersions for package ${packageName}`);
    }
    if (new Set(supportedVersions).size !== supportedVersions.length) {
      throw new Error(`Duplicate supportedVersions for package ${packageName}`);
    }

    if (
      latestSupportedVersion !== supportedVersions[supportedVersions.length - 1]
    ) {
      throw new Error(
        `latestSupportedVersion for ${packageName} (${latestSupportedVersion}) does not match last supportedVersion (${supportedVersions[supportedVersions.length - 1]})`,
      );
    }

    result.push({
      packageName,
      name,
      apkFileType,
      signatures: [...signatures],
      patchNames: [...patchNames],
      supportedVersions: [...supportedVersions],
      latestSupportedVersion,
    });
  }

  return result;
}

export const MONITORED_APPS: TargetAppConfig[] =
  parseMonitoredApps(rawPatchesList);
