import { describe, expect, test } from 'bun:test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  HISTORICAL_EXCLUSIONS,
  type Violation,
  isHistoricallyExcluded,
  run,
  scanRepository,
} from './check-no-python';

const gateScript = path.resolve(__dirname, 'check-no-python.ts');

function writeTracked(root: string, files: Record<string, string>): void {
  for (const [relPath, content] of Object.entries(files)) {
    const absPath = path.join(root, relPath);
    fs.mkdirSync(path.dirname(absPath), { recursive: true });
    fs.writeFileSync(absPath, content);
  }
  initAndTrack(root);
}

function initAndTrack(root: string): void {
  const init = Bun.spawnSync(['git', 'init', '-q', root], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (init.exitCode !== 0) {
    throw new Error(`git init failed: ${init.stderr.toString()}`);
  }
  const add = Bun.spawnSync(['git', '-C', root, 'add', '-A'], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (add.exitCode !== 0) {
    throw new Error(`git add failed: ${add.stderr.toString()}`);
  }
}

function messages(violations: readonly Violation[]): string[] {
  return violations.map((v) => `${v.file}:${v.line ?? 0} ${v.message}`);
}

function withTempRepo(runTest: (root: string) => void): void {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'no-python-gate-'));
  try {
    runTest(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

describe('Python-free gate scanning', () => {
  test('flags Python files, manifests and commands', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'tools/apk-lab/apk_lab/cli.py': 'print("retired")\n',
        'tools/apk-lab/pyproject.toml':
          '[project]\nname = "apk-lab"\nrequires-python = ">=3.12"\n',
        'package.json':
          '{"scripts":{"play-accounts":"uv run --project tools/apk-lab apk-lab play-accounts"}}\n',
        '.github/workflows/ci.yml':
          'jobs:\n  test:\n    steps:\n      - uses: actions/setup-python@v5\n        with:\n          python-version: "3.12"\n',
        'scripts/check.ts':
          'const result = Bun.spawnSync(["python3", "-m", "pytest"]);\n',
        'docs/guide.md':
          'Run `pytest -q` before committing; see tools/apk-lab/apk_lab/cli.py.\n',
        '.github/AGENTS.md':
          'Python scripts follow Ruff formatting/lint rules.\n',
      });

      const found = scanRepository(root);
      const text = messages(found).join('\n');
      expect(text).toContain('tools/apk-lab/apk_lab/cli.py');
      expect(text).toContain('Python source or stub file remains');
      expect(text).toContain('Python package manifest remains');
      expect(text).toContain('Python package interpreter requirement');
      expect(text).toContain('uv command requirement');
      expect(text).toContain('Python version setup requirement');
      expect(text).toContain('pytest requirement');
      expect(text).toContain('Ruff requirement');
      expect(text).toContain('retired Python APK Lab path');
    });
  });

  test('flags pre-commit framework configuration and commands', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        '.pre-commit-config.yaml':
          'repos:\n  - repo: https://github.com/pre-commit/pre-commit-hooks\n    rev: v4.0.0\n    hooks:\n      - id: check-yaml\n        language: python\n',
        'scripts/setup.sh': '#!/bin/sh\npre-commit install --install-hooks\n',
      });

      const text = messages(scanRepository(root)).join('\n');
      expect(text).toContain('pre-commit framework configuration remains');
      expect(text).toContain('pre-commit framework command');
    });
  });

  test('accepts a Python-free repository', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'package.json': '{"scripts":{"test":"bun test"}}\n',
        'scripts/tool.ts': 'console.log("native tooling");\n',
        '.github/workflows/ci.yml':
          'jobs:\n  test:\n    steps:\n      - run: bun test\n',
        'docs/guide.md': 'Use Bun and Cargo for every check.\n',
        'tools/example/Cargo.toml': '[package]\nname = "example"\n',
        'tools/apk-lab/apkmirror_runner.cjs': 'module.exports = {};\n',
      });

      expect(scanRepository(root)).toEqual([]);
    });
  });

  test('honours explicit historical exclusions only', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'TO-RUST.md': 'python3.12 -m venv /tmp/to-rust-charts\n',
        'CHANGELOG.md': 'Historic uv run --project tools/apk-lab release\n',
        'docs/benchmarks/apk-lab/baseline.json':
          '{"oracle":"tools/apk-lab/.venv/bin/python"}\n',
        'plans/009-retire.md': 'Run `pytest` first.\n',
        'tools/apk-lab/migration/contracts.json':
          '{"pythonVersion":">=3.12"}\n',
      });

      for (const relPath of [
        'TO-RUST.md',
        'CHANGELOG.md',
        'docs/benchmarks/apk-lab/baseline.json',
        'plans/009-retire.md',
        'tools/apk-lab/migration/contracts.json',
      ]) {
        expect(isHistoricallyExcluded(relPath)).toBe(true);
      }
      expect(scanRepository(root)).toEqual([]);
    });
  });

  test('still scans active docs outside historical exclusions', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'docs/apk-lab.md':
          'Run `uv run --project tools/apk-lab apk-lab doctor`.\n',
      });
      const text = messages(scanRepository(root)).join('\n');
      expect(text).toContain('docs/apk-lab.md');
      expect(text).toContain('uv command requirement');
    });
  });

  test('every historical exclusion carries a reason', () => {
    for (const exclusion of HISTORICAL_EXCLUSIONS) {
      expect(exclusion.prefix.length).toBeGreaterThan(0);
      expect(exclusion.reason.length).toBeGreaterThan(0);
    }
  });
});

describe('Python-free gate CLI', () => {
  test('fails on an injected fixture and passes after it is removed', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'package.json': '{"scripts":{"test":"bun test"}}\n',
        'tools/legacy/run.py': 'print("python")\n',
      });

      const failing = Bun.spawnSync(
        [process.execPath, gateScript, '--root', root],
        { stdout: 'pipe', stderr: 'pipe' },
      );
      expect(failing.exitCode).toBe(1);
      expect(failing.stdout.toString()).toContain('tools/legacy/run.py');
      expect(failing.stdout.toString()).toContain(
        'Python source or stub file remains',
      );

      fs.rmSync(path.join(root, 'tools/legacy/run.py'));
      const add = Bun.spawnSync(['git', '-C', root, 'add', '-A'], {
        stdout: 'pipe',
        stderr: 'pipe',
      });
      expect(add.exitCode).toBe(0);

      const passing = Bun.spawnSync(
        [process.execPath, gateScript, '--root', root],
        { stdout: 'pipe', stderr: 'pipe' },
      );
      expect(passing.exitCode).toBe(0);
      expect(passing.stdout.toString()).toContain(
        'no first-party Python requirements found',
      );
    });
  });

  test('runs with no Python executable on PATH and never invokes one', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'package.json': '{"scripts":{"test":"bun test"}}\n',
      });

      const binDir = fs.mkdtempSync(path.join(os.tmpdir(), 'no-python-path-'));
      try {
        const gitPath = Bun.which('git');
        expect(gitPath).toBeString();
        fs.symlinkSync(gitPath as string, path.join(binDir, 'git'));

        const invocationLog = path.join(binDir, 'python-invoked.log');
        for (const name of ['python', 'python3', 'uv', 'pytest']) {
          const sentinel = path.join(binDir, name);
          fs.writeFileSync(
            sentinel,
            `#!/bin/sh\necho "${name}" >> "${invocationLog}"\nexit 1\n`,
            { mode: 0o755 },
          );
        }

        const result = Bun.spawnSync(
          [process.execPath, gateScript, '--root', root],
          {
            stdout: 'pipe',
            stderr: 'pipe',
            env: { ...process.env, PATH: binDir },
          },
        );
        expect(result.exitCode).toBe(0);
        expect(fs.existsSync(invocationLog)).toBe(false);
      } finally {
        fs.rmSync(binDir, { recursive: true, force: true });
      }
    });
  });

  test('run() reports violations as JSON', () => {
    withTempRepo((root) => {
      writeTracked(root, {
        'requirements-dev.txt': 'pytest==8.0.0\n',
      });
      const output: string[] = [];
      const originalLog = console.log;
      console.log = (value: unknown) => output.push(String(value));
      try {
        const code = run(['--root', root, '--json']);
        expect(code).toBe(1);
        const parsed = JSON.parse(output.join('\n')) as {
          violations: Violation[];
        };
        expect(
          parsed.violations.some((v) =>
            v.message.includes('pip requirements file remains'),
          ),
        ).toBe(true);
      } finally {
        console.log = originalLog;
      }
    });
  });
});
