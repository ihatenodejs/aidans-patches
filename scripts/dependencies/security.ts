import * as fs from 'node:fs';
import * as path from 'node:path';
import { RUST_PROJECTS } from '../rust-projects';
import { defaultRunner, type CommandRunner } from './apply';

export type SecuritySeverity =
  'critical' | 'high' | 'moderate' | 'low' | 'info' | 'unknown';

export interface SecurityFinding {
  source: string;
  package: string;
  severity: SecuritySeverity;
  title: string;
  url?: string;
}

export interface SecurityAuditResult {
  findings: SecurityFinding[];
  skipped: string[];
  errors: string[];
}

const SEVERITY_ORDER: Record<SecuritySeverity, number> = {
  critical: 0,
  high: 1,
  moderate: 2,
  low: 3,
  info: 4,
  unknown: 5,
};

function normalizeSeverity(value: unknown): SecuritySeverity {
  const severity = String(value ?? '').toLowerCase();
  if (severity === 'critical') return 'critical';
  if (severity === 'high') return 'high';
  if (severity === 'moderate' || severity === 'medium') return 'moderate';
  if (severity === 'low') return 'low';
  if (severity === 'info' || severity === 'informational') return 'info';
  return 'unknown';
}

/** Parses `bun audit --json`: a map of package name to advisories. */
export function parseBunAuditJson(
  text: string,
  source: string,
): SecurityFinding[] {
  const parsed = JSON.parse(text) as unknown;
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return [];
  const advisoriesByPackage = parsed as Record<
    string,
    Array<{ title?: string; severity?: string; url?: string }>
  >;
  const findings: SecurityFinding[] = [];
  for (const [pkg, advisories] of Object.entries(advisoriesByPackage)) {
    if (!Array.isArray(advisories)) continue;
    for (const advisory of advisories) {
      findings.push({
        source,
        package: pkg,
        severity: normalizeSeverity(advisory.severity),
        title: advisory.title || 'Security advisory',
        ...(advisory.url ? { url: advisory.url } : {}),
      });
    }
  }
  return findings;
}

/** Parses `npm audit --json` (auditReportVersion 2). */
export function parseNpmAuditJson(
  text: string,
  source: string,
): SecurityFinding[] {
  const parsed = JSON.parse(text) as unknown;
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return [];
  const report = parsed as {
    vulnerabilities?: Record<
      string,
      {
        name?: string;
        severity?: string;
        via?: Array<
          string | { title?: string; url?: string; severity?: string }
        >;
      }
    >;
  };
  const findings: SecurityFinding[] = [];
  for (const [pkg, entry] of Object.entries(report.vulnerabilities ?? {})) {
    const advisories = (entry.via ?? []).filter(
      (via): via is { title?: string; url?: string; severity?: string } =>
        typeof via === 'object' && via !== null,
    );
    if (advisories.length === 0) {
      findings.push({
        source,
        package: entry.name || pkg,
        severity: normalizeSeverity(entry.severity),
        title: 'Security advisory',
      });
      continue;
    }
    for (const advisory of advisories) {
      findings.push({
        source,
        package: entry.name || pkg,
        severity: normalizeSeverity(advisory.severity ?? entry.severity),
        title: advisory.title || 'Security advisory',
        ...(advisory.url ? { url: advisory.url } : {}),
      });
    }
  }
  return findings;
}

/** Parses `cargo audit --json`. */
export function parseCargoAuditJson(
  text: string,
  source: string,
): SecurityFinding[] {
  const parsed = JSON.parse(text) as unknown;
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return [];
  const report = parsed as {
    vulnerabilities?: {
      list?: Array<{
        advisory?: { title?: string; severity?: string; url?: string };
        package?: { name?: string };
      }>;
    };
  };
  const findings: SecurityFinding[] = [];
  for (const entry of report.vulnerabilities?.list ?? []) {
    findings.push({
      source,
      package: entry.package?.name || 'unknown',
      severity: normalizeSeverity(entry.advisory?.severity),
      title: entry.advisory?.title || 'Security advisory',
      ...(entry.advisory?.url ? { url: entry.advisory.url } : {}),
    });
  }
  return findings;
}

function dedupe(findings: SecurityFinding[]): SecurityFinding[] {
  const seen = new Set<string>();
  return findings.filter((finding) => {
    const key = `${finding.source}|${finding.package}|${finding.severity}|${finding.title}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function sortFindings(findings: SecurityFinding[]): SecurityFinding[] {
  return [...findings].sort(
    (a, b) =>
      SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] ||
      a.source.localeCompare(b.source) ||
      a.package.localeCompare(b.package),
  );
}

function parseJsonOutput(stdout: string): string | null {
  const trimmed = stdout.trim();
  if (!trimmed) return null;
  const firstBrace = trimmed.indexOf('{');
  const lastBrace = trimmed.lastIndexOf('}');
  if (firstBrace < 0 || lastBrace <= firstBrace) return null;
  const candidate = trimmed.slice(firstBrace, lastBrace + 1);
  try {
    JSON.parse(candidate);
    return candidate;
  } catch {
    return null;
  }
}

async function runAudit(
  runner: CommandRunner,
  command: string,
  args: string[],
  cwd: string,
): Promise<{ stdout: string; stderr: string; code: number }> {
  try {
    return await runner(command, args, cwd);
  } catch (err) {
    return { stdout: '', stderr: (err as Error).message, code: 1 };
  }
}

export async function runSecurityAudit(
  repoRoot: string,
  runner: CommandRunner = defaultRunner,
): Promise<SecurityAuditResult> {
  const findings: SecurityFinding[] = [];
  const skipped: string[] = [];
  const errors: string[] = [];

  for (const [label, rel] of [
    ['bun (root)', '.'],
    ['bun (worker)', 'worker'],
    ['bun (site)', 'site'],
  ] as const) {
    const cwd = path.join(repoRoot, rel);
    if (!fs.existsSync(path.join(cwd, 'bun.lock'))) {
      skipped.push(`${label}: no bun.lock`);
      continue;
    }
    const res = await runAudit(runner, 'bun', ['audit', '--json'], cwd);
    const json = parseJsonOutput(res.stdout);
    if (json) {
      findings.push(...parseBunAuditJson(json, label));
    } else if (res.code !== 0) {
      errors.push(`${label}: ${res.stderr.trim() || 'audit failed'}`);
    }
  }

  if (fs.existsSync(path.join(repoRoot, 'package-lock.json'))) {
    const res = await runAudit(
      runner,
      'npm',
      ['audit', '--json', '--package-lock-only'],
      repoRoot,
    );
    const json = parseJsonOutput(res.stdout);
    if (json) {
      findings.push(...parseNpmAuditJson(json, 'npm (release lock)'));
    } else if (res.code !== 0) {
      errors.push(`npm (release lock): ${res.stderr.trim() || 'audit failed'}`);
    }
  } else {
    skipped.push('npm (release lock): no package-lock.json');
  }

  const cargoAudit = await runAudit(
    runner,
    'cargo',
    ['audit', '--version'],
    repoRoot,
  );
  if (cargoAudit.code !== 0) {
    skipped.push(
      'cargo: cargo-audit is not installed (install with `cargo install cargo-audit`)',
    );
  } else {
    for (const project of RUST_PROJECTS) {
      const lockPath = path.join(repoRoot, project.lockfile);
      if (!fs.existsSync(lockPath)) {
        skipped.push(`cargo (${project.label}): no ${project.lockfile}`);
        continue;
      }
      const res = await runAudit(
        runner,
        'cargo',
        ['audit', '--json'],
        path.dirname(lockPath),
      );
      const json = parseJsonOutput(res.stdout);
      if (json) {
        findings.push(...parseCargoAuditJson(json, `cargo (${project.label})`));
      } else if (res.code !== 0) {
        errors.push(
          `cargo (${project.label}): ${res.stderr.trim() || 'audit failed'}`,
        );
      }
    }
  }

  return {
    findings: sortFindings(dedupe(findings)),
    skipped,
    errors,
  };
}

export function formatSecurityReport(result: SecurityAuditResult): string {
  const lines: string[] = ['', '=== DEPENDENCY SECURITY AUDIT (opt-in) ==='];
  if (result.findings.length === 0) {
    lines.push('  No known advisories reported.');
  } else {
    const counts = new Map<SecuritySeverity, number>();
    for (const finding of result.findings) {
      counts.set(finding.severity, (counts.get(finding.severity) ?? 0) + 1);
    }
    const summary = [...counts.entries()]
      .map(([severity, count]) => `${severity}: ${count}`)
      .join(', ');
    lines.push(`  Findings: ${result.findings.length} (${summary})`);
    lines.push('');
    for (const finding of result.findings) {
      lines.push(
        `  [${finding.severity}] ${finding.package} (${finding.source}): ${finding.title}`,
      );
      if (finding.url) lines.push(`      ${finding.url}`);
    }
  }
  if (result.skipped.length > 0) {
    lines.push('');
    lines.push('  Skipped:');
    for (const item of result.skipped) lines.push(`    - ${item}`);
  }
  if (result.errors.length > 0) {
    lines.push('');
    lines.push('  Errors:');
    for (const item of result.errors) lines.push(`    - ${item}`);
  }
  lines.push('');
  return lines.join('\n');
}
