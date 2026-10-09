import type { CompatibilityRecord, FailureStage } from './types';

export interface CompatBadgeData {
  label: string;
  bg: string;
  text: string;
  border: string;
  dot: string;
  titleAttr: string;
  isLinkable: boolean;
  workflowRunUrl?: string | null;
}

export function escapeHtml(unsafe: string): string {
  return unsafe
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

export function resolveErrorLabel(
  stage?: FailureStage | null,
  reason?: string | null,
): string {
  if (stage === 'acquisition') return 'APK download failed';
  if (stage === 'r2-lookup') return 'R2 lookup failed';
  if (stage === 'r2-upload') return 'R2 upload failed';
  if (stage === 'r2-download') return 'R2 download failed';
  if (stage === 'compatibility-check') return 'Test error';
  if (stage === 'pipeline') return 'Pipeline error';

  const reasonLower = (reason || '').toLowerCase();
  if (
    reasonLower.includes('acquisition') ||
    reasonLower.includes('no fixture slot') ||
    reasonLower.includes('no apk') ||
    reasonLower.includes('missing fixture')
  ) {
    return 'APK download failed';
  }
  if (reasonLower.includes('download')) return 'R2 download failed';
  if (reasonLower.includes('upload')) return 'R2 upload failed';
  if (reasonLower.includes('lookup')) return 'R2 lookup failed';
  return 'Test error';
}

export function formatCompatBadge(
  compatRecord: CompatibilityRecord | null | undefined,
  defaultLabel = 'Not tested',
  isQueued = false,
): CompatBadgeData {
  if (isQueued || compatRecord?.status === 'queued') {
    return {
      label: 'Queued',
      bg: 'bg-amber-950/60',
      text: 'text-amber-400',
      border: 'border-amber-800/40',
      dot: 'bg-amber-400',
      titleAttr: 'Tests queued',
      isLinkable: Boolean(compatRecord?.workflowRunUrl),
      workflowRunUrl: compatRecord?.workflowRunUrl,
    };
  }

  if (compatRecord?.status === 'running') {
    return {
      label: 'Running',
      bg: 'bg-amber-950/60',
      text: 'text-amber-400',
      border: 'border-amber-800/40',
      dot: 'bg-amber-400',
      titleAttr: 'Tests running',
      isLinkable: Boolean(compatRecord?.workflowRunUrl),
      workflowRunUrl: compatRecord?.workflowRunUrl,
    };
  }

  if (!compatRecord) {
    return {
      label: defaultLabel || 'Not tested',
      bg: 'bg-zinc-900',
      text: 'text-zinc-500',
      border: 'border-white/10',
      dot: 'bg-zinc-500',
      titleAttr: '',
      isLinkable: false,
      workflowRunUrl: null,
    };
  }

  const s = compatRecord.status;
  const passed =
    typeof compatRecord.passedCount === 'number' ? compatRecord.passedCount : 0;
  const failed =
    typeof compatRecord.failedCount === 'number' ? compatRecord.failedCount : 0;
  const total = passed + failed;

  if (s === 'compatible') {
    const title =
      total > 0
        ? `${passed} of ${total} patch tests passed (compatible)`
        : 'All patch tests passed';
    return {
      label: 'Compatible',
      bg: 'bg-emerald-950/60',
      text: 'text-emerald-400',
      border: 'border-emerald-800/40',
      dot: 'bg-emerald-400',
      titleAttr: title,
      isLinkable: Boolean(compatRecord.workflowRunUrl),
      workflowRunUrl: compatRecord.workflowRunUrl,
    };
  }

  if (s === 'incompatible') {
    const label = total > 0 ? `${passed}/${total}` : 'Incompatible';
    const title =
      total > 0
        ? `${passed} of ${total} patch tests passed (incompatible)`
        : 'Patch compatibility check failed';
    return {
      label,
      bg: 'bg-rose-950/60',
      text: 'text-rose-400',
      border: 'border-rose-800/40',
      dot: 'bg-rose-400',
      titleAttr: title,
      isLinkable: Boolean(compatRecord.workflowRunUrl),
      workflowRunUrl: compatRecord.workflowRunUrl,
    };
  }

  if (s === 'error') {
    const label = resolveErrorLabel(
      compatRecord.failureStage,
      compatRecord.failureReason,
    );
    const title = compatRecord.failureReason || 'Target check failed';
    return {
      label,
      bg: 'bg-rose-950/60',
      text: 'text-rose-400',
      border: 'border-rose-800/40',
      dot: 'bg-rose-400',
      titleAttr: title,
      isLinkable: Boolean(compatRecord.workflowRunUrl),
      workflowRunUrl: compatRecord.workflowRunUrl,
    };
  }

  return {
    label: defaultLabel || 'Not tested',
    bg: 'bg-zinc-900',
    text: 'text-zinc-500',
    border: 'border-white/10',
    dot: 'bg-zinc-500',
    titleAttr: '',
    isLinkable: false,
    workflowRunUrl: null,
  };
}

export function renderCompatBadgeHtml(
  compatRecord: CompatibilityRecord | null | undefined,
  defaultLabel = 'Not tested',
  isQueued = false,
): string {
  const badge = formatCompatBadge(compatRecord, defaultLabel, isQueued);
  const titleAttr = badge.titleAttr
    ? ` title="${escapeHtml(badge.titleAttr)}"`
    : '';

  const spanContent = `<span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-sm text-[10px] ${badge.bg} ${badge.text} border ${badge.border}"${titleAttr}><span class="w-1.5 h-1.5 rounded-full ${badge.dot}"></span><span>${escapeHtml(badge.label)}</span></span>`;

  if (badge.isLinkable && badge.workflowRunUrl) {
    const linkTitle = badge.titleAttr
      ? ` title="${escapeHtml(badge.titleAttr)}"`
      : ' title="CI Run"';
    return `<a href="${escapeHtml(badge.workflowRunUrl)}" target="_blank" rel="noopener noreferrer" class="hover:opacity-80 transition-opacity"${linkTitle}>${spanContent}</a>`;
  }

  return spanContent;
}
