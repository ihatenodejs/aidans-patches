import { describe, expect, test } from 'bun:test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { generateNotes } from '@semantic-release/release-notes-generator';
import dryRunConfig from '../release.dryrun.config.mjs';

const ROOT = path.resolve(import.meta.dir, '..');

interface TypeEntry {
  type?: string;
  scope?: string;
  section?: string;
  hidden?: boolean;
}

interface ReleaseRule {
  type?: string;
  scope?: string;
  release?: string | false;
}

type PluginEntry = string | [string, Record<string, unknown>];

interface ReleaseConfig {
  plugins: PluginEntry[];
}

/** Release rules that decide a version bump, and therefore the note sections. */
const RELEASING_TYPES = ['fix', 'feat', 'bump', 'perf'];

const releaseConfig = Bun.YAML.parse(
  fs.readFileSync(path.join(ROOT, '.releaserc'), 'utf8'),
) as ReleaseConfig;

function pluginConfig(
  plugins: PluginEntry[],
  name: string,
): Record<string, unknown> {
  const entry = plugins.find(
    (plugin) => (typeof plugin === 'string' ? plugin : plugin[0]) === name,
  );
  if (!Array.isArray(entry)) {
    throw new Error(`release config has no ${name} plugin options`);
  }
  return entry[1];
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function releaseRulesFrom(options: Record<string, unknown>): ReleaseRule[] {
  const rules = options.releaseRules ?? [];
  if (!Array.isArray(rules)) {
    throw new Error('releaseRules must be an array');
  }
  const entries = rules.filter(isObject);
  if (entries.length !== rules.length) {
    throw new Error('releaseRules has a non-object entry');
  }
  return entries as ReleaseRule[];
}

function typesFrom(options: Record<string, unknown>): TypeEntry[] {
  const presetConfig = options.presetConfig;
  if (!isObject(presetConfig) || !('types' in presetConfig)) {
    throw new Error('release notes config has no presetConfig.types');
  }
  const types = presetConfig.types;
  if (!Array.isArray(types)) {
    throw new Error('release notes presetConfig.types must be an array');
  }
  const entries = types.filter(isObject);
  if (entries.length !== types.length) {
    throw new Error('release notes presetConfig.types has a non-object entry');
  }
  return entries as TypeEntry[];
}

const analyzerOptions = pluginConfig(
  releaseConfig.plugins,
  '@semantic-release/commit-analyzer',
);
const notesOptions = pluginConfig(
  releaseConfig.plugins,
  '@semantic-release/release-notes-generator',
);
const releaseRules = releaseRulesFrom(analyzerOptions);
const types = typesFrom(notesOptions);

const nonApplicationScopes = releaseRules
  .filter((rule) => rule.release === false && rule.scope)
  .map((rule) => rule.scope as string);

describe('release notes scope exclusions', () => {
  test('every non-application scope is hidden for every releasing type', () => {
    // `release: false` only suppresses the version bump. The notes generator
    // keeps any commit whose (type, scope) pair has no entry, so each
    // non-application scope needs an explicit hidden entry per releasing type.
    expect(nonApplicationScopes.length).toBeGreaterThan(0);

    for (const scope of nonApplicationScopes) {
      for (const type of RELEASING_TYPES) {
        const hidden = types.findIndex(
          (entry) => entry.type === type && entry.scope === scope,
        );
        expect(hidden, `${type}(${scope}) has no release-notes entry`).not.toBe(
          -1,
        );
        expect(
          types[hidden]?.hidden,
          `${type}(${scope}) must not appear in release notes`,
        ).toBe(true);

        // The preset keeps the first (type, scope) match, so the exclusion
        // has to be declared before the sectioned entry for the same type.
        const sectioned = types.findIndex(
          (entry) => entry.type === type && !entry.scope && entry.section,
        );
        expect(sectioned, `${type} has no section entry`).not.toBe(-1);
        expect(hidden).toBeLessThan(sectioned);
      }
    }
  });

  test('non-application commits are dropped and application commits kept', async () => {
    const notes = await generateNotes(notesOptions, {
      commits: [
        {
          hash: 'a'.repeat(40),
          message: 'fix(ci): separate release and Maven credentials',
        },
        {
          hash: 'b'.repeat(40),
          message: 'fix(worker): serialize app state through durable objects',
        },
        {
          hash: 'c'.repeat(40),
          message: 'feat(apk-lab): add APKPure acquisition fallback',
        },
        {
          hash: 'd'.repeat(40),
          message: 'fix(sezzle): reject unexpected update bytecode',
        },
        {
          hash: 'e'.repeat(40),
          message: 'feat(flow-free): add unlock and Google Play patches',
        },
      ],
      lastRelease: { gitTag: 'v1.10.0' },
      nextRelease: { version: '1.11.0', gitTag: 'v1.11.0' },
      options: {
        repositoryUrl: 'git@github.com:ihatenodejs/aidans-patches.git',
      },
      cwd: ROOT,
    });

    expect(notes).not.toContain('**ci:**');
    expect(notes).not.toContain('**worker:**');
    expect(notes).not.toContain('**apk-lab:**');
    expect(notes).toContain('**sezzle:** reject unexpected update bytecode');
    expect(notes).toContain(
      '**flow-free:** add unlock and Google Play patches',
    );
  });

  test('the dry-run config mirrors the release rules and notes types', () => {
    const dryRunAnalyzer = pluginConfig(
      dryRunConfig.plugins,
      '@semantic-release/commit-analyzer',
    );
    const dryRunNotes = pluginConfig(
      dryRunConfig.plugins,
      '@semantic-release/release-notes-generator',
    );

    expect(releaseRulesFrom(dryRunAnalyzer)).toEqual(releaseRules);
    expect(typesFrom(dryRunNotes)).toEqual(types);
  });
});
