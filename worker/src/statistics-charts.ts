import { escapeXml } from './svg';
import type { DownloadSnapshot, ReleaseLineDownloads } from './types';

const FONT_FAMILY =
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
const MONO_FONT =
  "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace";

function formatNumber(num: number): string {
  return num.toLocaleString('en-US');
}

function calculateNiceMax(rawMax: number): number {
  if (rawMax <= 0) return 10;
  const power = Math.pow(10, Math.floor(Math.log10(rawMax)));
  const fraction = rawMax / power;
  let niceFraction: number;
  if (fraction <= 1) niceFraction = 1;
  else if (fraction <= 2) niceFraction = 2;
  else if (fraction <= 5) niceFraction = 5;
  else niceFraction = 10;
  return niceFraction * power;
}

export function renderTotalDownloadsChartSvg(
  history: DownloadSnapshot[],
  updatedAt?: string,
): string {
  const width = 800;
  const height = 258;
  const safeUpdated = updatedAt ? escapeXml(updatedAt.slice(0, 10)) : '';

  if (history.length === 0) {
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="Total Downloads Chart">
  <rect width="100%" height="100%" fill="#160d2b" rx="8"/>
  <text x="16" y="28" font-family="${FONT_FAMILY}" font-size="16" font-weight="600" fill="#f5f3ff">Total Downloads</text>
  ${safeUpdated ? `<text x="784" y="28" font-family="${FONT_FAMILY}" font-size="11" fill="#c4b5fd" text-anchor="end">Updated ${safeUpdated}</text>` : ''}
  <text x="400" y="145" font-family="${FONT_FAMILY}" font-size="14" fill="#c4b5fd" text-anchor="middle">No download data yet</text>
</svg>`;
  }

  const plotLeft = 52;
  const plotRight = 784;
  const plotTop = 46;
  const plotBottom = 220;
  const plotWidth = plotRight - plotLeft;
  const plotHeight = plotBottom - plotTop;

  const rawMax = Math.max(...history.map((h) => h.totalDownloads));
  const yMax = calculateNiceMax(rawMax);
  const yTicksCount = 4;
  const yStep = yMax / yTicksCount;

  let gridLines = '';
  for (let i = 0; i <= yTicksCount; i++) {
    const val = Math.round(i * yStep);
    const y = plotBottom - (i / yTicksCount) * plotHeight;
    gridLines += `  <line x1="${plotLeft}" y1="${y}" x2="${plotRight}" y2="${y}" stroke="#4c3575" stroke-dasharray="4 4" stroke-width="1"/>\n`;
    gridLines += `  <text x="${plotLeft - 6}" y="${y + 4}" font-family="${FONT_FAMILY}" font-size="10.5" fill="#c4b5fd" text-anchor="end">${formatNumber(val)}</text>\n`;
  }
  let dataPath = '';
  let areaPath = '';
  let pointsMarkup = '';
  let xLabelsMarkup = '';

  if (history.length === 1) {
    const pt = history[0];
    const cx = plotLeft + plotWidth / 2;
    const cy = plotBottom - (pt.totalDownloads / yMax) * plotHeight;
    pointsMarkup += `  <circle cx="${cx}" cy="${cy}" r="5" fill="#a78bfa" stroke="#160d2b" stroke-width="2"/>\n`;
    pointsMarkup += `  <text x="${cx}" y="${cy - 10}" font-family="${FONT_FAMILY}" font-size="11" font-weight="600" fill="#f5f3ff" text-anchor="middle">${formatNumber(pt.totalDownloads)}</text>\n`;
    xLabelsMarkup += `  <text x="${cx}" y="${plotBottom + 19}" font-family="${FONT_FAMILY}" font-size="10.5" fill="#c4b5fd" text-anchor="middle">${escapeXml(pt.date)}</text>\n`;
  } else {
    const n = history.length;
    const coords: Array<{ x: number; y: number; val: number; date: string }> =
      [];

    for (let i = 0; i < n; i++) {
      const x = plotLeft + (i / (n - 1)) * plotWidth;
      const y = plotBottom - (history[i].totalDownloads / yMax) * plotHeight;
      coords.push({
        x,
        y,
        val: history[i].totalDownloads,
        date: history[i].date,
      });
    }

    const pathCommands = coords
      .map(
        (c, i) => `${i === 0 ? 'M' : 'L'} ${c.x.toFixed(1)} ${c.y.toFixed(1)}`,
      )
      .join(' ');
    dataPath = `  <path d="${pathCommands}" fill="none" stroke="#a78bfa" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>\n`;

    const first = coords[0];
    const last = coords[coords.length - 1];
    areaPath = `  <path d="${pathCommands} L ${last.x.toFixed(1)} ${plotBottom} L ${first.x.toFixed(1)} ${plotBottom} Z" fill="url(#violetGradient)" opacity="0.25"/>\n`;

    // Only draw circle markers if there aren't too many points, plus always for the latest point
    if (n <= 30) {
      for (const pt of coords) {
        pointsMarkup += `  <circle cx="${pt.x.toFixed(1)}" cy="${pt.y.toFixed(1)}" r="3.5" fill="#a78bfa"/>\n`;
      }
    } else {
      pointsMarkup += `  <circle cx="${last.x.toFixed(1)}" cy="${last.y.toFixed(1)}" r="4" fill="#a78bfa"/>\n`;
    }

    // Adaptive X-axis labels (at most 5 visible)
    const maxLabels = Math.min(5, n);
    const chosenIndices = new Set<number>();
    for (let k = 0; k < maxLabels; k++) {
      const idx = Math.round((k * (n - 1)) / (maxLabels - 1));
      chosenIndices.add(idx);
    }

    for (const idx of chosenIndices) {
      const c = coords[idx];
      let anchor = 'middle';
      if (idx === 0) anchor = 'start';
      else if (idx === n - 1) anchor = 'end';
      xLabelsMarkup += `  <text x="${c.x.toFixed(1)}" y="${plotBottom + 19}" font-family="${FONT_FAMILY}" font-size="10.5" fill="#c4b5fd" text-anchor="${anchor}">${escapeXml(c.date)}</text>\n`;
    }
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="Total Downloads Chart">
  <defs>
    <linearGradient id="violetGradient" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#a78bfa" stop-opacity="0.8"/>
      <stop offset="100%" stop-color="#a78bfa" stop-opacity="0.0"/>
    </linearGradient>
  </defs>
  <rect width="100%" height="100%" fill="#160d2b" rx="8"/>
  <text x="16" y="28" font-family="${FONT_FAMILY}" font-size="16" font-weight="600" fill="#f5f3ff">Total Downloads</text>
  ${safeUpdated ? `<text x="784" y="28" font-family="${FONT_FAMILY}" font-size="11" fill="#c4b5fd" text-anchor="end">Updated ${safeUpdated}</text>` : ''}
${gridLines}
  <line x1="${plotLeft}" y1="${plotBottom}" x2="${plotRight}" y2="${plotBottom}" stroke="#4c3575" stroke-width="1.5"/>
  <line x1="${plotLeft}" y1="${plotTop}" x2="${plotLeft}" y2="${plotBottom}" stroke="#4c3575" stroke-width="1.5"/>
${areaPath}${dataPath}${pointsMarkup}${xLabelsMarkup}</svg>`;
}

export function renderReleaseDownloadsChartSvg(
  lines: ReleaseLineDownloads[],
  updatedAt?: string,
): string {
  const width = 800;
  const safeUpdated = updatedAt ? escapeXml(updatedAt.slice(0, 10)) : '';

  if (lines.length === 0) {
    const emptyHeight = 130;
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${emptyHeight}" width="${width}" height="${emptyHeight}" role="img" aria-label="Release Downloads Chart">
  <rect width="100%" height="100%" fill="#160d2b" rx="8"/>
  <text x="16" y="28" font-family="${FONT_FAMILY}" font-size="16" font-weight="600" fill="#f5f3ff">Release Downloads</text>
  ${safeUpdated ? `<text x="784" y="28" font-family="${FONT_FAMILY}" font-size="11" fill="#c4b5fd" text-anchor="end">Updated ${safeUpdated}</text>` : ''}
  <text x="400" y="85" font-family="${FONT_FAMILY}" font-size="14" fill="#c4b5fd" text-anchor="middle">No download data yet</text>
</svg>`;
  }

  const rowHeight = 32;
  const topOffset = 46;
  const barStart = 58;
  const maxBarWidth = 660;
  const maxVal = Math.max(...lines.map((l) => l.totalDownloads), 1);
  const height = topOffset + (lines.length - 1) * rowHeight + 20 + 16;
  let rowsMarkup = '';
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const y = topOffset + i * rowHeight;
    const barWidth =
      line.totalDownloads > 0
        ? Math.max(3, Math.round((line.totalDownloads / maxVal) * maxBarWidth))
        : 0;
    const safeLabel = escapeXml(line.label);
    const formattedCount = formatNumber(line.totalDownloads);

    rowsMarkup += `  <text x="16" y="${y + 15}" font-family="${MONO_FONT}" font-size="12.5" font-weight="600" fill="#f5f3ff">${safeLabel}</text>\n`;
    if (barWidth > 0) {
      rowsMarkup += `  <rect x="${barStart}" y="${y}" width="${barWidth}" height="20" rx="4" fill="#a78bfa"/>\n`;
    }
    rowsMarkup += `  <text x="${barStart + barWidth + 8}" y="${y + 15}" font-family="${FONT_FAMILY}" font-size="11.5" font-weight="500" fill="#c4b5fd">${formattedCount}</text>\n`;
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="Release Downloads Chart">
  <rect width="100%" height="100%" fill="#160d2b" rx="8"/>
  <text x="16" y="28" font-family="${FONT_FAMILY}" font-size="16" font-weight="600" fill="#f5f3ff">Release Downloads</text>
  ${safeUpdated ? `<text x="784" y="28" font-family="${FONT_FAMILY}" font-size="11" fill="#c4b5fd" text-anchor="end">Updated ${safeUpdated}</text>` : ''}
${rowsMarkup}</svg>`;
}

export function renderErrorChartSvg(title: string, message: string): string {
  const width = 800;
  const height = 200;
  const safeTitle = escapeXml(title);
  const safeMessage = escapeXml(message);

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="${safeTitle}">
  <rect width="100%" height="100%" fill="#160d2b" rx="8"/>
  <text x="16" y="28" font-family="${FONT_FAMILY}" font-size="16" font-weight="600" fill="#f5f3ff">${safeTitle}</text>
  <text x="400" y="115" font-family="${FONT_FAMILY}" font-size="15" fill="#f87171" text-anchor="middle">${safeMessage}</text>
</svg>`;
}
