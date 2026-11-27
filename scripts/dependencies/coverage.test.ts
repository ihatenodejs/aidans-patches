import { describe, expect, test } from 'bun:test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { RUST_PROJECTS } from '../rust-projects';
import {
  GRADLE_BUILD_FILES,
  JAVASCRIPT_MANIFESTS,
  REGISTERED_CARGO_MANIFESTS,
  REGISTERED_CMAKE_MANIFESTS,
  collectInventory,
} from './inventory';
import { loadAndValidatePolicy } from './model';

const REPO_ROOT = path.resolve(__dirname, '../..');

const SKIP_DIRECTORIES = new Set([
  '.git',
  '.apk-lab',
  '.astro',
  '.gradle',
  '.tools',
  '.venv',
  '__pycache__',
  'analysis',
  'apk',
  'build',
  'dist',
  'node_modules',
  'target',
]);

function walk(
  directory: string,
  predicate: (relativePath: string) => boolean,
): string[] {
  const found: string[] = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (SKIP_DIRECTORIES.has(entry.name)) continue;
    const absolute = path.join(directory, entry.name);
    const relative = path.relative(REPO_ROOT, absolute);
    if (entry.isDirectory()) {
      found.push(...walk(absolute, predicate));
    } else if (predicate(relative)) {
      found.push(relative);
    }
  }
  return found.sort();
}

function expectRegistered(
  discovered: string[],
  registered: readonly string[],
  label: string,
): void {
  const discoveredSet = new Set(discovered);
  const registeredSet = new Set(registered);
  const unregistered = discovered.filter((file) => !registeredSet.has(file));
  const missing = registered.filter((file) => !discoveredSet.has(file));
  expect({ label, unregistered, missing }).toEqual({
    label,
    unregistered: [],
    missing: [],
  });
}

describe('Dependency manifest coverage', () => {
  test('every JavaScript manifest is registered', () => {
    expectRegistered(
      walk(REPO_ROOT, (file) => path.basename(file) === 'package.json'),
      JAVASCRIPT_MANIFESTS.map((manifest) => manifest.manifest),
      'package.json',
    );
  });

  test('every Cargo manifest is registered', () => {
    expectRegistered(
      walk(REPO_ROOT, (file) => path.basename(file) === 'Cargo.toml'),
      REGISTERED_CARGO_MANIFESTS,
      'Cargo.toml',
    );
  });

  test('every CMake manifest is registered', () => {
    expectRegistered(
      walk(REPO_ROOT, (file) => path.basename(file) === 'CMakeLists.txt'),
      REGISTERED_CMAKE_MANIFESTS,
      'CMakeLists.txt',
    );
  });

  test('every Gradle build script is registered', () => {
    expectRegistered(
      walk(REPO_ROOT, (file) => path.basename(file) === 'build.gradle.kts'),
      GRADLE_BUILD_FILES,
      'build.gradle.kts',
    );
  });

  test('every registered Rust project has a manifest', () => {
    for (const project of RUST_PROJECTS) {
      expect(fs.existsSync(path.join(REPO_ROOT, project.manifest))).toBe(true);
    }
  });

  test('composite actions are registered or absent', () => {
    expect(
      walk(
        REPO_ROOT,
        (file) =>
          file.startsWith('.github/actions/') &&
          (file.endsWith('action.yml') || file.endsWith('action.yaml')),
      ),
    ).toEqual([]);
  });

  test('real inventory covers pinned assets and target-specific crates', () => {
    const { items } = collectInventory(
      REPO_ROOT,
      loadAndValidatePolicy(REPO_ROOT),
    );
    const ids = new Set(items.map((item) => item.id));
    for (const id of [
      'embedded:apple-color-emoji',
      'embedded:tessdata-eng',
      'cmake:tools/asset-prep/vendor/turbojpeg-sys/CMakeLists.txt#vendor/libjpeg-turbo',
      'tool:wrangler-compat',
      "crates:tools/apk-lab/Cargo.toml#target.'cfg(unix)'.dependencies/libc",
    ]) {
      expect(ids.has(id)).toBe(true);
    }
  });

  test('real inventory has no Bun/npm lockfile divergence', () => {
    const { items } = collectInventory(
      REPO_ROOT,
      loadAndValidatePolicy(REPO_ROOT),
    );
    expect(
      items
        .filter((item) => item.lockfileMismatch)
        .map((item) => `${item.id}: ${item.lockfileMismatch}`),
    ).toEqual([]);
  });
});
