import { describe, expect, test } from 'bun:test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import type { CommandRunner } from './apply';
import {
  formatSecurityReport,
  parseBunAuditJson,
  parseCargoAuditJson,
  parseNpmAuditJson,
  runSecurityAudit,
  sortFindings,
} from './security';

describe('Security audit parsers', () => {
  test('parses bun audit advisories', () => {
    const findings = parseBunAuditJson(
      JSON.stringify({
        handlebars: [
          {
            title: 'Injection',
            severity: 'critical',
            url: 'https://example.com/advisory',
          },
        ],
      }),
      'bun (root)',
    );
    expect(findings).toEqual([
      {
        source: 'bun (root)',
        package: 'handlebars',
        severity: 'critical',
        title: 'Injection',
        url: 'https://example.com/advisory',
      },
    ]);
  });

  test('parses npm audit advisories including dependency-only entries', () => {
    const findings = parseNpmAuditJson(
      JSON.stringify({
        auditReportVersion: 2,
        vulnerabilities: {
          foo: {
            name: 'foo',
            severity: 'high',
            via: [{ title: 'Prototype pollution', url: 'https://example.com' }],
          },
          bar: { name: 'bar', severity: 'moderate', via: ['foo'] },
        },
      }),
      'npm (release lock)',
    );
    expect(findings.map((f) => [f.package, f.severity, f.title])).toEqual([
      ['foo', 'high', 'Prototype pollution'],
      ['bar', 'moderate', 'Security advisory'],
    ]);
  });

  test('parses cargo audit advisories', () => {
    const findings = parseCargoAuditJson(
      JSON.stringify({
        vulnerabilities: {
          list: [
            {
              advisory: {
                title: 'Use after free',
                severity: 'critical',
                url: 'https://example.com/cargo',
              },
              package: { name: 'ring' },
            },
          ],
        },
      }),
      'cargo (APK Lab)',
    );
    expect(findings[0]).toMatchObject({
      package: 'ring',
      severity: 'critical',
      title: 'Use after free',
    });
  });

  test('sorts findings by severity then source', () => {
    const sorted = sortFindings([
      {
        source: 'b',
        package: 'low',
        severity: 'low',
        title: 'low',
      },
      {
        source: 'a',
        package: 'crit',
        severity: 'critical',
        title: 'crit',
      },
      {
        source: 'a',
        package: 'mod',
        severity: 'moderate',
        title: 'mod',
      },
    ]);
    expect(sorted.map((f) => f.severity)).toEqual([
      'critical',
      'moderate',
      'low',
    ]);
  });
});

describe('Security audit runner', () => {
  test('collects JS findings and skips missing cargo-audit', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'security-audit-'));
    try {
      fs.writeFileSync(path.join(tmpDir, 'bun.lock'), '{}');
      fs.writeFileSync(path.join(tmpDir, 'package-lock.json'), '{}');
      fs.mkdirSync(path.join(tmpDir, 'worker'));
      fs.writeFileSync(path.join(tmpDir, 'worker', 'bun.lock'), '{}');

      const runner: CommandRunner = async (cmd, args) => {
        if (cmd === 'bun') {
          return {
            code: 0,
            stdout: JSON.stringify({
              left: [{ title: 'Advisory', severity: 'high' }],
            }),
            stderr: '',
          };
        }
        if (cmd === 'npm') {
          return {
            code: 0,
            stdout: JSON.stringify({
              vulnerabilities: {
                right: { name: 'right', severity: 'critical', via: ['left'] },
              },
            }),
            stderr: '',
          };
        }
        if (cmd === 'cargo' && args.includes('--version')) {
          return { code: 1, stdout: '', stderr: 'not installed' };
        }
        return { code: 0, stdout: '{}', stderr: '' };
      };

      const result = await runSecurityAudit(tmpDir, runner);
      expect(result.findings.map((f) => f.source)).toEqual([
        'npm (release lock)',
        'bun (root)',
        'bun (worker)',
      ]);
      expect(result.skipped.some((item) => item.includes('cargo-audit'))).toBe(
        true,
      );
      expect(result.errors).toEqual([]);
      expect(formatSecurityReport(result)).toContain('Findings: 3');
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  test('runs cargo audit per lockfile when available', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'security-cargo-'));
    try {
      const lockDir = path.join(tmpDir, 'tools', 'apk-lab');
      fs.mkdirSync(lockDir, { recursive: true });
      fs.writeFileSync(path.join(lockDir, 'Cargo.lock'), 'version = 4');

      const seen: string[] = [];
      const runner: CommandRunner = async (cmd, args, cwd) => {
        if (cmd === 'cargo' && args.includes('--version')) {
          return { code: 0, stdout: 'cargo-audit 1.0.0', stderr: '' };
        }
        if (cmd === 'cargo' && args[0] === 'audit') {
          seen.push(cwd);
          return {
            code: 0,
            stdout: JSON.stringify({
              vulnerabilities: {
                list: [
                  {
                    advisory: { title: 'Cargo advisory', severity: 'high' },
                    package: { name: 'crate' },
                  },
                ],
              },
            }),
            stderr: '',
          };
        }
        return { code: 0, stdout: '{}', stderr: '' };
      };

      const result = await runSecurityAudit(tmpDir, runner);
      expect(seen).toContain(lockDir);
      expect(
        result.findings.some(
          (finding) =>
            finding.source === 'cargo (APK Lab)' && finding.package === 'crate',
        ),
      ).toBe(true);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});
