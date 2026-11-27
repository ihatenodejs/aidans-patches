import { describe, expect, test } from 'bun:test';
import { spawnSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  applyApprovedProposal,
  replaceCargoDependencyVersion,
  seedStagingDirectory,
} from './apply';
import {
  collectInventory,
  compareLockfileResolutions,
  extractKtlintVersion,
  parseBunLock,
  parseBunLockPackages,
} from './inventory';
import {
  type DependencyItem,
  type DependencyPolicy,
  type Proposal,
  canonicalJson,
  computeProposalDigest,
  loadAndValidatePolicy,
} from './model';
import { buildSummary, formatTextReport, isMajorUpdate } from './report';
import { SourceManager, compareSemver, resolveDependencyItem } from './sources';

describe('Model & Policy', () => {
  test('canonicalJson produces deterministic sorted key string', () => {
    const a = { b: 2, a: 1, c: { y: 20, x: 10 } };
    const b = { c: { x: 10, y: 20 }, a: 1, b: 2 };
    expect(canonicalJson(a)).toBe(canonicalJson(b));
    expect(canonicalJson(a)).toBe('{"a":1,"b":2,"c":{"x":10,"y":20}}');
  });

  test('computeProposalDigest computes SHA-256 over sorted inputs and updates', () => {
    const inputs = [
      { path: 'b.json', sha256: '2222' },
      { path: 'a.json', sha256: '1111' },
    ];
    const updates = [
      { id: 'npm:b', from: '1.0.0', to: '2.0.0' },
      { id: 'npm:a', from: '1.0.0', to: '2.0.0' },
    ];
    const digest1 = computeProposalDigest(inputs, updates);
    const digest2 = computeProposalDigest(
      [...inputs].reverse(),
      [...updates].reverse(),
    );
    expect(digest1).toBe(digest2);
    expect(digest1).toMatch(/^[a-f0-9]{64}$/);
  });

  test('loadAndValidatePolicy parses valid policy and enforces non-empty reasons', () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'policy-test-'));
    try {
      const validPolicy = {
        schemaVersion: 1,
        holds: [
          { id: 'npm:pkg', value: '^1.0.0', reason: 'Compat with legacy' },
        ],
      };
      fs.writeFileSync(
        path.join(tmpDir, 'dependency-policy.json'),
        JSON.stringify(validPolicy),
      );
      const p = loadAndValidatePolicy(tmpDir);
      expect(p.schemaVersion).toBe(1);
      expect(p.holds.length).toBe(1);
      expect(p.holds[0].id).toBe('npm:pkg');

      // Empty reason rejection
      const invalidPolicy = {
        schemaVersion: 1,
        holds: [{ id: 'npm:pkg', value: '^1.0.0', reason: '   ' }],
      };
      fs.writeFileSync(
        path.join(tmpDir, 'dependency-policy.json'),
        JSON.stringify(invalidPolicy),
      );
      expect(() => loadAndValidatePolicy(tmpDir)).toThrow(
        'mandatory non-empty text',
      );

      // Duplicate ID rejection
      const dupPolicy = {
        schemaVersion: 1,
        holds: [
          { id: 'npm:pkg', value: '^1.0.0', reason: 'R1' },
          { id: 'npm:pkg', value: '^1.0.0', reason: 'R2' },
        ],
      };
      fs.writeFileSync(
        path.join(tmpDir, 'dependency-policy.json'),
        JSON.stringify(dupPolicy),
      );
      expect(() => loadAndValidatePolicy(tmpDir)).toThrow('Duplicate hold id');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

describe('Sources & Version Logic', () => {
  test('compareSemver orders correctly', () => {
    expect(compareSemver('1.0.0', '1.0.1')).toBe(-1);
    expect(compareSemver('2.0.0', '1.9.9')).toBe(1);
    expect(compareSemver('^1.2.3', '1.2.3')).toBe(0);
    expect(compareSemver('v2.5.0', '2.5.0')).toBe(0);
  });

  test('compareSemver pads partial versions and ranges', () => {
    expect(compareSemver('7.0.1', '7')).toBe(1);
    expect(compareSemver('2.2.0', 'v2')).toBe(1);
    expect(compareSemver('1.0.229', '1.0')).toBe(1);
    expect(compareSemver('4.1.0', '4.1')).toBe(0);
    expect(compareSemver('=9.8.2', '9.8.1')).toBe(1);
  });

  test('resolves the pinned Bun runtime as an actionable update', async () => {
    const mockFetch = async (url: string) => {
      if (url.includes('api.github.com/repos/oven-sh/bun/releases')) {
        return new Response(
          JSON.stringify([
            { tag_name: 'bun-v1.5.0', draft: false, prerelease: false },
          ]),
        );
      }
      return new Response('Not Found', { status: 404 });
    };
    const sources = new SourceManager({ fetch: mockFetch });
    const makeItem = (
      declared: string,
      status: DependencyItem['status'] = 'current',
    ): DependencyItem => ({
      id: 'tool:bun',
      ecosystem: 'tool',
      name: 'bun-cli',
      locations: ['package.json (packageManager)'],
      declared,
      resolved: declared,
      latest: null,
      status,
    });

    const update = makeItem('1.4.5');
    await resolveDependencyItem(update, sources);
    expect(update.status).toBe('update');
    expect(update.latest).toBe('1.5.0');
    expect(update.latestDisplay).toBe('1.5.0');

    const current = makeItem('1.5.0');
    await resolveDependencyItem(current, sources);
    expect(current.status).toBe('current');

    const ahead = makeItem('1.6.0');
    await resolveDependencyItem(ahead, sources);
    expect(ahead.status).toBe('ahead');

    const channel = makeItem('latest', 'moving');
    await resolveDependencyItem(channel, sources);
    expect(channel.status).toBe('moving');
    expect(channel.latestDisplay).toBe('latest (1.5.0)');

    const mismatch = makeItem('1.4.5', 'error');
    await resolveDependencyItem(mismatch, sources);
    expect(mismatch.status).toBe('error');
  });

  test('isMajorUpdate detects major version changes', () => {
    expect(isMajorUpdate('^9.3.1', '10.0.0')).toBe(true);
    expect(isMajorUpdate('1.5.0', '1.6.0')).toBe(false);
    expect(isMajorUpdate('0.13.3', '0.14.0')).toBe(false);
  });

  test('SourceManager resolves npm and crates with mocked fetch', async () => {
    const mockFetch = async (url: string) => {
      if (url.includes('registry.npmjs.org/wrangler')) {
        return new Response(
          JSON.stringify({
            'dist-tags': { latest: '3.120.0' },
            versions: { '3.119.0': {}, '3.120.0': {} },
          }),
        );
      }
      if (url.includes('crates.io/api/v1/crates/serde')) {
        return new Response(
          JSON.stringify({
            versions: [
              { num: '1.0.218', yanked: false },
              { num: '1.0.219', yanked: false },
            ],
          }),
        );
      }
      return new Response('Not Found', { status: 404 });
    };

    const sources = new SourceManager({ fetch: mockFetch });
    const npmRes = await sources.resolveNpm('wrangler');
    expect(npmRes.latest).toBe('3.120.0');

    const cratesRes = await sources.resolveCrates('serde');
    expect(cratesRes.latest).toBe('1.0.219');
  });

  test('SourceManager compares full semver strings for GitHub Actions', async () => {
    const mockFetch = async (url: string) => {
      if (url.includes('api.github.com/repos/actions/setup-java/releases')) {
        return new Response(
          JSON.stringify([
            { tag_name: 'v6.1.0', draft: false, prerelease: false },
            { tag_name: 'v6.0.1', draft: false, prerelease: false },
          ]),
        );
      }
      return new Response('Not Found', { status: 404 });
    };

    const sources = new SourceManager({ fetch: mockFetch });
    const items: DependencyItem[] = [
      {
        id: 'action:.github/workflows/ci.yml#actions/setup-java',
        ecosystem: 'action',
        name: 'actions/setup-java',
        locations: ['.github/workflows/ci.yml:10'],
        declared: 'v6.0.1',
        resolved: 'v6.0.1',
        latest: null,
        status: 'current',
      },
    ];

    await resolveDependencyItem(items[0], sources);
    expect(items[0].latest).toBe('6.1.0');
    expect(items[0].status).toBe('update');

    // Same version remains current
    const currentItems: DependencyItem[] = [
      {
        id: 'action:.github/workflows/ci.yml#actions/setup-java',
        ecosystem: 'action',
        name: 'actions/setup-java',
        locations: ['.github/workflows/ci.yml:10'],
        declared: 'v6.1.0',
        resolved: 'v6.1.0',
        latest: null,
        status: 'current',
      },
    ];
    await resolveDependencyItem(currentItems[0], sources);
    expect(currentItems[0].status).toBe('current');
  });

  test('resolves artifact checksums and Android toolchain revisions', async () => {
    const ktlintSha =
      'a3fd620207d5c40da6ca789b95e7f823c54e854b7fade7f613e91096a3706d75';
    const jadxSha =
      '1755e4242ade71a077438723689f404079592ec7764a3a0572f98fef94ebcbf3';
    const mockFetch = async (url: string) => {
      if (url.includes('repos/pinterest/ktlint/releases')) {
        return new Response(
          JSON.stringify([
            {
              tag_name: '1.8.0',
              draft: false,
              prerelease: false,
              assets: [
                {
                  name: 'ktlint',
                  browser_download_url:
                    'https://github.com/ktlint/ktlint/releases/download/1.8.0/ktlint',
                  digest: `sha256:${ktlintSha}`,
                },
              ],
            },
          ]),
        );
      }
      if (url.includes('repos/skylot/jadx/releases')) {
        return new Response(
          JSON.stringify([
            {
              tag_name: 'v1.5.7',
              draft: false,
              prerelease: false,
              assets: [
                {
                  name: 'jadx-1.5.7.zip',
                  browser_download_url:
                    'https://github.com/skylot/jadx/releases/download/v1.5.7/jadx-1.5.7.zip',
                  digest: `sha256:${jadxSha}`,
                },
              ],
            },
          ]),
        );
      }
      if (url.includes('repository2-3.xml')) {
        return new Response(`
          <remotePackage path="ndk;28.2.13676358">
            <revision><major>28</major><minor>2</minor><micro>13676358</micro></revision>
            <channelRef ref="channel-0"/>
          </remotePackage>
          <remotePackage path="ndk;30.0.16248370">
            <revision><major>30</major><minor>0</minor><micro>16248370</micro></revision>
            <channelRef ref="channel-0"/>
          </remotePackage>
          <remotePackage path="build-tools;36.0.0">
            <revision><major>36</major><minor>0</minor><micro>0</micro></revision>
            <channelRef ref="channel-0"/>
          </remotePackage>
          <remotePackage path="build-tools;37.0.0">
            <revision><major>37</major><minor>0</minor><micro>0</micro></revision>
            <channelRef ref="channel-0"/>
          </remotePackage>
          <remotePackage path="build-tools;38.0.0-rc1">
            <revision><major>38</major><minor>0</minor><micro>0</micro></revision>
            <channelRef ref="channel-1"/>
          </remotePackage>
        `);
      }
      return new Response('Not Found', { status: 404 });
    };
    const sources = new SourceManager({ fetch: mockFetch });
    const ktlint: DependencyItem = {
      id: 'tool:ktlint',
      ecosystem: 'tool',
      name: 'ktlint',
      locations: ['scripts/audit.ts'],
      declared: '1.5.0',
      resolved: '1.5.0',
      latest: null,
      status: 'current',
    };
    const jadx: DependencyItem = {
      id: 'tool:apk-lab/jadx',
      ecosystem: 'tool',
      name: 'jadx',
      locations: ['tools/apk-lab/tools.lock.json'],
      declared: '1.5.6',
      resolved: '1.5.6',
      latest: null,
      latestUrl:
        'https://github.com/skylot/jadx/releases/download/v1.5.6/jadx-1.5.6.zip',
      latestExecutablePath: 'bin/jadx',
      latestProbePattern: '1\\.5\\.6',
      status: 'current',
    };
    const ndk: DependencyItem = {
      id: 'tool:ndk',
      ecosystem: 'tool',
      name: 'android-ndk',
      locations: ['patches/build.gradle.kts'],
      declared: 'r28c (28.2.13676358)',
      resolved: 'r28c (28.2.13676358)',
      latest: null,
      status: 'current',
    };
    const buildTools: DependencyItem = {
      id: 'tool:android-build-tools',
      ecosystem: 'tool',
      name: 'android-build-tools',
      locations: ['tools/apk-lab/src/tools.rs'],
      declared: '36.0.0',
      resolved: '36.0.0',
      latest: null,
      status: 'current',
    };

    await resolveDependencyItem(ktlint, sources);
    await resolveDependencyItem(jadx, sources);
    await resolveDependencyItem(ndk, sources);
    await resolveDependencyItem(buildTools, sources);

    expect(ktlint.status).toBe('update');
    expect(ktlint.latest).toBe('1.8.0');
    expect(ktlint.latestAsset).toBe(ktlintSha);

    expect(jadx.status).toBe('update');
    expect(jadx.latest).toBe('1.5.7');
    expect(jadx.latestAsset).toBe(jadxSha);
    expect(jadx.latestUrl).toContain('v1.5.7/jadx-1.5.7.zip');
    expect(jadx.latestExecutablePath).toBe('bin/jadx');
    expect(jadx.latestProbePattern).toBe('1\\.5\\.7');

    expect(ndk.status).toBe('update');
    expect(ndk.latest).toBe('30.0.16248370');
    expect(ndk.latestLabel).toBe('r30');

    expect(buildTools.status).toBe('update');
    expect(buildTools.latest).toBe('37.0.0');
  });
});

describe('Inventory Collection', () => {
  test('collectInventory extracts items and marks holds accurately', () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'inv-test-'));
    try {
      const packageJson = {
        name: 'test-root',
        devDependencies: {
          oxlint: '^1.87.0',
          prettier: '^3.9.9',
        },
      };
      fs.writeFileSync(
        path.join(tmpDir, 'package.json'),
        JSON.stringify(packageJson, null, 2),
      );

      const policy: DependencyPolicy = {
        schemaVersion: 1,
        holds: [
          {
            id: 'npm:package.json#devDependencies/prettier',
            value: '^3.9.9',
            reason: 'Fixed style',
          },
        ],
      };
      fs.writeFileSync(
        path.join(tmpDir, 'dependency-policy.json'),
        JSON.stringify(policy, null, 2),
      );

      const { items, inputs } = collectInventory(tmpDir, policy);
      expect(items.length).toBe(2);
      expect(inputs.some((i) => i.path === 'package.json')).toBe(true);

      const oxlintItem = items.find((i) => i.name === 'oxlint');
      expect(oxlintItem?.status).toBe('current');

      const prettierItem = items.find((i) => i.name === 'prettier');
      expect(prettierItem?.status).toBe('held');
      expect(prettierItem?.reason).toBe('Fixed style');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
  test('collectInventory extracts CMake FetchContent and Gradle catalog dependencies', () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'inv-cmake-test-'));
    try {
      const cmakeDir = path.join(tmpDir, 'extensions', 'adobescan-pdf-native');
      fs.mkdirSync(cmakeDir, { recursive: true });
      fs.writeFileSync(
        path.join(cmakeDir, 'CMakeLists.txt'),
        `FetchContent_Declare(
    libjpeg-turbo
    URL https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/3.2.0/libjpeg-turbo-3.2.0.tar.gz
    URL_HASH SHA256=6f30092cef9fb839779646608f4ee14ae3cbac989c47fa05e841b0841f09878e
)
FetchContent_Declare(
    tesseract
    URL https://github.com/tesseract-ocr/tesseract/archive/refs/tags/5.5.1.tar.gz
    URL_HASH SHA256=a7a3f2a7420cb6a6a94d80c24163e183cf1d2f1bed2df3bbc397c81808a57237
)`,
      );

      const gradleDir = path.join(tmpDir, 'gradle');
      fs.mkdirSync(gradleDir, { recursive: true });
      fs.writeFileSync(
        path.join(gradleDir, 'libs.versions.toml'),
        `[versions]
pdfbox-android = "2.0.27.0"

[libraries]
pdfbox-android = { module = "com.tom-roush:pdfbox-android", version.ref = "pdfbox-android" }
`,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const { items, inputs } = collectInventory(tmpDir, policy);

      const jpegItem = items.find((i) => i.name === 'libjpeg-turbo');
      expect(jpegItem?.declared).toBe('3.2.0');
      expect(jpegItem?.ecosystem).toBe('cmake');

      const tessItem = items.find((i) => i.name === 'tesseract');
      expect(tessItem?.declared).toBe('5.5.1');
      expect(tessItem?.ecosystem).toBe('cmake');

      const pdfboxItem = items.find(
        (i) => i.name === 'com.tom-roush:pdfbox-android',
      );
      expect(pdfboxItem?.declared).toBe('2.0.27.0');
      expect(pdfboxItem?.ecosystem).toBe('gradle');

      expect(
        inputs.some(
          (i) => i.path === 'extensions/adobescan-pdf-native/CMakeLists.txt',
        ),
      ).toBe(true);
      expect(inputs.some((i) => i.path === 'gradle/libs.versions.toml')).toBe(
        true,
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('tracks the root packageManager Bun pin and flags workflow drift', () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'inv-bun-test-'));
    try {
      const write = (rel: string, body: string) => {
        const target = path.join(tmpDir, rel);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, body);
      };
      write(
        'package.json',
        JSON.stringify(
          { name: 'test-root', packageManager: 'bun@1.4.5' },
          null,
          2,
        ) + '\n',
      );
      write(
        '.github/workflows/ci.yml',
        'steps:\n  - uses: oven-sh/setup-bun@v2.2.0\n    with:\n      bun-version: 1.4.5\n',
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const bun = collectInventory(tmpDir, policy).items.find(
        (item) => item.id === 'tool:bun',
      );
      expect(bun?.declared).toBe('1.4.5');
      expect(bun?.resolved).toBe('1.4.5');
      expect(bun?.status).toBe('current');
      expect(bun?.locations).toContain('package.json (packageManager)');

      write(
        '.github/workflows/ci.yml',
        'steps:\n  - uses: oven-sh/setup-bun@v2.2.0\n    with:\n      bun-version: latest\n',
      );
      const drifted = collectInventory(tmpDir, policy).items.find(
        (item) => item.id === 'tool:bun',
      );
      expect(drifted?.status).toBe('error');
      expect(drifted?.reason).toContain('bun-version');
      expect(drifted?.reason).toContain('latest');

      write(
        'package.json',
        JSON.stringify(
          { name: 'test-root', packageManager: 'npm@10.0.0' },
          null,
          2,
        ) + '\n',
      );
      expect(() => collectInventory(tmpDir, policy)).toThrow('must pin Bun');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('parseBunLock parses mock bun.lock with trailing commas', () => {
    const mockBunLock = `{
      "lockfileVersion": 2,
      "workspaces": {
        "": {
          "devDependencies": {
            "prettier": "^3.9.9",
          },
        },
      },
      "packages": {
        "prettier": ["prettier@3.9.9", "", {}, "sha512-abc",],
      },
    }`;
    const parsed = parseBunLock(mockBunLock);
    expect(parsed.lockfileVersion).toBe(2);
    const pkgs = parseBunLockPackages(mockBunLock);
    expect(pkgs.prettier).toBeDefined();
  });

  test('extractKtlintVersion parses const KTLINT_VERSION declaration', () => {
    const mockAudit = `// ktlint pinned version
const KTLINT_VERSION = '1.6.0';
const KTLINT_SHA256 = 'abc...';
`;
    expect(extractKtlintVersion(mockAudit)).toBe('1.6.0');
  });
});

describe('Report Formatting', () => {
  test('buildSummary counts item statuses', () => {
    const items: DependencyItem[] = [
      {
        id: '1',
        ecosystem: 'npm',
        name: 'a',
        locations: [],
        declared: '1',
        resolved: '1',
        latest: '2',
        status: 'update',
      },
      {
        id: '2',
        ecosystem: 'npm',
        name: 'b',
        locations: [],
        declared: '1',
        resolved: '1',
        latest: '1',
        status: 'current',
      },
      {
        id: '3',
        ecosystem: 'npm',
        name: 'c',
        locations: [],
        declared: '1',
        resolved: '1',
        latest: '2',
        status: 'held',
        reason: 'Stay on 1',
      },
    ];
    const s = buildSummary(items, 150);
    expect(s.total).toBe(3);
    expect(s.update).toBe(1);
    expect(s.current).toBe(1);
    expect(s.held).toBe(1);
    expect(s.durationMs).toBe(150);
  });

  test('formatTextReport outputs formatted report with proposal digest', () => {
    const items: DependencyItem[] = [
      {
        id: 'npm:package.json#devDependencies/foo',
        ecosystem: 'npm',
        name: 'foo',
        locations: ['package.json'],
        declared: '^1.0.0',
        resolved: '1.0.0',
        latest: '2.0.0',
        status: 'update',
      },
    ];
    const report = {
      schemaVersion: 1 as const,
      items,
      summary: buildSummary(items, 100),
      proposal: {
        schemaVersion: 1 as const,
        inputs: [{ path: 'package.json', sha256: 'abc' }],
        updates: [
          {
            id: 'npm:package.json#devDependencies/foo',
            from: '1.0.0',
            to: '2.0.0',
          },
        ],
        digest:
          '1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef',
      },
    };
    const output = formatTextReport(report, false);
    expect(output).toContain('REPOSITORY DEPENDENCY FRESHNESS AUDIT');
    expect(output).toContain('foo');
    expect(output).toContain('MAJOR UPDATE');
    expect(output).toContain('Proposal SHA-256 Digest: 1234567890abcdef');
  });

  test('formatTextReport formats unsupported dependencies with warning indicator', () => {
    const items: DependencyItem[] = [
      {
        id: 'tool:custom',
        ecosystem: 'tool',
        name: 'custom-tool',
        locations: ['config.json'],
        declared: '1.0.0',
        resolved: '1.0.0',
        latest: null,
        status: 'unsupported',
        reason: 'Platform not supported',
      },
    ];
    const report = {
      schemaVersion: 1 as const,
      items,
      summary: buildSummary(items, 50),
      proposal: null,
    };
    const output = formatTextReport(report, false);
    expect(output).toContain('custom-tool');
    expect(output).toContain('unsupported');
    expect(output).toContain('UNSUPPORTED: 1');
  });
});

describe('Staging Seed', () => {
  test('copies tracked sources and explicit inputs without local artifacts', () => {
    const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'seed-repo-'));
    const staging = fs.mkdtempSync(path.join(os.tmpdir(), 'seed-staging-'));
    const inputStaging = fs.mkdtempSync(
      path.join(os.tmpdir(), 'seed-input-staging-'),
    );
    const env = {
      ...process.env,
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_NOSYSTEM: '1',
    };
    try {
      fs.writeFileSync(path.join(repo, 'package.json'), '{}');
      fs.writeFileSync(path.join(repo, 'bun.lock'), '{}');
      expect(spawnSync('git', ['init', '-q'], { cwd: repo, env }).status).toBe(
        0,
      );
      expect(
        spawnSync('git', ['add', '--', 'package.json', 'bun.lock'], {
          cwd: repo,
          env,
        }).status,
      ).toBe(0);

      fs.mkdirSync(path.join(repo, 'apk'));
      fs.writeFileSync(path.join(repo, 'apk', 'local.apk'), 'artifact');
      fs.mkdirSync(path.join(repo, '.apk-lab'));
      fs.writeFileSync(path.join(repo, '.apk-lab', 'state.bin'), 'artifact');
      fs.writeFileSync(path.join(repo, 'untracked.txt'), 'local input');

      expect(seedStagingDirectory(repo, staging).sort()).toEqual([
        'bun.lock',
        'package.json',
      ]);
      expect(fs.existsSync(path.join(staging, 'apk'))).toBe(false);
      expect(fs.existsSync(path.join(staging, '.apk-lab'))).toBe(false);
      expect(fs.existsSync(path.join(staging, 'untracked.txt'))).toBe(false);

      seedStagingDirectory(repo, inputStaging, ['untracked.txt']);
      expect(
        fs.readFileSync(path.join(inputStaging, 'untracked.txt'), 'utf8'),
      ).toBe('local input');
    } finally {
      fs.rmSync(repo, { recursive: true, force: true });
      fs.rmSync(staging, { recursive: true, force: true });
      fs.rmSync(inputStaging, { recursive: true, force: true });
    }
  });
});

describe('Transactional Apply', () => {
  test('applyApprovedProposal validates digest and updates manifest in staging before writing', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-test-'));
    try {
      const pkgJson = {
        name: 'test-app',
        devDependencies: {
          wrangler: '^3.111.0',
        },
      };
      const pkgPath = path.join(tmpDir, 'package.json');
      fs.writeFileSync(pkgPath, JSON.stringify(pkgJson, null, 2) + '\n');
      const sha256 = fs.readFileSync(pkgPath);
      const fileSha = Bun.SHA256.hash(sha256, 'hex');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: 'package.json', sha256: fileSha }],
        updates: [
          {
            id: 'npm:package.json#devDependencies/wrangler',
            from: '3.111.0',
            to: '3.120.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };

      // Mock runner that simulates bun install --lockfile-only
      const mockRunner = async (_cmd: string, args: string[], cwd: string) => {
        if (args.includes('--lockfile-only')) {
          fs.writeFileSync(path.join(cwd, 'bun.lock'), 'mock lock');
        }
        if (args.includes('--package-lock-only')) {
          fs.writeFileSync(
            path.join(cwd, 'package-lock.json'),
            'mock npm lock',
          );
        }
        return { code: 0, stdout: 'ok', stderr: '' };
      };

      const res = await applyApprovedProposal(tmpDir, proposal, policy, {
        runner: mockRunner,
      });
      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain('package.json');

      const updatedJson = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
      expect(updatedJson.devDependencies.wrangler).toBe('^3.120.0');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal rejects tampered inputs or mismatching digest', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-tamper-'));
    try {
      const pkgPath = path.join(tmpDir, 'package.json');
      fs.writeFileSync(pkgPath, '{"name":"tamper"}');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: 'package.json', sha256: 'wrong-hash' }],
        updates: [
          { id: 'npm:package.json#devDependencies/x', from: '1', to: '2' },
        ],
        digest: 'wrong-digest',
      };
      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };

      expect(applyApprovedProposal(tmpDir, proposal, policy)).rejects.toThrow(
        'Proposal digest verification failed',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal rejects update for held dependency', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-held-'));
    try {
      const pkgPath = path.join(tmpDir, 'package.json');
      fs.writeFileSync(pkgPath, '{"name":"held"}');
      const sha256 = Bun.SHA256.hash(fs.readFileSync(pkgPath), 'hex');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: 'package.json', sha256 }],
        updates: [
          {
            id: 'npm:package.json#devDependencies/held-pkg',
            from: '1.0.0',
            to: '2.0.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = {
        schemaVersion: 1,
        holds: [
          {
            id: 'npm:package.json#devDependencies/held-pkg',
            value: '1.0.0',
            reason: 'Do not touch',
          },
        ],
      };

      expect(applyApprovedProposal(tmpDir, proposal, policy)).rejects.toThrow(
        'Cannot apply update to held dependency',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal correctly updates version catalog entry in gradle/libs.versions.toml', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-gradle-'));
    try {
      const gradleDir = path.join(tmpDir, 'gradle');
      fs.mkdirSync(gradleDir, { recursive: true });
      const catalogRel = 'gradle/libs.versions.toml';
      const catalogPath = path.join(tmpDir, catalogRel);
      const initialContent =
        '[versions]\nagp = "8.2.0"\nkotlin = "1.9.20"\n\n[libraries]\n';
      fs.writeFileSync(catalogPath, initialContent);
      const sha256 = Bun.SHA256.hash(fs.readFileSync(catalogPath), 'hex');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: catalogRel, sha256 }],
        updates: [
          {
            id: `gradle:${catalogRel}#versions/agp`,
            from: '8.2.0',
            to: '8.3.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };

      const res = await applyApprovedProposal(tmpDir, proposal, policy);
      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain(catalogRel);

      const updatedContent = fs.readFileSync(catalogPath, 'utf8');
      expect(updatedContent).toContain('agp = "8.3.0"');
      expect(updatedContent).toContain('kotlin = "1.9.20"');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
  test('applyApprovedProposal correctly updates CMake FetchContent block', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-cmake-'));
    try {
      const cmakeRel = 'extensions/adobescan-pdf-native/CMakeLists.txt';
      const cmakePath = path.join(tmpDir, cmakeRel);
      fs.mkdirSync(path.dirname(cmakePath), { recursive: true });
      const initialContent = `FetchContent_Declare(
    tesseract
    URL https://github.com/tesseract-ocr/tesseract/archive/refs/tags/5.5.1.tar.gz
    URL_HASH SHA256=a7a3f2a7420cb6a6a94d80c24163e183cf1d2f1bed2df3bbc397c81808a57237
)`;
      fs.writeFileSync(cmakePath, initialContent);
      const sha256 = Bun.SHA256.hash(fs.readFileSync(cmakePath), 'hex');

      const newFakeSha =
        '111122223333444455556666777788889999aaaabbbbccccddddeeeeffff0000';
      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: cmakeRel, sha256 }],
        updates: [
          {
            id: `cmake:${cmakeRel}#fetch/tesseract`,
            from: '5.5.1',
            to: '5.5.3',
            assetIdentity: newFakeSha,
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const res = await applyApprovedProposal(tmpDir, proposal, policy);
      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain(cmakeRel);

      const updatedContent = fs.readFileSync(cmakePath, 'utf8');
      expect(updatedContent).toContain(
        'https://github.com/tesseract-ocr/tesseract/archive/refs/tags/5.5.3.tar.gz',
      );
      expect(updatedContent).toContain(`URL_HASH SHA256=${newFakeSha}`);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal falls back to from-to replacement for non-named FetchContent package without assetIdentity', async () => {
    const tmpDir = fs.mkdtempSync(
      path.join(os.tmpdir(), 'apply-cmake-fallback-'),
    );
    const origFetch = globalThis.fetch;
    try {
      const cmakeRel = 'extensions/adobescan-pdf-native/CMakeLists.txt';
      const cmakePath = path.join(tmpDir, cmakeRel);
      fs.mkdirSync(path.dirname(cmakePath), { recursive: true });
      const initialContent = `FetchContent_Declare(
    foo
    URL https://example.com/packages/foo-1.0.0.tar.gz
    URL_HASH SHA256=1111111111111111111111111111111111111111111111111111111111111111
)`;
      fs.writeFileSync(cmakePath, initialContent);
      const sha256 = Bun.SHA256.hash(fs.readFileSync(cmakePath), 'hex');

      const fakeArchiveContent = 'archive-payload-bytes';
      const expectedSha256 = Bun.SHA256.hash(fakeArchiveContent, 'hex');
      globalThis.fetch = (async () => {
        return new Response(fakeArchiveContent, { status: 200 });
      }) as typeof fetch;

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: cmakeRel, sha256 }],
        updates: [
          {
            id: `cmake:${cmakeRel}#fetch/foo`,
            from: '1.0.0',
            to: '2.0.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const res = await applyApprovedProposal(tmpDir, proposal, policy);
      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain(cmakeRel);

      const updatedContent = fs.readFileSync(cmakePath, 'utf8');
      expect(updatedContent).toContain(
        'https://example.com/packages/foo-2.0.0.tar.gz',
      );
      expect(updatedContent).toContain(`URL_HASH SHA256=${expectedSha256}`);
    } finally {
      globalThis.fetch = origFetch;
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal correctly updates scoped NPM package in package.json', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-scoped-npm-'));
    try {
      const pkgJson = {
        name: 'test-app',
        devDependencies: {
          '@semantic-release/github': '^10.0.0',
        },
      };
      const pkgPath = path.join(tmpDir, 'package.json');
      fs.writeFileSync(pkgPath, JSON.stringify(pkgJson, null, 2) + '\n');
      const fileSha = Bun.SHA256.hash(fs.readFileSync(pkgPath), 'hex');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: 'package.json', sha256: fileSha }],
        updates: [
          {
            id: 'npm:package.json#devDependencies/@semantic-release/github',
            from: '10.0.0',
            to: '10.1.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const mockRunner = async (_cmd: string, args: string[], cwd: string) => {
        if (args.includes('--package-lock-only')) {
          fs.writeFileSync(
            path.join(cwd, 'package-lock.json'),
            'mock npm lock',
          );
        }
        return { code: 0, stdout: 'ok', stderr: '' };
      };

      const res = await applyApprovedProposal(tmpDir, proposal, policy, {
        runner: mockRunner,
      });
      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain('package.json');

      const updatedJson = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
      expect(updatedJson.devDependencies['@semantic-release/github']).toBe(
        '^10.1.0',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applies a Bun runtime pin and keeps workflow pins aligned', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-bun-'));
    try {
      const pkgPath = path.join(tmpDir, 'package.json');
      const workflowPath = path.join(tmpDir, '.github/workflows/ci.yml');
      fs.mkdirSync(path.dirname(workflowPath), { recursive: true });
      fs.writeFileSync(
        pkgPath,
        JSON.stringify(
          {
            name: 'test-root',
            packageManager: 'bun@1.4.5',
            devDependencies: { prettier: '^3.10.1' },
          },
          null,
          2,
        ) + '\n',
      );
      fs.writeFileSync(
        workflowPath,
        'steps:\n  - uses: oven-sh/setup-bun@v2.2.0\n    with:\n      bun-version: 1.4.5\n',
      );

      const inputs = [pkgPath, workflowPath].map((absolute) => ({
        path: path.relative(tmpDir, absolute).split(path.sep).join('/'),
        sha256: Bun.SHA256.hash(fs.readFileSync(absolute), 'hex'),
      }));
      const proposal: Proposal = {
        schemaVersion: 1,
        inputs,
        updates: [{ id: 'tool:bun', from: '1.4.5', to: '1.5.0' }],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const runnerCalls: string[] = [];
      const res = await applyApprovedProposal(
        tmpDir,
        proposal,
        { schemaVersion: 1, holds: [] },
        {
          runner: async (cmd, args) => {
            runnerCalls.push([cmd, ...args].join(' '));
            return { code: 0, stdout: '', stderr: '' };
          },
        },
      );

      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain('package.json');
      expect(res.changedFiles).toContain('.github/workflows/ci.yml');
      expect(runnerCalls).toEqual([]);
      expect(fs.readFileSync(pkgPath, 'utf8')).toContain(
        '"packageManager": "bun@1.5.0"',
      );
      expect(fs.readFileSync(workflowPath, 'utf8')).toContain(
        'bun-version: 1.5.0',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applyApprovedProposal throws error on invalid NPM package ID structure', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-npm-invalid-'));
    try {
      const pkgJson = { name: 'test-app', dependencies: { foo: '1.0.0' } };
      const pkgPath = path.join(tmpDir, 'package.json');
      fs.writeFileSync(pkgPath, JSON.stringify(pkgJson, null, 2) + '\n');
      const sha256 = Bun.SHA256.hash(fs.readFileSync(pkgPath), 'hex');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [{ path: 'package.json', sha256 }],
        updates: [
          {
            id: 'npm:package.json#no-slash-here',
            from: '1.0.0',
            to: '2.0.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      await expect(
        applyApprovedProposal(tmpDir, proposal, policy),
      ).rejects.toThrow(
        'Invalid NPM package ID structure: npm:package.json#no-slash-here',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('applies artifact and toolchain updates with coordinated metadata', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-tools-'));
    try {
      const write = (rel: string, body: string) => {
        const target = path.join(tmpDir, rel);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, body);
      };
      write(
        'gradle/wrapper/gradle-wrapper.properties',
        'distributionSha256Sum=acd53f1edaf02f1a8ff99879f8a34b302661a057d9b063ae9e35b552f804d20a\n' +
          'distributionUrl=https\\://services.gradle.org/distributions/gradle-9.7.1-bin.zip\n',
      );
      write(
        'scripts/audit.ts',
        "const KTLINT_VERSION = '1.5.0';\nconst KTLINT_SHA256 =\n  'a16be01dcc480aab2f55f444b620142152f66e31564b3b9376506d624c28a2ad';\n",
      );
      write(
        'tools/apk-lab/tools.lock.json',
        JSON.stringify(
          {
            tools: {
              jadx: {
                version: '1.5.6',
                url: 'https://github.com/skylot/jadx/releases/download/v1.5.6/jadx-1.5.6.zip',
                sha256: 'old',
                executablePath: 'bin/jadx',
                versionProbe: { expectedPattern: '1\\.5\\.6' },
              },
            },
          },
          null,
          2,
        ) + '\n',
      );
      write(
        'patches/build.gradle.kts',
        'val REQUIRED_NDK_VERSION = "28.2.13676358"\n',
      );
      write(
        'tools/apk-lab/src/tools.rs',
        'let x = build_tools.join("36.0.0").join(bin_name);\n',
      );
      write(
        '.github/workflows/ci.yml',
        "jobs:\n  x:\n    steps:\n      - uses: actions/setup-java@v6.3.0\n        with:\n          java-version: '21'\n      - uses: nttld/setup-ndk@v1\n        with:\n          ndk-version: r28c\n      - uses: dtolnay/rust-toolchain@1.97.1\n",
      );
      write(
        'extensions/nhl-adblock-native/rust-toolchain.toml',
        '[toolchain]\nchannel = "1.97.1"\n',
      );
      write(
        'extensions/extension/src/main/java/app/aidan/extension/aftership/OsmMapView.java',
        '// unpkg.com/leaflet@1.9.4/dist/leaflet.js\n',
      );
      write(
        'tools/apk-lab/Cargo.toml',
        '[package]\nname = "apk-lab"\nversion = "0.1.0"\n\n[dependencies]\nzip = "=9.8.1"\n',
      );
      write(
        'tools/apk-lab/Cargo.lock',
        'version = 4\n\n[[package]]\nname = "zip"\nversion = "9.8.1"\n',
      );

      const inputs = [
        'gradle/wrapper/gradle-wrapper.properties',
        'scripts/audit.ts',
        'tools/apk-lab/tools.lock.json',
        'patches/build.gradle.kts',
        'tools/apk-lab/src/tools.rs',
        '.github/workflows/ci.yml',
        'extensions/nhl-adblock-native/rust-toolchain.toml',
        'extensions/extension/src/main/java/app/aidan/extension/aftership/OsmMapView.java',
        'tools/apk-lab/Cargo.toml',
        'tools/apk-lab/Cargo.lock',
      ].map((rel) => ({
        path: rel,
        sha256: Bun.SHA256.hash(fs.readFileSync(path.join(tmpDir, rel)), 'hex'),
      }));
      const updates = [
        {
          id: 'tool:gradle',
          from: '9.7.1',
          to: '9.9.2',
          assetIdentity:
            '971fcd7150ca4ba04852c0b2ca0d2d0ab9468e14b03473eab19528809914c76b',
        },
        {
          id: 'tool:ktlint',
          from: '1.5.0',
          to: '1.8.0',
          assetIdentity:
            'a3fd620207d5c40da6ca789b95e7f823c54e854b7fade7f613e91096a3706d75',
        },
        {
          id: 'tool:apk-lab/jadx',
          from: '1.5.6',
          to: '1.5.7',
          assetIdentity:
            '1755e4242ade71a077438723689f404079592ec7764a3a0572f98fef94ebcbf3',
          assetUrl:
            'https://github.com/skylot/jadx/releases/download/v1.5.7/jadx-1.5.7.zip',
          executablePath: 'bin/jadx',
          versionProbePattern: '1\\.5\\.7',
        },
        {
          id: 'action:actions/setup-java',
          from: 'v6.3.0',
          to: '6.4.0',
        },
        { id: 'tool:java', from: '21', to: '25' },
        { id: 'tool:rust-native', from: '1.97.1', to: '1.98.0' },
        {
          id: 'tool:ndk',
          from: 'r28c (28.2.13676358)',
          to: '30.0.16248370',
          label: 'r30',
        },
        {
          id: 'tool:android-build-tools',
          from: '36.0.0',
          to: '37.0.0',
        },
        { id: 'embedded:leaflet', from: '1.9.4', to: '1.9.5' },
        {
          id: 'crates:tools/apk-lab/Cargo.toml#dependencies/zip',
          from: '=9.8.1',
          to: '9.8.2',
        },
      ];
      const proposal: Proposal = {
        schemaVersion: 1,
        inputs,
        updates,
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      const cargoCalls: string[][] = [];
      const res = await applyApprovedProposal(
        tmpDir,
        proposal,
        { schemaVersion: 1, holds: [] },
        {
          runner: async (cmd, args) => {
            if (cmd === 'cargo') cargoCalls.push(args);
            return { code: 0, stdout: '', stderr: '' };
          },
        },
      );

      expect(res.success).toBe(true);
      expect(res.changedFiles).toContain('tools/apk-lab/Cargo.lock');
      expect(cargoCalls).toHaveLength(1);
      expect(cargoCalls[0]).toContain('--precise');
      expect(cargoCalls[0]).toContain('9.8.2');

      const wrapper = fs.readFileSync(
        path.join(tmpDir, 'gradle/wrapper/gradle-wrapper.properties'),
        'utf8',
      );
      expect(wrapper).toContain(
        'distributionUrl=https\\://services.gradle.org/distributions/gradle-9.9.2-bin.zip',
      );
      expect(wrapper).toContain(
        'distributionSha256Sum=971fcd7150ca4ba04852c0b2ca0d2d0ab9468e14b03473eab19528809914c76b',
      );

      const audit = fs.readFileSync(
        path.join(tmpDir, 'scripts/audit.ts'),
        'utf8',
      );
      expect(audit).toContain("const KTLINT_VERSION = '1.8.0';");
      expect(audit).toContain(
        "KTLINT_SHA256 = 'a3fd620207d5c40da6ca789b95e7f823c54e854b7fade7f613e91096a3706d75'",
      );

      const toolLock = JSON.parse(
        fs.readFileSync(
          path.join(tmpDir, 'tools/apk-lab/tools.lock.json'),
          'utf8',
        ),
      );
      expect(toolLock.tools.jadx.version).toBe('1.5.7');
      expect(toolLock.tools.jadx.url).toContain('v1.5.7/jadx-1.5.7.zip');
      expect(toolLock.tools.jadx.sha256).toBe(
        '1755e4242ade71a077438723689f404079592ec7764a3a0572f98fef94ebcbf3',
      );
      expect(toolLock.tools.jadx.executablePath).toBe('bin/jadx');
      expect(toolLock.tools.jadx.versionProbe.expectedPattern).toBe(
        '1\\.5\\.7',
      );

      expect(
        fs.readFileSync(path.join(tmpDir, 'patches/build.gradle.kts'), 'utf8'),
      ).toContain('REQUIRED_NDK_VERSION = "30.0.16248370"');
      expect(
        fs.readFileSync(
          path.join(tmpDir, 'tools/apk-lab/src/tools.rs'),
          'utf8',
        ),
      ).toContain('build_tools.join("37.0.0")');

      const workflow = fs.readFileSync(
        path.join(tmpDir, '.github/workflows/ci.yml'),
        'utf8',
      );
      expect(workflow).toContain('actions/setup-java@v6.4.0');
      expect(workflow).toContain("java-version: '25'");
      expect(workflow).toContain('ndk-version: r30');
      expect(workflow).toContain('dtolnay/rust-toolchain@1.98.0');

      expect(
        fs.readFileSync(
          path.join(
            tmpDir,
            'extensions/nhl-adblock-native/rust-toolchain.toml',
          ),
          'utf8',
        ),
      ).toContain('channel = "1.98.0"');
      expect(
        fs.readFileSync(
          path.join(
            tmpDir,
            'extensions/extension/src/main/java/app/aidan/extension/aftership/OsmMapView.java',
          ),
          'utf8',
        ),
      ).toContain('unpkg.com/leaflet@1.9.5/');
      expect(
        fs.readFileSync(path.join(tmpDir, 'tools/apk-lab/Cargo.toml'), 'utf8'),
      ).toContain('zip = "=9.8.2"');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('rejects proposals with no matching apply strategy', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apply-unhandled-'));
    try {
      fs.writeFileSync(
        path.join(tmpDir, 'dependency-policy.json'),
        JSON.stringify({ schemaVersion: 1, holds: [] }),
      );
      const inputs = [
        {
          path: 'dependency-policy.json',
          sha256: Bun.SHA256.hash(
            fs.readFileSync(path.join(tmpDir, 'dependency-policy.json')),
            'hex',
          ),
        },
      ];
      const proposal: Proposal = {
        schemaVersion: 1,
        inputs,
        updates: [{ id: 'tool:not-registered', from: '1', to: '2' }],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );

      await expect(
        applyApprovedProposal(tmpDir, proposal, {
          schemaVersion: 1,
          holds: [],
        }),
      ).rejects.toThrow('No apply strategy matched: tool:not-registered');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

describe('Lockfile and manifest expansion coverage', () => {
  test('compareLockfileResolutions detects version drift and accepts git prefixes', () => {
    const lock = {
      packages: {
        'node_modules/foo': { version: '2.0.0' },
        'node_modules/bar': {
          resolved: 'git+ssh://git@example.com/bar.git#abcdef1234567890',
        },
      },
    };
    expect(compareLockfileResolutions('foo', '2.0.0', lock)).toBeNull();
    expect(compareLockfileResolutions('foo', '1.0.0', lock)).toContain(
      'lockfile mismatch',
    );
    expect(compareLockfileResolutions('bar', 'abcdef1', lock)).toBeNull();
    expect(compareLockfileResolutions('missing', '1.0.0', lock)).toContain(
      'no resolution',
    );
  });

  test('replaceCargoDependencyVersion updates only the requested target section', () => {
    const manifest = [
      '[dependencies]',
      'libc = "0.2"',
      '',
      "[target.'cfg(unix)'.dependencies]",
      'libc = "0.2"',
      '',
      "[target.'cfg(windows)'.dependencies]",
      'libc = "0.2"',
      '',
    ].join('\n');
    const updated = replaceCargoDependencyVersion(
      manifest,
      "target.'cfg(unix)'.dependencies",
      'libc',
      '0.2.191',
    );
    expect(updated).toContain('[dependencies]\nlibc = "0.2"');
    expect(updated).toContain(
      '[target.\'cfg(unix)\'.dependencies]\nlibc = "0.2.191"',
    );
    expect(updated).toContain(
      '[target.\'cfg(windows)\'.dependencies]\nlibc = "0.2"',
    );
  });

  test('collectInventory parses target-specific Cargo dependencies', () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'inv-target-cargo-'));
    try {
      fs.mkdirSync(path.join(tmpDir, 'tools', 'apk-lab'), { recursive: true });
      fs.writeFileSync(
        path.join(tmpDir, 'tools', 'apk-lab', 'Cargo.toml'),
        [
          '[package]',
          'name = "apk-lab"',
          'version = "0.1.0"',
          '',
          '[dependencies]',
          'serde = "1.0.218"',
          '',
          "[target.'cfg(unix)'.dependencies]",
          'libc = "0.2"',
          '',
        ].join('\n'),
      );
      fs.writeFileSync(
        path.join(tmpDir, 'tools', 'apk-lab', 'Cargo.lock'),
        [
          'version = 4',
          '',
          '[[package]]',
          'name = "serde"',
          'version = "1.0.229"',
          '',
          '[[package]]',
          'name = "libc"',
          'version = "0.2.191"',
          '',
        ].join('\n'),
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const { items } = collectInventory(tmpDir, policy);
      const libc = items.find((item) => item.name === 'libc');
      expect(libc?.id).toBe(
        "crates:tools/apk-lab/Cargo.toml#target.'cfg(unix)'.dependencies/libc",
      );
      expect(libc?.declared).toBe('0.2');
      expect(libc?.resolved).toBe('0.2.191');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('collectInventory parses and applies literal Gradle dependencies', async () => {
    const tmpDir = fs.mkdtempSync(
      path.join(os.tmpdir(), 'inv-gradle-literal-'),
    );
    try {
      const buildRel = 'extensions/extension/build.gradle.kts';
      const buildPath = path.join(tmpDir, buildRel);
      fs.mkdirSync(path.dirname(buildPath), { recursive: true });
      fs.writeFileSync(
        buildPath,
        [
          'dependencies {',
          '    implementation("com.example:widget:1.0.0")',
          '    compileOnly(libs.gson)',
          '}',
          '',
        ].join('\n'),
      );

      const policy: DependencyPolicy = { schemaVersion: 1, holds: [] };
      const { items } = collectInventory(tmpDir, policy);
      const widget = items.find((item) => item.name === 'com.example:widget');
      expect(widget?.id).toBe(
        'gradle:extensions/extension/build.gradle.kts#dependencies/com.example:widget',
      );
      expect(widget?.declared).toBe('1.0.0');

      const proposal: Proposal = {
        schemaVersion: 1,
        inputs: [
          {
            path: buildRel,
            sha256: Bun.SHA256.hash(fs.readFileSync(buildPath), 'hex'),
          },
        ],
        updates: [
          {
            id: 'gradle:extensions/extension/build.gradle.kts#dependencies/com.example:widget',
            from: '1.0.0',
            to: '2.0.0',
          },
        ],
        digest: '',
      };
      proposal.digest = computeProposalDigest(
        proposal.inputs,
        proposal.updates,
      );
      const res = await applyApprovedProposal(tmpDir, proposal, policy);
      expect(res.success).toBe(true);
      expect(fs.readFileSync(buildPath, 'utf8')).toContain(
        'implementation("com.example:widget:2.0.0")',
      );
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});
