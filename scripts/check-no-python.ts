#!/usr/bin/env bun
/**
 * Deterministic Python-free gate.
 *
 * Scans tracked first-party executable, configuration and source files for
 * retired Python/uv/pip/pytest/mypy/Ruff/pre-commit requirements. Historical
 * evidence is excluded explicitly; anything else must be migrated to the
 * native/Bun toolchain instead of reintroducing an interpreter requirement.
 */
import * as fs from 'node:fs';
import * as path from 'node:path';

export interface Violation {
  file: string;
  line?: number;
  message: string;
}

export interface HistoricalExclusion {
  prefix: string;
  reason: string;
}

/**
 * Historical artifacts that may still contain Python references without
 * creating a runtime or build requirement. This list is deliberately narrow:
 * active docs, scripts, workflows and manifests are still scanned.
 */
export const HISTORICAL_EXCLUSIONS: readonly HistoricalExclusion[] = [
  {
    prefix: 'plans/',
    reason: 'local advisor plans, ignored by git',
  },
  {
    prefix: 'docs/benchmarks/',
    reason: 'immutable measured migration evidence',
  },
  {
    prefix: 'tools/asset-prep/tests/fixtures/',
    reason: 'frozen Pillow oracle capture inputs and contracts from plan 006',
  },
  {
    prefix: 'tools/asset-prep/tests/baseline_contracts.rs',
    reason:
      'asserts the frozen Pillow baseline contract; it never runs the interpreter',
  },
  {
    prefix: 'scripts/test-fixtures/',
    reason: 'frozen generator parity fixtures mirrored from retired output',
  },
  {
    prefix: 'scripts/install-hooks.ts',
    reason:
      'detects and migrates pre-commit wrapper templates; does not require the framework',
  },
  {
    prefix: 'scripts/hooks.test.ts',
    reason:
      'exercises legacy pre-commit wrapper migration without requiring the framework',
  },
  {
    prefix: 'scripts/check-no-python.ts',
    reason: 'the gate definition necessarily contains its own rule patterns',
  },
  {
    prefix: 'scripts/check-no-python.test.ts',
    reason: 'gate fixtures deliberately contain violating sample content',
  },
  {
    prefix: 'docs/apk-lab-rust-migration.md',
    reason: 'historical migration report; records retired Python evidence',
  },
  {
    prefix: 'CHANGELOG.md',
    reason: 'immutable release history',
  },
];

const SCANNABLE_EXTENSIONS: Record<string, true> = {
  '.ts': true,
  '.tsx': true,
  '.mts': true,
  '.cts': true,
  '.js': true,
  '.jsx': true,
  '.mjs': true,
  '.cjs': true,
  '.json': true,
  '.jsonc': true,
  '.yml': true,
  '.yaml': true,
  '.toml': true,
  '.gradle': true,
  '.kts': true,
  '.sh': true,
  '.bash': true,
  '.zsh': true,
  '.rs': true,
  '.kt': true,
  '.java': true,
  '.cmake': true,
  '.md': true,
  '.txt': true,
  '.xml': true,
  '.properties': true,
  '.cfg': true,
  '.ini': true,
};

const SCANNABLE_BASENAMES: Record<string, true> = {
  Dockerfile: true,
  Makefile: true,
  gradlew: true,
  'gradlew.bat': true,
  'CMakeLists.txt': true,
};

const RETIRED_PATHS: readonly { pattern: RegExp; message: string }[] = [
  {
    pattern: /(^|\/)\.python-version$/,
    message: 'Python version pin remains',
  },
  { pattern: /(^|\/)pytest\.ini$/, message: 'pytest configuration remains' },
  {
    pattern: /(^|\/)pyproject\.toml$/,
    message: 'Python package manifest remains',
  },
  { pattern: /(^|\/)uv\.lock$/, message: 'uv lockfile remains' },
  {
    pattern: /(^|\/)requirements(?:[-.][\w-]+)?\.txt$/,
    message: 'pip requirements file remains',
  },
  { pattern: /(^|\/)Pipfile(?:\.lock)?$/, message: 'Pipenv manifest remains' },
  { pattern: /(^|\/)poetry\.lock$/, message: 'Poetry lockfile remains' },
  { pattern: /(^|\/)setup\.py$/, message: 'Python setup script remains' },
  { pattern: /(^|\/)setup\.cfg$/, message: 'Python packaging config remains' },
  { pattern: /(^|\/)tox\.ini$/, message: 'tox configuration remains' },
  { pattern: /(^|\/)noxfile\.py$/, message: 'nox configuration remains' },
  {
    pattern: /(^|\/)\.pre-commit-config\.ya?ml$/,
    message: 'pre-commit framework configuration remains',
  },
  {
    pattern: /\.pyi?$/,
    message: 'Python source or stub file remains',
  },
  {
    pattern: /(^|\/)tools\/apk-lab\/apk_lab\//,
    message: 'retired Python APK Lab package path remains',
  },
];

const CONTENT_RULES: readonly { pattern: RegExp; message: string }[] = [
  {
    pattern: /^#!.*\bpython(?:3(?:\.\d+)?)?\b/m,
    message: 'Python shebang',
  },
  {
    pattern:
      /(?:^|[\s"'`=:(])python(?:3(?:\.\d+)?)?\s+(?:-m|-c|-V|--version|--help)\b/,
    message: 'Python interpreter invocation',
  },
  {
    pattern: /\bpython(?:3(?:\.\d+)?)?\s+-m\s+(?:pip|pytest|mypy|ruff)\b/,
    message: 'Python module tool invocation',
  },
  {
    pattern: /\buv\s+(?:run|sync|lock|pip|tool|venv|export|add|remove)\b/,
    message: 'uv command requirement',
  },
  {
    pattern: /\buvx\s+[\w.-]+/,
    message: 'uvx command requirement',
  },
  {
    pattern: /\bpip3?\s+(?:install|uninstall|download|wheel|freeze)\b/,
    message: 'pip command requirement',
  },
  {
    pattern: /pypi\.org|\bpypi\b/i,
    message: 'PyPI requirement',
  },
  {
    pattern: /\bpytest\b/i,
    message: 'pytest requirement',
  },
  {
    pattern: /\bmypy\b/i,
    message: 'mypy requirement',
  },
  {
    pattern: /\bruff\b/i,
    message: 'Ruff requirement',
  },
  {
    pattern: /python-version\s*:/,
    message: 'Python version setup requirement',
  },
  {
    pattern: /requires-python\s*[=:]|python_requires\s*[=:]/,
    message: 'Python package interpreter requirement',
  },
  {
    pattern: /\bpre-commit\s+(?:install|run|autoupdate|try-repo)\b/,
    message: 'pre-commit framework command',
  },
  {
    pattern: /^\s*language:\s*python\b/m,
    message: 'pre-commit Python hook',
  },
  {
    pattern: /^\s*repo:\s*https:\/\/github\.com\/pre-commit\//m,
    message: 'pre-commit framework repository',
  },
  {
    pattern: /Command::new\(\s*"python3?"\s*\)/,
    message: 'Rust code spawning a Python interpreter',
  },
  {
    pattern: /find_in_path\(\s*"python3?"\s*\)/,
    message: 'Python interpreter lookup',
  },
  {
    pattern: /\.venv\/bin\/python\b/,
    message: 'virtualenv Python interpreter path',
  },
  {
    pattern:
      /tools\/apk-lab\/(?:apk_lab\/|uv\.lock|pyproject\.toml|\.python-version)/,
    message: 'retired Python APK Lab path',
  },
  {
    pattern: /\bpytest\.ini\b/,
    message: 'retired pytest configuration path',
  },
  {
    pattern: /\bpyproject\.toml\b/,
    message: 'retired Python package manifest path',
  },
  {
    pattern: /\buv\.lock\b/,
    message: 'retired uv lockfile path',
  },
];

const MAX_SCANNED_BYTES = 1_000_000;

export function isHistoricallyExcluded(relPath: string): boolean {
  return HISTORICAL_EXCLUSIONS.some(({ prefix }) =>
    prefix.endsWith('/') ? relPath.startsWith(prefix) : relPath === prefix,
  );
}

export function isScannable(relPath: string): boolean {
  const base = path.posix.basename(relPath);
  if (SCANNABLE_BASENAMES[base]) return true;
  return Boolean(SCANNABLE_EXTENSIONS[path.posix.extname(base)]);
}

function lineNumberAt(text: string, index: number): number {
  let line = 1;
  for (let i = 0; i < index; i++) {
    if (text.charCodeAt(i) === 10) line++;
  }
  return line;
}

export function scanFile(root: string, relPath: string): Violation[] {
  const violations: Violation[] = [];

  for (const { pattern, message } of RETIRED_PATHS) {
    if (pattern.test(relPath)) {
      violations.push({ file: relPath, message });
    }
  }

  if (!isScannable(relPath)) return violations;

  const absPath = path.resolve(root, relPath);
  let stat: fs.Stats;
  try {
    stat = fs.statSync(absPath);
  } catch {
    return violations;
  }
  if (!stat.isFile() || stat.size > MAX_SCANNED_BYTES) return violations;

  const buf = fs.readFileSync(absPath);
  if (buf.includes(0)) return violations;
  const text = buf.toString('utf8');

  for (const { pattern, message } of CONTENT_RULES) {
    const flags = pattern.flags.includes('g')
      ? pattern.flags
      : `${pattern.flags}g`;
    const global = new RegExp(pattern.source, flags);
    let match: RegExpExecArray | null;
    while ((match = global.exec(text)) !== null) {
      const line = lineNumberAt(text, match.index);
      violations.push({ file: relPath, line, message });
      if (match[0].length === 0) global.lastIndex++;
    }
  }

  return violations;
}

export function listTrackedFiles(root: string): string[] {
  const result = Bun.spawnSync(
    [
      'git',
      '-C',
      root,
      'ls-files',
      '-z',
      '--cached',
      '--others',
      '--exclude-standard',
    ],
    { stdout: 'pipe', stderr: 'pipe' },
  );
  if (result.exitCode !== 0) {
    throw new Error(
      `git ls-files failed in ${root}: ${result.stderr.toString().trim()}`,
    );
  }
  return result.stdout
    .toString()
    .split('\0')
    .filter((entry) => entry.length > 0)
    .sort();
}

export function scanRepository(
  root: string,
  files: readonly string[] = listTrackedFiles(root),
): Violation[] {
  const violations: Violation[] = [];
  for (const relPath of files) {
    if (isHistoricallyExcluded(relPath)) continue;
    if (!fs.existsSync(path.resolve(root, relPath))) continue;
    violations.push(...scanFile(root, relPath));
  }
  return violations.sort(
    (a, b) =>
      a.file.localeCompare(b.file) ||
      (a.line ?? 0) - (b.line ?? 0) ||
      a.message.localeCompare(b.message),
  );
}

export function formatViolations(violations: readonly Violation[]): string {
  if (violations.length === 0) {
    return 'Python-free gate: no first-party Python requirements found.';
  }
  const lines = [
    `Python-free gate: ${violations.length} first-party Python requirement(s) found.`,
  ];
  for (const violation of violations) {
    const location =
      violation.line === undefined
        ? violation.file
        : `${violation.file}:${violation.line}`;
    lines.push(`  ${location}  ${violation.message}`);
  }
  return lines.join('\n');
}

function printUsage(): never {
  console.log(`Usage: bun scripts/check-no-python.ts [--root <dir>] [--json]

Scans tracked first-party executable, configuration and source files for
retired Python/uv/pip/pytest/mypy/Ruff/pre-commit requirements.

Exit codes:
  0: no first-party Python requirements
  1: violations found or scan failed
`);
  process.exit(0);
}

export function run(argv: readonly string[]): number {
  let root = process.cwd();
  let json = false;

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--help' || arg === '-h') {
      printUsage();
    } else if (arg === '--json') {
      json = true;
    } else if (arg === '--root') {
      const value = argv[++i];
      if (!value) {
        console.error('Error: --root requires a directory');
        return 1;
      }
      root = path.resolve(value);
    } else {
      console.error(`Error: unknown argument ${arg}`);
      return 1;
    }
  }

  let violations: Violation[];
  try {
    violations = scanRepository(root);
  } catch (error) {
    console.error(`Error: ${(error as Error).message}`);
    return 1;
  }

  if (json) {
    console.log(JSON.stringify({ violations }, null, 2));
  } else {
    console.log(formatViolations(violations));
  }
  return violations.length === 0 ? 0 : 1;
}

if (import.meta.main) {
  process.exit(run(process.argv.slice(2)));
}
