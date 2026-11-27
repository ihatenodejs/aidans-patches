import { spawn, spawnSync } from 'node:child_process';
import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { ndkLabelFromRevision, replaceVersionText } from './sources';
import { RUST_PROJECTS } from '../rust-projects';
import {
  type DependencyPolicy,
  type Proposal,
  type ProposalUpdate,
  computeProposalDigest,
} from './model';

export function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Replaces a dependency version inside one Cargo.toml section. Target-specific
 * tables use the same `[target.'cfg(...)'.dependencies]` header text as their
 * inventory section path, so the match stays scoped to the right table.
 */
export function replaceCargoDependencyVersion(
  content: string,
  section: string,
  pkgName: string,
  to: string,
): string {
  const headers = [`[${section}]`];
  if (section.startsWith('target.')) {
    headers.push(`[${section.replace(".'", '.').replace("'.", '.')}]`);
  }
  const header = headers.find((candidate) => content.includes(candidate));
  if (!header) {
    throw new Error(
      `Cannot update Cargo dependency: section ${headers[0]} is missing`,
    );
  }
  const headerIndex = content.indexOf(header);
  const nextSection = content.indexOf('\n[', headerIndex + header.length);
  const blockEnd = nextSection === -1 ? content.length : nextSection + 1;
  const block = content.slice(headerIndex, blockEnd);
  const safePkgName = escapeRegex(pkgName);
  const tableRegex = new RegExp(
    `(${safePkgName}\\s*=\\s*\\{[^}]*version\\s*=\\s*)"([^"]+)"`,
    'g',
  );
  const simpleRegex = new RegExp(`(${safePkgName}\\s*=\\s*)"([^"]+)"`, 'g');
  const replaceVersion = (match: string, prefix: string, oldVer: string) => {
    const exact = oldVer.startsWith('=') ? '=' : '';
    return `${prefix}"${exact}${to}"`;
  };
  const updated = tableRegex.test(block)
    ? block.replace(tableRegex, replaceVersion)
    : block.replace(simpleRegex, replaceVersion);
  if (updated === block) {
    throw new Error(
      `Cannot update Cargo dependency: ${pkgName} was not found in ${header}`,
    );
  }
  return content.slice(0, headerIndex) + updated + content.slice(blockEnd);
}

async function resolveAssetHash(
  assetIdentity: string | undefined,
  assetUrl: string | undefined,
  label: string,
): Promise<string> {
  if (assetIdentity) return assetIdentity;
  if (!assetUrl) {
    throw new Error(
      `${label} update is missing both an asset checksum and a download URL`,
    );
  }
  const res = await fetch(assetUrl);
  if (!res.ok) {
    throw new Error(`Failed to download ${assetUrl}: HTTP ${res.status}`);
  }
  return crypto
    .createHash('sha256')
    .update(Buffer.from(await res.arrayBuffer()))
    .digest('hex');
}

export type CommandRunner = (
  cmd: string,
  args: string[],
  cwd: string,
) => Promise<{ code: number; stdout: string; stderr: string }>;

export async function defaultRunner(
  cmd: string,
  args: string[],
  cwd: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
  const { promise, resolve } = Promise.withResolvers<{
    code: number;
    stdout: string;
    stderr: string;
  }>();

  const child = spawn(cmd, args, {
    cwd,
    shell: false,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, FORCE_COLOR: '0' },
  });

  let stdout = '';
  let stderr = '';

  child.stdout.on('data', (d) => (stdout += d.toString()));
  child.stderr.on('data', (d) => (stderr += d.toString()));

  child.on('close', (code) => {
    resolve({ code: code ?? 1, stdout, stderr });
  });

  child.on('error', (err) => {
    resolve({ code: 1, stdout, stderr: stderr + err.message });
  });

  return promise;
}

export interface ApplyOptions {
  runner?: CommandRunner;
}

export interface ApplyResult {
  success: boolean;
  appliedUpdates: ProposalUpdate[];
  changedFiles: string[];
  message: string;
}

const STAGING_SKIP_DIRECTORIES: Record<string, true> = {
  node_modules: true,
  '.git': true,
  target: true,
  build: true,
  '.gradle': true,
  dist: true,
  '.astro': true,
  '.apk-lab': true,
  apk: true,
  analysis: true,
  '.tools': true,
  '.wrangler': true,
  '.venv': true,
  __pycache__: true,
};

/**
 * Staging exists so lockfile regeneration and update commands run against an
 * isolated tree. Copying the worktree walked local build artifacts too: a
 * checkout with `.apk-lab` and `apk/` staged gigabytes per update. Prefer Git's
 * tracked file list, which is bounded, deterministic, and excludes ignored
 * artifact trees. Fall back to a filtered walk for non-repository inputs such
 * as the test fixtures.
 */
function listTrackedFiles(repoRoot: string): string[] | null {
  if (!fs.existsSync(path.join(repoRoot, '.git'))) return null;
  const result = spawnSync(
    'git',
    ['-C', repoRoot, 'ls-files', '-z', '--cached'],
    { encoding: 'buffer', maxBuffer: 256 * 1024 * 1024 },
  );
  if (result.error || result.status !== 0 || !result.stdout) return null;
  return result.stdout.toString('utf8').split('\0').filter(Boolean);
}

function walkStagingFiles(root: string, relative = ''): string[] {
  const directory = path.join(root, relative);
  const files: string[] = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const child = relative ? `${relative}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      if (Object.hasOwn(STAGING_SKIP_DIRECTORIES, entry.name)) continue;
      files.push(...walkStagingFiles(root, child));
    } else {
      files.push(child);
    }
  }
  return files;
}

function stagingRelativePath(value: string): string {
  const normalized = value.replaceAll('\\', '/');
  const parts = normalized.split('/');
  if (
    !normalized ||
    normalized.startsWith('/') ||
    /^[a-zA-Z]:/.test(normalized) ||
    parts.includes('..')
  ) {
    throw new Error(`Refusing to stage path outside the repository: ${value}`);
  }
  return normalized;
}

export function seedStagingDirectory(
  repoRoot: string,
  stagingDir: string,
  extraPaths: Iterable<string> = [],
): string[] {
  const candidates = listTrackedFiles(repoRoot) ?? walkStagingFiles(repoRoot);
  const relativePaths = new Set<string>();
  for (const candidate of candidates) {
    relativePaths.add(stagingRelativePath(candidate));
  }
  for (const extra of extraPaths) {
    relativePaths.add(stagingRelativePath(extra));
  }

  const rootPrefix = stagingDir.endsWith(path.sep)
    ? stagingDir
    : `${stagingDir}${path.sep}`;
  const copied: string[] = [];
  for (const relative of relativePaths) {
    const source = path.resolve(repoRoot, relative);
    const destination = path.resolve(stagingDir, relative);
    if (destination !== stagingDir && !destination.startsWith(rootPrefix)) {
      throw new Error(`Refusing to stage path outside root: ${relative}`);
    }
    let stats: fs.Stats;
    try {
      stats = fs.lstatSync(source);
    } catch {
      // Tracked but deleted from the worktree; there is nothing to copy.
      continue;
    }
    if (stats.isDirectory()) continue; // Submodule gitlink.
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    if (stats.isSymbolicLink()) {
      fs.symlinkSync(fs.readlinkSync(source), destination);
    } else {
      fs.copyFileSync(source, destination);
      fs.chmodSync(destination, stats.mode & 0o777);
    }
    copied.push(relative);
  }
  return copied;
}

export async function applyApprovedProposal(
  repoRoot: string,
  proposal: Proposal,
  policy: DependencyPolicy,
  options: ApplyOptions = {},
): Promise<ApplyResult> {
  const runner = options.runner || defaultRunner;

  // 1. Verify proposal digest
  const expectedDigest = computeProposalDigest(
    proposal.inputs,
    proposal.updates,
  );
  if (proposal.digest !== expectedDigest) {
    throw new Error(
      `Proposal digest verification failed: expected ${expectedDigest}, got ${proposal.digest}`,
    );
  }

  // 2. Verify all inputs exist and match current hashes
  for (const input of proposal.inputs) {
    const absPath = path.resolve(repoRoot, input.path);
    if (!fs.existsSync(absPath)) {
      throw new Error(`Proposal input file not found: ${input.path}`);
    }
    const currentSha256 = crypto
      .createHash('sha256')
      .update(fs.readFileSync(absPath))
      .digest('hex');
    if (currentSha256 !== input.sha256) {
      throw new Error(
        `Proposal input file has changed since proposal generation: ${input.path} (expected ${input.sha256.slice(0, 8)}, got ${currentSha256.slice(0, 8)})`,
      );
    }
  }

  // 3. Verify no held items are included in updates
  const heldIds = new Set(policy.holds.map((h) => h.id));
  for (const update of proposal.updates) {
    if (heldIds.has(update.id)) {
      throw new Error(`Cannot apply update to held dependency: ${update.id}`);
    }
  }

  if (proposal.updates.length === 0) {
    return {
      success: true,
      appliedUpdates: [],
      changedFiles: [],
      message: 'No updates to apply.',
    };
  }

  // 4. Create staging directory
  const stagingDir = fs.mkdtempSync(
    path.join(os.tmpdir(), 'aidans-patches-deps-'),
  );
  const touchedFiles = new Set<string>();
  const appliedIds = new Set<string>();

  try {
    // Seed staging with tracked sources plus the verified proposal inputs.
    // Local artifact trees must never be copied.
    seedStagingDirectory(
      repoRoot,
      stagingDir,
      proposal.inputs.map((input) => input.path),
    );

    // 5. Apply scalar updates to staged files
    for (const update of proposal.updates) {
      const { id, from, to } = update;

      // --- NPM Packages ---
      if (id.startsWith('npm:')) {
        const hashIdx = id.indexOf('#');
        if (hashIdx === -1) {
          throw new Error(`Invalid NPM package ID structure: ${id}`);
        }
        const manifestRel = id.slice(4, hashIdx);
        const rest = id.slice(hashIdx + 1);
        const slashIdx = rest.indexOf('/');
        if (slashIdx === -1) {
          throw new Error(`Invalid NPM package ID structure: ${id}`);
        }
        const section = rest.slice(0, slashIdx);
        const pkgName = rest.slice(slashIdx + 1);
        const stagedManifest = path.join(stagingDir, manifestRel);

        if (fs.existsSync(stagedManifest)) {
          const jsonText = fs.readFileSync(stagedManifest, 'utf8');
          const json = JSON.parse(jsonText);
          if (json[section] && json[section][pkgName]) {
            const currentVal = json[section][pkgName] as string;
            if (currentVal.startsWith('^')) {
              json[section][pkgName] = `^${to}`;
            } else if (currentVal.startsWith('~')) {
              json[section][pkgName] = `~${to}`;
            } else if (currentVal.startsWith('git+')) {
              throw new Error(
                `Cannot update git branch dependency '${pkgName}' with deps:update; refresh the lockfile with the package manager instead`,
              );
            } else {
              json[section][pkgName] = to;
            }
            fs.writeFileSync(
              stagedManifest,
              JSON.stringify(json, null, 2) + '\n',
            );
            touchedFiles.add(manifestRel);
            appliedIds.add(id);
          }
        }
      }

      // --- Cargo Packages ---
      else if (id.startsWith('crates:')) {
        const hashIdx = id.indexOf('#');
        const manifestRel = id.slice(7, hashIdx);
        const sectionRef = id.slice(hashIdx + 1);
        const slashIdx = sectionRef.lastIndexOf('/');
        if (slashIdx === -1) {
          throw new Error(`Invalid Cargo dependency ID structure: ${id}`);
        }
        const section = sectionRef.slice(0, slashIdx);
        const pkgName = sectionRef.slice(slashIdx + 1);
        const stagedManifest = path.join(stagingDir, manifestRel);

        if (fs.existsSync(stagedManifest)) {
          const content = fs.readFileSync(stagedManifest, 'utf8');
          const updated = replaceCargoDependencyVersion(
            content,
            section,
            pkgName,
            to,
          );
          fs.writeFileSync(stagedManifest, updated);
          touchedFiles.add(manifestRel);
          appliedIds.add(id);
        }
      }

      // --- Gradle Settings & Catalog ---
      else if (id.startsWith('gradle:settings.gradle.kts')) {
        const stagedSettings = path.join(stagingDir, 'settings.gradle.kts');
        if (fs.existsSync(stagedSettings)) {
          let content = fs.readFileSync(stagedSettings, 'utf8');
          content = content.replace(
            /(id\(["']app\.morphe\.patches["']\)\s+version\s+["'])([^"']+)(["'])/,
            `$1${to}$3`,
          );
          fs.writeFileSync(stagedSettings, content);
          touchedFiles.add('settings.gradle.kts');
          appliedIds.add(id);
        }
      } else if (id.startsWith('gradle:') && id.includes('#dependencies/')) {
        const hashIdx = id.indexOf('#');
        const buildRel = id.slice(7, hashIdx);
        const coordinates = id.slice(hashIdx + '#dependencies/'.length);
        const stagedBuild = path.join(stagingDir, buildRel);
        if (!fs.existsSync(stagedBuild)) {
          throw new Error(
            `Cannot update Gradle dependency: ${buildRel} is missing`,
          );
        }
        const content = fs.readFileSync(stagedBuild, 'utf8');
        const escaped = escapeRegex(`${coordinates}:${from}`);
        const regex = new RegExp(`(["'])${escaped}(?=@|["'])`, 'g');
        const updated = content.replace(regex, `$1${coordinates}:${to}`);
        if (updated === content) {
          throw new Error(
            `Cannot update Gradle dependency: ${coordinates}:${from} was not found in ${buildRel}`,
          );
        }
        fs.writeFileSync(stagedBuild, updated);
        touchedFiles.add(buildRel);
        appliedIds.add(id);
      } else if (id.startsWith('gradle:gradle/libs.versions.toml')) {
        const stagedCatalog = path.join(
          stagingDir,
          'gradle/libs.versions.toml',
        );
        if (fs.existsSync(stagedCatalog)) {
          const hashIdx = id.indexOf('#');
          const fragment = hashIdx >= 0 ? id.slice(hashIdx + 1) : '';
          const key = fragment.split('/')[1] || '';
          let content = fs.readFileSync(stagedCatalog, 'utf8');
          const vRegex = new RegExp(`(${key}\\s*=\\s*)"([^"]+)"`);
          content = content.replace(vRegex, `$1"${to}"`);
          fs.writeFileSync(stagedCatalog, content);
          touchedFiles.add('gradle/libs.versions.toml');
          appliedIds.add(id);
        }
      }

      // --- Tool: Gradle Wrapper ---
      else if (id === 'tool:gradle') {
        const wrapperRel = 'gradle/wrapper/gradle-wrapper.properties';
        const stagedProps = path.join(stagingDir, wrapperRel);
        if (!fs.existsSync(stagedProps)) {
          throw new Error(
            `Cannot update Gradle wrapper: ${wrapperRel} is missing`,
          );
        }
        let content = fs.readFileSync(stagedProps, 'utf8');
        const updatedUrl = content.replace(
          /gradle-([0-9.]+)-(bin|all)\.zip/,
          `gradle-${to}-$2.zip`,
        );
        if (updatedUrl === content) {
          throw new Error(
            `Cannot update Gradle wrapper: no distributionUrl matching ${from} in ${wrapperRel}`,
          );
        }
        const sha256 = await resolveAssetHash(
          update.assetIdentity,
          undefined,
          'Gradle wrapper',
        );
        const withSha = updatedUrl.replace(
          /distributionSha256Sum=[a-f0-9]{64}/,
          `distributionSha256Sum=${sha256}`,
        );
        if (withSha === updatedUrl) {
          throw new Error(
            `Cannot update Gradle wrapper: ${wrapperRel} has no distributionSha256Sum`,
          );
        }
        content = withSha;
        fs.writeFileSync(stagedProps, content);
        touchedFiles.add(wrapperRel);
        appliedIds.add(id);
      }

      // --- Tool: ktlint ---
      else if (id === 'tool:ktlint') {
        const auditRel = 'scripts/audit.ts';
        const stagedAudit = path.join(stagingDir, auditRel);
        if (!fs.existsSync(stagedAudit)) {
          throw new Error(`Cannot update ktlint: ${auditRel} is missing`);
        }
        const sha256 = await resolveAssetHash(
          update.assetIdentity,
          update.assetUrl ??
            `https://github.com/pinterest/ktlint/releases/download/${to}/ktlint`,
          'ktlint',
        );
        let content = fs.readFileSync(stagedAudit, 'utf8');
        const updatedVersion = content.replace(
          /KTLINT_VERSION\s*=\s*["'][^"']+["']/,
          `KTLINT_VERSION = '${to}'`,
        );
        if (updatedVersion === content) {
          throw new Error(
            `Cannot update ktlint: no KTLINT_VERSION in ${auditRel}`,
          );
        }
        const withSha = updatedVersion.replace(
          /KTLINT_SHA256\s*=\s*["'][a-f0-9]{64}["']/,
          `KTLINT_SHA256 = '${sha256}'`,
        );
        if (withSha === updatedVersion) {
          throw new Error(
            `Cannot update ktlint: no KTLINT_SHA256 in ${auditRel}`,
          );
        }
        content = withSha;
        fs.writeFileSync(stagedAudit, content);
        touchedFiles.add(auditRel);
        appliedIds.add(id);
      }

      // --- Tool: APK Lab tools ---
      else if (id.startsWith('tool:apk-lab/')) {
        const toolName = id.slice(13);
        const toolLockRel = 'tools/apk-lab/tools.lock.json';
        const stagedToolLock = path.join(stagingDir, toolLockRel);
        if (!fs.existsSync(stagedToolLock)) {
          throw new Error(
            `Cannot update ${toolName}: ${toolLockRel} is missing`,
          );
        }
        const json = JSON.parse(fs.readFileSync(stagedToolLock, 'utf8'));
        const tool = json.tools?.[toolName];
        if (!tool) {
          throw new Error(
            `Cannot update ${toolName}: entry is missing from ${toolLockRel}`,
          );
        }
        const assetUrl = update.assetUrl
          ? update.assetUrl
          : replaceVersionText(String(tool.url ?? ''), from, to);
        if (!assetUrl) {
          throw new Error(
            `Cannot update ${toolName}: no download URL is available`,
          );
        }
        const sha256 = await resolveAssetHash(
          update.assetIdentity,
          assetUrl,
          toolName,
        );
        tool.version = to;
        tool.sha256 = sha256;
        tool.url = assetUrl;
        if (typeof tool.executablePath === 'string') {
          tool.executablePath =
            update.executablePath ||
            replaceVersionText(tool.executablePath, from, to);
        }
        if (tool.versionProbe && typeof tool.versionProbe === 'object') {
          tool.versionProbe.expectedPattern =
            update.versionProbePattern ||
            replaceVersionText(
              String(tool.versionProbe.expectedPattern ?? ''),
              from,
              to,
            );
        }
        fs.writeFileSync(stagedToolLock, JSON.stringify(json, null, 2) + '\n');
        touchedFiles.add(toolLockRel);
        appliedIds.add(id);
      }

      // --- GitHub Actions ---
      else if (id.startsWith('action:')) {
        const actionName = id.slice(7);
        const workflowsDir = path.join(stagingDir, '.github/workflows');
        let matched = false;
        if (fs.existsSync(workflowsDir)) {
          for (const file of fs.readdirSync(workflowsDir)) {
            if (!file.endsWith('.yml') && !file.endsWith('.yaml')) continue;
            const wPath = path.join(workflowsDir, file);
            let content = fs.readFileSync(wPath, 'utf8');
            const regex = new RegExp(`(uses:\\s*${actionName}@)[^\\s#]+`, 'g');
            if (regex.test(content)) {
              content = content.replace(regex, `$1v${to.replace(/^v/, '')}`);
              fs.writeFileSync(wPath, content);
              touchedFiles.add(`.github/workflows/${file}`);
              matched = true;
            }
          }
        }
        if (!matched) {
          throw new Error(
            `Cannot update action ${actionName}: no workflow references it`,
          );
        }
        appliedIds.add(id);
      }

      // --- Rust Toolchain Native ---
      else if (id === 'tool:rust-native') {
        const toolchainRel =
          'extensions/nhl-adblock-native/rust-toolchain.toml';
        const stagedTc = path.join(stagingDir, toolchainRel);
        if (!fs.existsSync(stagedTc)) {
          throw new Error(
            `Cannot update Rust toolchain: ${toolchainRel} is missing`,
          );
        }
        let content = fs.readFileSync(stagedTc, 'utf8');
        const updated = content.replace(
          /(channel\s*=\s*")([^"]+)(")/,
          `$1${to}$3`,
        );
        if (updated === content) {
          throw new Error(
            `Cannot update Rust toolchain: no channel in ${toolchainRel}`,
          );
        }
        fs.writeFileSync(stagedTc, updated);
        touchedFiles.add(toolchainRel);
        const workflowsDir = path.join(stagingDir, '.github/workflows');
        if (fs.existsSync(workflowsDir)) {
          for (const file of fs.readdirSync(workflowsDir)) {
            if (!file.endsWith('.yml')) continue;
            const wPath = path.join(workflowsDir, file);
            const workflow = fs.readFileSync(wPath, 'utf8');
            if (workflow.includes('dtolnay/rust-toolchain@')) {
              const next = workflow.replace(
                /(dtolnay\/rust-toolchain@)[0-9.]+/,
                `$1${to}`,
              );
              if (next !== workflow) {
                fs.writeFileSync(wPath, next);
                touchedFiles.add(`.github/workflows/${file}`);
              }
            }
          }
        }
        appliedIds.add(id);
      }

      // --- Tool: Java ---
      else if (id === 'tool:java') {
        const workflowsDir = path.join(stagingDir, '.github/workflows');
        let matched = false;
        if (fs.existsSync(workflowsDir)) {
          for (const file of fs.readdirSync(workflowsDir)) {
            if (!file.endsWith('.yml')) continue;
            const wPath = path.join(workflowsDir, file);
            const workflow = fs.readFileSync(wPath, 'utf8');
            const next = workflow.replace(
              /(java-version:\s*['"]?)[0-9]+(['"]?)/g,
              `$1${to}$2`,
            );
            if (next !== workflow) {
              fs.writeFileSync(wPath, next);
              touchedFiles.add(`.github/workflows/${file}`);
              matched = true;
            }
          }
        }
        if (!matched) {
          throw new Error(
            'Cannot update Java: no java-version input in workflows',
          );
        }
        appliedIds.add(id);
      }

      // --- Android NDK ---
      else if (id === 'tool:ndk') {
        const patchBuildRel = 'patches/build.gradle.kts';
        const stagedPatchBuild = path.join(stagingDir, patchBuildRel);
        if (!fs.existsSync(stagedPatchBuild)) {
          throw new Error(`Cannot update NDK: ${patchBuildRel} is missing`);
        }
        let content = fs.readFileSync(stagedPatchBuild, 'utf8');
        const updated = content.replace(
          /REQUIRED_NDK_VERSION\s*=\s*"[^"]+"/,
          `REQUIRED_NDK_VERSION = "${to}"`,
        );
        if (updated === content) {
          throw new Error(
            `Cannot update NDK: no REQUIRED_NDK_VERSION in ${patchBuildRel}`,
          );
        }
        fs.writeFileSync(stagedPatchBuild, updated);
        touchedFiles.add(patchBuildRel);
        const label = update.label || ndkLabelFromRevision(to);
        if (!label) {
          throw new Error(
            `Cannot update NDK: no release label for revision ${to}`,
          );
        }
        const workflowsDir = path.join(stagingDir, '.github/workflows');
        if (fs.existsSync(workflowsDir)) {
          for (const file of fs.readdirSync(workflowsDir)) {
            if (!file.endsWith('.yml')) continue;
            const wPath = path.join(workflowsDir, file);
            const workflow = fs.readFileSync(wPath, 'utf8');
            if (workflow.includes('ndk-version:')) {
              const next = workflow.replace(
                /(ndk-version:\s*)[^\s\n]+/,
                `$1${label}`,
              );
              if (next !== workflow) {
                fs.writeFileSync(wPath, next);
                touchedFiles.add(`.github/workflows/${file}`);
              }
            }
          }
        }
        appliedIds.add(id);
      }

      // --- Android build-tools ---
      else if (id === 'tool:android-build-tools') {
        const locatorRel = 'tools/apk-lab/src/tools.rs';
        const stagedLocator = path.join(stagingDir, locatorRel);
        if (!fs.existsSync(stagedLocator)) {
          throw new Error(
            `Cannot update Android build-tools: ${locatorRel} is missing`,
          );
        }
        const content = fs.readFileSync(stagedLocator, 'utf8');
        const updated = content.replace(
          /(build_tools\.join\(")([0-9.]+)("\)\.join\(bin_name\))/,
          `$1${to}$3`,
        );
        if (updated === content) {
          throw new Error(
            `Cannot update Android build-tools: no pinned version in ${locatorRel}`,
          );
        }
        fs.writeFileSync(stagedLocator, updated);
        touchedFiles.add(locatorRel);
        appliedIds.add(id);
      }

      // --- Embedded Leaflet ---
      else if (id === 'embedded:leaflet') {
        const osmRel =
          'extensions/extension/src/main/java/app/aidan/extension/aftership/OsmMapView.java';
        const stagedOsm = path.join(stagingDir, osmRel);
        if (!fs.existsSync(stagedOsm)) {
          throw new Error(`Cannot update Leaflet: ${osmRel} is missing`);
        }
        const content = fs.readFileSync(stagedOsm, 'utf8');
        const updated = content.replace(
          /unpkg\.com\/leaflet@[0-9.]+\//g,
          `unpkg.com/leaflet@${to}/`,
        );
        if (updated === content) {
          throw new Error(
            `Cannot update Leaflet: no unpkg asset URL in ${osmRel}`,
          );
        }
        fs.writeFileSync(stagedOsm, updated);
        touchedFiles.add(osmRel);
        appliedIds.add(id);
      }

      // --- CMake FetchContent ---
      else if (
        id.startsWith(
          'cmake:extensions/adobescan-pdf-native/CMakeLists.txt#fetch/',
        )
      ) {
        const pkgName = id.split('#fetch/')[1];
        const manifestRel = 'extensions/adobescan-pdf-native/CMakeLists.txt';
        const stagedCMake = path.join(stagingDir, manifestRel);
        if (fs.existsSync(stagedCMake)) {
          let content = fs.readFileSync(stagedCMake, 'utf8');
          const fetchBlockRegex = new RegExp(
            `FetchContent_Declare\\s*\\(\\s*${pkgName}[\\s\\S]*?\\)`,
            'g',
          );

          let newUrl = '';
          if (pkgName === 'qpdf') {
            newUrl = `https://github.com/qpdf/qpdf/releases/download/v${to}/qpdf-${to}.tar.gz`;
          } else if (pkgName === 'libjpeg-turbo') {
            newUrl = `https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${to}/libjpeg-turbo-${to}.tar.gz`;
          } else if (pkgName === 'leptonica') {
            newUrl = `https://github.com/DanBloomberg/leptonica/releases/download/${to}/leptonica-${to}.tar.gz`;
          } else if (pkgName === 'tesseract') {
            newUrl = `https://github.com/tesseract-ocr/tesseract/archive/refs/tags/${to}.tar.gz`;
          }

          let matchedOldUrl = '';
          const blockMatch = fetchBlockRegex.exec(content);
          fetchBlockRegex.lastIndex = 0;
          if (!blockMatch) {
            throw new Error(
              `Cannot update ${pkgName}: no FetchContent_Declare block in ${manifestRel}`,
            );
          }
          const urlMatch = blockMatch[0].match(/URL\s+([^\s)]+)/);
          if (!urlMatch) {
            throw new Error(
              `Cannot update ${pkgName}: FetchContent block has no URL in ${manifestRel}`,
            );
          }
          matchedOldUrl = urlMatch[1];
          if (!newUrl) {
            newUrl = replaceVersionText(matchedOldUrl, from, to);
          }
          if (!newUrl || newUrl === matchedOldUrl) {
            throw new Error(
              `Cannot update ${pkgName}: URL ${matchedOldUrl} does not contain version ${from}`,
            );
          }

          const newSha256 = await resolveAssetHash(
            update.assetIdentity,
            newUrl,
            pkgName,
          );

          const nextContent = content.replace(fetchBlockRegex, (block) => {
            let updated = block.replace(matchedOldUrl, newUrl);
            updated = updated.replace(
              /URL_HASH\s+SHA256=[a-fA-F0-9]{64}/,
              `URL_HASH SHA256=${newSha256}`,
            );
            return updated;
          });
          if (nextContent === content) {
            throw new Error(
              `Cannot update ${pkgName}: no changes were applied in ${manifestRel}`,
            );
          }
          fs.writeFileSync(stagedCMake, nextContent);
          touchedFiles.add(manifestRel);
          appliedIds.add(id);
        }
      }
    }

    const unhandled = proposal.updates.filter(
      (update) => !appliedIds.has(update.id),
    );
    if (unhandled.length > 0) {
      throw new Error(
        `No apply strategy matched: ${unhandled.map((update) => update.id).join(', ')}`,
      );
    }

    // 6. Regenerate staged lockfiles
    if (touchedFiles.has('package.json')) {
      const bunRes = await runner(
        'bun',
        ['install', '--lockfile-only', '--ignore-scripts'],
        stagingDir,
      );
      if (bunRes.code !== 0)
        throw new Error(
          `bun install --lockfile-only failed in staging: ${bunRes.stderr}`,
        );
      touchedFiles.add('bun.lock');

      const npmRes = await runner(
        'npm',
        [
          'install',
          '--package-lock-only',
          '--ignore-scripts',
          '--no-audit',
          '--no-fund',
        ],
        stagingDir,
      );
      if (npmRes.code !== 0)
        throw new Error(
          `npm install --package-lock-only failed in staging: ${npmRes.stderr}`,
        );
      touchedFiles.add('package-lock.json');
    }

    if (touchedFiles.has('worker/package.json')) {
      const bunRes = await runner(
        'bun',
        ['install', '--lockfile-only', '--ignore-scripts'],
        path.join(stagingDir, 'worker'),
      );
      if (bunRes.code !== 0)
        throw new Error(`bun install in worker failed: ${bunRes.stderr}`);
      touchedFiles.add('worker/bun.lock');
    }

    if (touchedFiles.has('site/package.json')) {
      const bunRes = await runner(
        'bun',
        ['install', '--lockfile-only', '--ignore-scripts'],
        path.join(stagingDir, 'site'),
      );
      if (bunRes.code !== 0)
        throw new Error(`bun install in site failed: ${bunRes.stderr}`);
      touchedFiles.add('site/bun.lock');
    }

    for (const project of RUST_PROJECTS) {
      if (!touchedFiles.has(project.manifest)) continue;
      const projectDir = path.dirname(path.join(stagingDir, project.manifest));
      const cargoUpdates = proposal.updates.filter((update) =>
        update.id.startsWith(`crates:${project.manifest}#`),
      );
      for (const update of cargoUpdates) {
        const cragment = update.id.split('#')[1] ?? '';
        const packageName = cragment.split('/')[1] ?? '';
        const res = await runner(
          'cargo',
          [
            'update',
            '--manifest-path',
            path.basename(project.manifest),
            '--package',
            packageName,
            '--precise',
            update.to,
          ],
          projectDir,
        );
        if (res.code !== 0) {
          throw new Error(
            `cargo update failed for ${project.label} ${packageName}: ${res.stderr}`,
          );
        }
      }
      const lockPath = path.join(stagingDir, project.lockfile);
      if (!fs.existsSync(lockPath)) {
        throw new Error(
          `Lockfile regeneration did not produce ${project.lockfile}`,
        );
      }
      touchedFiles.add(project.lockfile);
    }

    // 7. Atomic publication to repository
    const changedFilesList = Array.from(touchedFiles).sort();
    const backupMap = new Map<string, Buffer>();

    // Take backup snapshots of current repo files
    for (const relFile of changedFilesList) {
      const realPath = path.resolve(repoRoot, relFile);
      if (fs.existsSync(realPath)) {
        backupMap.set(relFile, fs.readFileSync(realPath));
      }
    }

    try {
      for (const relFile of changedFilesList) {
        const stagedFile = path.resolve(stagingDir, relFile);
        const repoFile = path.resolve(repoRoot, relFile);
        if (fs.existsSync(stagedFile)) {
          fs.mkdirSync(path.dirname(repoFile), { recursive: true });
          fs.copyFileSync(stagedFile, repoFile);
        }
      }
    } catch (writeErr) {
      // Rollback
      for (const [relFile, buf] of backupMap.entries()) {
        fs.writeFileSync(path.resolve(repoRoot, relFile), buf);
      }
      throw new Error(
        `Write failed during publication; rolled back all changes: ${(writeErr as Error).message}`,
      );
    }

    return {
      success: true,
      appliedUpdates: proposal.updates,
      changedFiles: changedFilesList,
      message: `Successfully applied ${proposal.updates.length} dependency updates across ${changedFilesList.length} files.`,
    };
  } finally {
    // Clean staging
    try {
      fs.rmSync(stagingDir, { recursive: true, force: true });
    } catch {
      // Ignore cleanup error
    }
  }
}
