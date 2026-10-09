import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  formatCompatBadge,
  renderCompatBadgeHtml,
  resolveErrorLabel,
} from './status';
import type { CompatibilityRecord } from './types';

describe('resolveErrorLabel', () => {
  it('resolves explicit failureStage accurately', () => {
    assert.equal(resolveErrorLabel('acquisition'), 'APK download failed');
    assert.equal(resolveErrorLabel('r2-lookup'), 'R2 lookup failed');
    assert.equal(resolveErrorLabel('r2-upload'), 'R2 upload failed');
    assert.equal(resolveErrorLabel('r2-download'), 'R2 download failed');
    assert.equal(resolveErrorLabel('compatibility-check'), 'Test error');
    assert.equal(resolveErrorLabel('pipeline'), 'Pipeline error');
  });

  it('falls back to keyword matching when failureStage is null/undefined', () => {
    assert.equal(
      resolveErrorLabel(null, 'No fixture slot matches target version'),
      'APK download failed',
    );
    assert.equal(
      resolveErrorLabel(undefined, 'Target acquisition failed'),
      'APK download failed',
    );
    assert.equal(
      resolveErrorLabel(null, 'Failed to download from R2'),
      'R2 download failed',
    );
    assert.equal(
      resolveErrorLabel(null, 'Failed to upload to R2'),
      'R2 upload failed',
    );
    assert.equal(
      resolveErrorLabel(null, 'R2 lookup head failed'),
      'R2 lookup failed',
    );
    assert.equal(
      resolveErrorLabel(null, 'Morphe crashed with SIGSEGV'),
      'Test error',
    );
  });
});

describe('formatCompatBadge', () => {
  it('handles queued state', () => {
    const badge = formatCompatBadge(null, 'Not tested', true);
    assert.equal(badge.label, 'Queued');
    assert.equal(badge.text, 'text-amber-400');
    assert.equal(badge.bg, 'bg-amber-950/60');
  });

  it('handles running state', () => {
    const record: CompatibilityRecord = {
      requestId: 'req-1',
      role: 'target',
      versionName: '1.0.0',
      versionCode: 10,
      patchBundleVersion: '1.4.0',
      gitRevision: 'rev1',
      testedAt: '2026-10-06T00:00:00Z',
      passedCount: 0,
      failedCount: 0,
      status: 'running',
    };
    const badge = formatCompatBadge(record);
    assert.equal(badge.label, 'Running');
    assert.equal(badge.text, 'text-amber-400');
  });

  it('handles null / missing record', () => {
    const badge = formatCompatBadge(null);
    assert.equal(badge.label, 'Not tested');
    assert.equal(badge.text, 'text-zinc-500');
    assert.equal(badge.isLinkable, false);
  });

  it('handles compatible record', () => {
    const record: CompatibilityRecord = {
      requestId: 'req-1',
      role: 'target',
      versionName: '1.0.0',
      versionCode: 10,
      patchBundleVersion: '1.4.0',
      gitRevision: 'rev1',
      testedAt: '2026-10-06T00:00:00Z',
      passedCount: 5,
      failedCount: 0,
      status: 'compatible',
      workflowRunUrl: 'https://github.com/test/runs/1',
    };
    const badge = formatCompatBadge(record);
    assert.equal(badge.label, 'Compatible');
    assert.equal(badge.text, 'text-emerald-400');
    assert.equal(badge.isLinkable, true);
    assert.equal(badge.workflowRunUrl, 'https://github.com/test/runs/1');
  });

  it('handles incompatible record with counts', () => {
    const record: CompatibilityRecord = {
      requestId: 'req-1',
      role: 'target',
      versionName: '1.0.0',
      versionCode: 10,
      patchBundleVersion: '1.4.0',
      gitRevision: 'rev1',
      testedAt: '2026-10-06T00:00:00Z',
      passedCount: 4,
      failedCount: 1,
      status: 'incompatible',
    };
    const badge = formatCompatBadge(record);
    assert.equal(badge.label, '4/5');
    assert.equal(badge.text, 'text-rose-400');
  });

  it('handles stage-specific error records', () => {
    const stages = [
      { stage: 'acquisition' as const, expected: 'APK download failed' },
      { stage: 'r2-lookup' as const, expected: 'R2 lookup failed' },
      { stage: 'r2-upload' as const, expected: 'R2 upload failed' },
      { stage: 'r2-download' as const, expected: 'R2 download failed' },
      { stage: 'compatibility-check' as const, expected: 'Test error' },
      { stage: 'pipeline' as const, expected: 'Pipeline error' },
    ];

    for (const { stage, expected } of stages) {
      const record: CompatibilityRecord = {
        requestId: 'req-1',
        role: 'target',
        versionName: '1.0.0',
        versionCode: 0,
        patchBundleVersion: '1.4.0',
        gitRevision: 'rev1',
        testedAt: '2026-10-06T00:00:00Z',
        passedCount: 0,
        failedCount: 1,
        status: 'error',
        failureStage: stage,
        failureReason: `${stage} failed details`,
        workflowRunUrl: 'https://github.com/test/runs/99',
      };
      const badge = formatCompatBadge(record);
      assert.equal(badge.label, expected);
      assert.equal(badge.text, 'text-rose-400');
      assert.equal(badge.isLinkable, true);
      assert.equal(badge.titleAttr, `${stage} failed details`);
    }
  });
});

describe('renderCompatBadgeHtml', () => {
  it('escapes failure reason and creates link when workflowRunUrl present', () => {
    const record: CompatibilityRecord = {
      requestId: 'req-1',
      role: 'target',
      versionName: '1.0.0',
      versionCode: 0,
      patchBundleVersion: '1.4.0',
      gitRevision: 'rev1',
      testedAt: '2026-10-06T00:00:00Z',
      passedCount: 0,
      failedCount: 1,
      status: 'error',
      failureStage: 'acquisition',
      failureReason: '<Script>alert("xss")</Script>',
      workflowRunUrl: 'https://github.com/test/runs/42',
    };
    const html = renderCompatBadgeHtml(record);
    assert.ok(html.startsWith('<a href="https://github.com/test/runs/42"'));
    assert.ok(
      html.includes('&lt;Script&gt;alert(&quot;xss&quot;)&lt;/Script&gt;'),
    );
    assert.ok(html.includes('APK download failed'));
  });
});
