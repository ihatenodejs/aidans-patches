import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as path from 'node:path';

export type DependencyStatus =
  | 'current'
  | 'update'
  | 'held'
  | 'moving'
  | 'ahead'
  | 'unused'
  | 'not-checked'
  | 'unsupported'
  | 'error';

export interface LocationRef {
  file: string;
  line?: number;
  col?: number;
  context?: string;
}

export interface DependencyItem {
  id: string;
  ecosystem: string;
  name: string;
  locations: string[];
  declared: string;
  resolved: string | null;
  /** Plain upstream version used for proposals and comparisons. */
  latest: string | null;
  /** Human-facing latest value when it carries more than a version. */
  latestDisplay?: string;
  /** Human label coupled to the latest version, e.g. NDK r30. */
  latestLabel?: string;
  /** SHA-256 of the latest downloadable artifact, when the pin has one. */
  latestAsset?: string;
  /** Download URL for the latest artifact, when the pin has one. */
  latestUrl?: string;
  /** Versioned executable path for the latest artifact, when applicable. */
  latestExecutablePath?: string;
  /** Version probe pattern for the latest artifact, when applicable. */
  latestProbePattern?: string;
  status: DependencyStatus;
  reason?: string;
  sourceUrl?: string;
  durationMs?: number;
}

export interface ProposalInput {
  path: string;
  sha256: string;
}

export interface ProposalUpdate {
  id: string;
  from: string;
  to: string;
  commit?: string;
  /** SHA-256 of the replacement artifact. */
  assetIdentity?: string;
  /** Replacement download URL for artifact-based pins. */
  assetUrl?: string;
  /** Replacement executable path for artifact-based pins. */
  executablePath?: string;
  /** Replacement version probe pattern for artifact-based pins. */
  versionProbePattern?: string;
  /** Human label that must stay coupled to the numeric pin, e.g. NDK r30. */
  label?: string;
}

export interface Proposal {
  schemaVersion: 1;
  inputs: ProposalInput[];
  updates: ProposalUpdate[];
  digest: string;
}

export interface DependencySummary {
  total: number;
  current: number;
  update: number;
  held: number;
  moving: number;
  ahead: number;
  unused: number;
  notChecked: number;
  unsupported: number;
  error: number;
  durationMs: number;
}

export interface DependencyReport {
  schemaVersion: 1;
  items: DependencyItem[];
  summary: DependencySummary;
  proposal: Proposal | null;
}

export interface PolicyHold {
  id: string;
  value: string;
  reason: string;
}

export interface DependencyPolicy {
  schemaVersion: 1;
  holds: PolicyHold[];
}

export function canonicalJson(obj: unknown): string {
  if (obj === null || typeof obj !== 'object') {
    return JSON.stringify(obj);
  }
  if (Array.isArray(obj)) {
    return '[' + obj.map((item) => canonicalJson(item)).join(',') + ']';
  }
  const keys = Object.keys(obj as Record<string, unknown>).sort();
  const entries = keys.map(
    (k) =>
      JSON.stringify(k) +
      ':' +
      canonicalJson((obj as Record<string, unknown>)[k]),
  );
  return '{' + entries.join(',') + '}';
}

export function computeProposalDigest(
  inputs: ProposalInput[],
  updates: ProposalUpdate[],
): string {
  const sortedInputs = [...inputs].sort((a, b) => a.path.localeCompare(b.path));
  const sortedUpdates = [...updates].sort((a, b) => a.id.localeCompare(b.id));
  const canonicalPayload = canonicalJson({
    schemaVersion: 1,
    inputs: sortedInputs,
    updates: sortedUpdates,
  });
  return crypto
    .createHash('sha256')
    .update(canonicalPayload, 'utf8')
    .digest('hex');
}

export function loadAndValidatePolicy(
  repoRoot: string,
  policyFileName = 'dependency-policy.json',
): DependencyPolicy {
  const policyPath = path.resolve(repoRoot, policyFileName);
  if (!fs.existsSync(policyPath)) {
    throw new Error(`Dependency policy file not found at ${policyPath}`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(fs.readFileSync(policyPath, 'utf8'));
  } catch (err) {
    throw new Error(
      `Malformed JSON in dependency policy file ${policyPath}: ${(err as Error).message}`,
    );
  }

  if (typeof parsed !== 'object' || parsed === null) {
    throw new Error(`Policy in ${policyPath} must be an object`);
  }

  const p = parsed as Partial<DependencyPolicy>;
  if (p.schemaVersion !== 1) {
    throw new Error(`Policy in ${policyPath} must have schemaVersion: 1`);
  }

  if (!Array.isArray(p.holds)) {
    throw new Error(`Policy in ${policyPath} must specify a 'holds' array`);
  }

  const seenIds = new Set<string>();
  const validatedHolds: PolicyHold[] = [];

  for (let i = 0; i < p.holds.length; i++) {
    const h = p.holds[i];
    if (typeof h !== 'object' || h === null) {
      throw new Error(`Policy hold at index ${i} must be an object`);
    }
    if (typeof h.id !== 'string' || !h.id.trim()) {
      throw new Error(
        `Policy hold at index ${i} must have a non-empty string 'id'`,
      );
    }
    if (seenIds.has(h.id)) {
      throw new Error(`Duplicate hold id found in policy: ${h.id}`);
    }
    seenIds.add(h.id);

    if (typeof h.value !== 'string') {
      throw new Error(`Policy hold '${h.id}' must specify a string 'value'`);
    }
    if (typeof h.reason !== 'string' || !h.reason.trim()) {
      throw new Error(
        `Policy hold '${h.id}' requires a mandatory non-empty text 'reason' explaining why it is held`,
      );
    }

    validatedHolds.push({
      id: h.id.trim(),
      value: h.value,
      reason: h.reason.trim(),
    });
  }

  return {
    schemaVersion: 1,
    holds: validatedHolds,
  };
}
