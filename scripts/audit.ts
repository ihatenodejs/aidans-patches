#!/usr/bin/env bun
import { spawn } from 'node:child_process';
import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as path from 'node:path';

const KTLINT_1_5_0_SHA256 =
  'a16be01dcc480aab2f55f444b620142152f66e31564b3b9376506d624c28a2ad';
// --- ANSI Styling ---
const c = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
  cyan: '\x1b[36m',
  gray: '\x1b[90m',
  bgRed: '\x1b[41m\x1b[37m',
  bgGreen: '\x1b[42m\x1b[30m',
};

const sym = {
  ok: `${c.green}✔${c.reset}`,
  err: `${c.red}✖${c.reset}`,
  warn: `${c.yellow}⚠${c.reset}`,
  info: `${c.cyan}ℹ${c.reset}`,
  arrow: `${c.gray}→${c.reset}`,
};

interface StepResult {
  title: string;
  tool: string;
  success: boolean;
  durationMs: number;
  summary: string;
  diagnostics?: Diagnostic[];
  output?: string;
}

interface Diagnostic {
  tool: string;
  file: string;
  line?: number;
  col?: number;
  message: string;
  rule?: string;
  severity: 'error' | 'warning';
}

function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(2)}s`;
}

async function execCommand(
  cmd: string,
  args: string[],
  cwd = process.cwd(),
): Promise<{ code: number; stdout: string; stderr: string; durationMs: number }> {
  const start = performance.now();
  const { promise, resolve } = Promise.withResolvers<{
    code: number;
    stdout: string;
    stderr: string;
    durationMs: number;
  }>();

  const child = spawn(cmd, args, {
    cwd,
    shell: false,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, FORCE_COLOR: '1' },
  });

  let stdout = '';
  let stderr = '';

  child.stdout.on('data', (d) => (stdout += d.toString()));
  child.stderr.on('data', (d) => (stderr += d.toString()));

  child.on('close', (code) => {
    const durationMs = Math.round(performance.now() - start);
    resolve({ code: code ?? 1, stdout, stderr, durationMs });
  });

  child.on('error', (err) => {
    const durationMs = Math.round(performance.now() - start);
    resolve({ code: 1, stdout, stderr: stderr + err.message, durationMs });
  });

  return promise;
}

async function ensureKtlint(): Promise<string> {
  const localBin = path.join(process.cwd(), '.tools', 'ktlint');
  if (fs.existsSync(localBin)) {
    return localBin;
  }
  const systemKtlint = Bun.which('ktlint');
  if (systemKtlint) return systemKtlint;

  console.log(`${sym.info} Fetching pinned ktlint 1.5.0 binary to .tools/ktlint...`);
  fs.mkdirSync(path.dirname(localBin), { recursive: true });
  const res = await fetch('https://github.com/pinterest/ktlint/releases/download/1.5.0/ktlint');
  if (!res.ok) throw new Error(`Failed to download ktlint: ${res.statusText}`);
  const buf = Buffer.from(await res.arrayBuffer());
  const downloadedSha256 = crypto
    .createHash('sha256')
    .update(buf)
    .digest('hex');
  if (downloadedSha256 !== KTLINT_1_5_0_SHA256) {
    throw new Error(
      `ktlint 1.5.0 SHA-256 verification failed: expected ${KTLINT_1_5_0_SHA256}, got ${downloadedSha256}`,
    );
  }
  fs.writeFileSync(localBin, buf, { mode: 0o755 });
  return localBin;
}
function resolveRuffCommand(): { cmd: string; baseArgs: string[] } {
  const systemRuff = Bun.which('ruff');
  if (systemRuff) {
    return { cmd: systemRuff, baseArgs: [] };
  }
  const venvRuff = path.join(
    process.cwd(),
    'tools',
    'apk-lab',
    '.venv',
    process.platform === 'win32' ? 'Scripts' : 'bin',
    process.platform === 'win32' ? 'ruff.exe' : 'ruff',
  );
  if (fs.existsSync(venvRuff)) {
    return { cmd: venvRuff, baseArgs: [] };
  }
  const systemUvx = Bun.which('uvx');
  if (systemUvx) {
    return { cmd: systemUvx, baseArgs: ['ruff'] };
  }
  const systemUv = Bun.which('uv');
  if (systemUv) {
    return { cmd: systemUv, baseArgs: ['tool', 'run', 'ruff'] };
  }
  return { cmd: 'ruff', baseArgs: [] };
}


function printSectionHeader(title: string, subtitle?: string): void {
  console.log();
  console.log(`${c.bold}${c.cyan}=== ${title.toUpperCase()} ===${c.reset}${subtitle ? ` ${c.gray}(${subtitle})${c.reset}` : ''}`);
}

function printResultsSummary(category: string, results: StepResult[]): boolean {
  const totalDuration = results.reduce((acc, r) => acc + r.durationMs, 0);
  const allPassed = results.every((r) => r.success);

  console.log();
  console.log(`${c.bold}${c.gray}${'―'.repeat(70)}${c.reset}`);
  console.log(`${c.bold}  ${category.toUpperCase()} SUMMARY${c.reset}`);
  console.log(`${c.bold}${c.gray}${'―'.repeat(70)}${c.reset}`);

  for (const res of results) {
    const icon = res.success ? sym.ok : sym.err;
    const dur = c.gray + formatDuration(res.durationMs).padStart(8) + c.reset;
    const name = res.title.padEnd(28);
    console.log(`  ${icon} ${name} ${dur}  ${c.dim}${res.summary}${c.reset}`);
  }

  console.log(`${c.bold}${c.gray}${'―'.repeat(70)}${c.reset}`);
  if (allPassed) {
    console.log(`  ${c.bold}${c.green}✔ All ${results.length} checks passed cleanly${c.reset} ${c.gray}in ${formatDuration(totalDuration)}${c.reset}`);
  } else {
    const failedCount = results.filter((r) => !r.success).length;
    console.log(`  ${c.bold}${c.red}✖ ${failedCount} of ${results.length} checks failed${c.reset} ${c.gray}in ${formatDuration(totalDuration)}${c.reset}`);
  }
  console.log();
  return allPassed;
}

function renderDiagnostics(diagnostics: Diagnostic[]): void {
  if (diagnostics.length === 0) return;
  console.log();
  for (const d of diagnostics) {
    const loc = `${d.file}${d.line ? `:${d.line}${d.col ? `:${d.col}` : ''}` : ''}`;
    const badge = d.severity === 'error' ? `${c.red}error${c.reset}` : `${c.yellow}warning${c.reset}`;
    const rule = d.rule ? ` ${c.gray}(${d.rule})${c.reset}` : '';
    const toolBadge = `${c.cyan}[${d.tool}]${c.reset}`;
    console.log(`  ${badge} ${c.bold}${loc}${c.reset}${rule} ${toolBadge}`);
    console.log(`    ${d.message}`);
  }
}

function printRawCommandOutput(stdout: string, stderr: string): void {
  const combined = [stdout.trim(), stderr.trim()].filter(Boolean).join('\n');
  if (!combined) return;
  console.log(c.dim + combined + c.reset);
}

// ============================================================================
// FORMAT COMMAND
// ============================================================================
async function runFormat(checkOnly = false): Promise<boolean> {
  printSectionHeader('Format', checkOnly ? 'audit check only' : 'formatting in place');
  const results: StepResult[] = [];
  const ktlintPath = await ensureKtlint();

  // 1. Prettier (Web / TypeScript / Astro / Markdown / Configs)
  process.stdout.write(`  ${sym.arrow} Formatting web, configs & docs (Prettier)... `);
  const prettierPatterns = [
    'worker/**/*.{ts,json,jsonc}',
    'site/src/**/*.{ts,astro}',
    'package.json',
    '.prettierrc.json',
    '.oxlintrc.json',
  ];
  const prettierArgs = checkOnly
    ? ['prettier', '--check', ...prettierPatterns]
    : ['prettier', '--write', ...prettierPatterns];
  const pRes = await execCommand('bunx', prettierArgs);
  const pPassed = pRes.code === 0;
  const pSummary = checkOnly
    ? pPassed
      ? 'All files match formatting rules'
      : 'Files need formatting'
    : 'Formatted web, docs & config files';
  console.log(pPassed ? sym.ok : sym.err);
  if (!pPassed && checkOnly) {
    console.log(c.dim + pRes.stdout.trim().split('\n').slice(0, 10).join('\n') + c.reset);
  }
  results.push({
    title: 'Web & Configs (Prettier)',
    tool: 'prettier',
    success: pPassed,
    durationMs: pRes.durationMs,
    summary: pSummary,
  });

  // 2. Ruff (Python)
  process.stdout.write(`  ${sym.arrow} Formatting Python sources (Ruff)... `);
  const ruff = resolveRuffCommand();
  const ruffArgs = checkOnly
    ? ['format', '--check', 'tools/apk-lab', '.github/scripts']
    : ['format', 'tools/apk-lab', '.github/scripts'];
  const rRes = await execCommand(ruff.cmd, [...ruff.baseArgs, ...ruffArgs]);
  const rPassed = rRes.code === 0;
  const rSummary = checkOnly
    ? rPassed
      ? 'All Python files formatted'
      : 'Python files need formatting'
    : 'Formatted Python sources';
  console.log(rPassed ? sym.ok : sym.err);
  if (!rPassed) {
    printRawCommandOutput(rRes.stdout, rRes.stderr);
  }
  results.push({
    title: 'Python (Ruff format)',
    tool: 'ruff',
    success: rPassed,
    durationMs: rRes.durationMs,
    summary: rSummary,
  });

  // 3. ktlint (Kotlin)
  process.stdout.write(`  ${sym.arrow} Formatting Kotlin sources (ktlint)... `);
  const ktlintArgs = checkOnly
    ? ['patches/src/main/kotlin/**/*.kt']
    : ['-F', 'patches/src/main/kotlin/**/*.kt'];
  const kRes = await execCommand(ktlintPath, ktlintArgs);
  const kPassed = kRes.code === 0;
  const kSummary = checkOnly
    ? kPassed
      ? 'All Kotlin files formatted'
      : 'Kotlin style deviations detected'
    : 'Formatted Kotlin sources';
  console.log(kPassed ? sym.ok : sym.err);
  if (!kPassed && checkOnly) {
    console.log(c.dim + (kRes.stderr || kRes.stdout).trim().split('\n').slice(0, 10).join('\n') + c.reset);
  }
  results.push({
    title: 'Kotlin (ktlint)',
    tool: 'ktlint',
    success: kPassed,
    durationMs: kRes.durationMs,
    summary: kSummary,
  });

  return printResultsSummary('Format', results);
}

// ============================================================================
// LINT COMMAND
// ============================================================================
async function runLint(autoFix = false): Promise<boolean> {
  printSectionHeader('Lint', autoFix ? 'auto-fix enabled' : 'auditing code quality');
  const results: StepResult[] = [];
  const allDiagnostics: Diagnostic[] = [];
  const ktlintPath = await ensureKtlint();

  // 1. Oxlint (Web / TypeScript / Astro)
  process.stdout.write(`  ${sym.arrow} Linting TypeScript, Worker & Site (Oxlint)... `);
  const oxlintArgs = ['oxlint', '--format', 'json'];
  if (autoFix) oxlintArgs.push('--fix');
  oxlintArgs.push('worker', 'site');

  const oxRes = await execCommand('bunx', oxlintArgs);
  const oxDiags: Diagnostic[] = [];
  try {
    const parsed = JSON.parse(oxRes.stdout);
    const diags = Array.isArray(parsed) ? parsed : parsed.diagnostics || [];
    for (const d of diags) {
      const loc = d.labels?.[0]?.span;
      oxDiags.push({
        tool: 'oxlint',
        file: d.filename || 'unknown',
        line: loc?.line,
        col: loc?.column,
        message: d.message,
        rule: d.code,
        severity: d.severity === 'error' ? 'error' : 'warning',
      });
    }
  } catch {
    printRawCommandOutput(oxRes.stdout, oxRes.stderr);
  }
  const oxErrors = oxDiags.filter((d) => d.severity === 'error').length;
  const oxWarns = oxDiags.filter((d) => d.severity === 'warning').length;
  const oxPassed = oxRes.code === 0 && oxErrors === 0;
  allDiagnostics.push(...oxDiags);

  console.log(oxPassed ? sym.ok : sym.err);
  results.push({
    title: 'TypeScript & Web (Oxlint)',
    tool: 'oxlint',
    success: oxPassed,
    durationMs: oxRes.durationMs,
    summary: `${oxDiags.length === 0 ? 'No issues' : `${oxErrors} errors, ${oxWarns} warnings`}`,
    diagnostics: oxDiags,
  });

  // 2. Ruff (Python)
  process.stdout.write(`  ${sym.arrow} Linting Python sources (Ruff)... `);
  const ruff = resolveRuffCommand();
  const ruffArgs = ['check', '--output-format', 'json'];
  if (autoFix) ruffArgs.push('--fix');
  ruffArgs.push('tools/apk-lab', '.github/scripts');

  const rRes = await execCommand(ruff.cmd, [...ruff.baseArgs, ...ruffArgs]);
  const rDiags: Diagnostic[] = [];
  try {
    const items = JSON.parse(rRes.stdout);
    if (Array.isArray(items)) {
      for (const item of items) {
        rDiags.push({
          tool: 'ruff',
          file: item.filename,
          line: item.location?.row,
          col: item.location?.column,
          message: item.message,
          rule: item.code,
          severity: 'error',
        });
      }
    }
  } catch {
    printRawCommandOutput(rRes.stdout, rRes.stderr);
  }
  const rPassed = rRes.code === 0 && rDiags.length === 0;
  allDiagnostics.push(...rDiags);

  console.log(rPassed ? sym.ok : sym.err);
  results.push({
    title: 'Python (Ruff check)',
    tool: 'ruff',
    success: rPassed,
    durationMs: rRes.durationMs,
    summary: `${rDiags.length === 0 ? 'No issues' : `${rDiags.length} issues found`}`,
    diagnostics: rDiags,
  });

  // 3. ktlint (Kotlin)
  process.stdout.write(`  ${sym.arrow} Linting Kotlin sources (ktlint)... `);
  const ktArgs = autoFix
    ? ['-F', 'patches/src/main/kotlin/**/*.kt']
    : ['patches/src/main/kotlin/**/*.kt', '--reporter=json'];

  const kRes = await execCommand(ktlintPath, ktArgs);
  const kDiags: Diagnostic[] = [];
  try {
    const files = JSON.parse(kRes.stdout);
    if (Array.isArray(files)) {
      for (const f of files) {
        for (const err of f.errors || []) {
          kDiags.push({
            tool: 'ktlint',
            file: f.file,
            line: err.line,
            col: err.column,
            message: err.message,
            rule: err.rule,
            severity: 'error',
          });
        }
      }
    }
  } catch {
    printRawCommandOutput(kRes.stdout, kRes.stderr);
  }
  const kPassed = kRes.code === 0 && kDiags.length === 0;
  allDiagnostics.push(...kDiags);

  console.log(kPassed ? sym.ok : sym.err);
  results.push({
    title: 'Kotlin (ktlint)',
    tool: 'ktlint',
    success: kPassed,
    durationMs: kRes.durationMs,
    summary: `${kDiags.length === 0 ? 'No issues' : `${kDiags.length} issues found`}`,
    diagnostics: kDiags,
  });

  renderDiagnostics(allDiagnostics);
  return printResultsSummary('Lint', results);
}

// ============================================================================
// TYPECHECK COMMAND
// ============================================================================
async function runTypecheck(): Promise<boolean> {
  printSectionHeader('Typecheck', 'TypeScript, Astro, Python, and Kotlin/Java');
  const results: StepResult[] = [];

  // 1. Worker TypeScript (tsc)
  process.stdout.write(`  ${sym.arrow} Checking Cloudflare Worker types (tsc)... `);
  const wRes = await execCommand('bun', ['run', '--cwd', 'worker', 'check']);
  const wPassed = wRes.code === 0;
  console.log(wPassed ? sym.ok : sym.err);
  if (!wPassed) {
    printRawCommandOutput(wRes.stdout, wRes.stderr);
  }
  results.push({
    title: 'Cloudflare Worker (tsc)',
    tool: 'tsc',
    success: wPassed,
    durationMs: wRes.durationMs,
    summary: wPassed ? 'Clean, no type diagnostics' : 'TypeScript errors encountered',
  });

  // 2. Astro Site (astro check)
  process.stdout.write(`  ${sym.arrow} Checking Astro Site types (astro check)... `);
  const sRes = await execCommand('bun', ['run', '--cwd', 'site', 'check']);
  const sPassed = sRes.code === 0;
  console.log(sPassed ? sym.ok : sym.err);
  if (!sPassed) {
    printRawCommandOutput(sRes.stdout, sRes.stderr);
  }
  results.push({
    title: 'Astro Site (astro check)',
    tool: 'astro check',
    success: sPassed,
    durationMs: sRes.durationMs,
    summary: sPassed ? '0 errors, 0 warnings' : 'Astro check diagnostics reported',
  });

  // 3. Python (mypy on tools/apk-lab)
  process.stdout.write(`  ${sym.arrow} Checking Python types (mypy on apk-lab)... `);
  const pyRes = await execCommand('uv', [
    'run',
    '--project',
    'tools/apk-lab',
    '--with',
    'mypy',
    'mypy',
    'tools/apk-lab/apk_lab',
    '--ignore-missing-imports',
  ]);
  const pyPassed = pyRes.code === 0;
  console.log(pyPassed ? sym.ok : sym.err);
  if (!pyPassed) {
    printRawCommandOutput(pyRes.stdout, pyRes.stderr);
  }
  results.push({
    title: 'APK Lab Python (mypy)',
    tool: 'mypy',
    success: pyPassed,
    durationMs: pyRes.durationMs,
    summary: pyPassed ? '15 source files verified' : 'Mypy type errors found',
  });

  // 4. Morphe Patches & Extensions (Gradle compileKotlin + compileReleaseJavaWithJavac)
  process.stdout.write(`  ${sym.arrow} Compiling Morphe Patches & Extension (Gradle)... `);
  const gRes = await execCommand('./gradlew', [
    ':patches:compileKotlin',
    ':extensions:extension:compileReleaseJavaWithJavac',
    '--console=plain',
  ]);
  const gPassed = gRes.code === 0;
  console.log(gPassed ? sym.ok : sym.err);
  if (!gPassed) {
    printRawCommandOutput(gRes.stdout, gRes.stderr);
  }
  results.push({
    title: 'Morphe Patches (Gradle)',
    tool: 'gradle',
    success: gPassed,
    durationMs: gRes.durationMs,
    summary: gPassed ? 'Kotlin and Java compilation successful' : 'Compilation failed',
  });

  return printResultsSummary('Typecheck', results);
}

// ============================================================================
// TEST COMMAND
// ============================================================================
async function runTest(): Promise<boolean> {
  printSectionHeader('Test', 'auditing all automated test suites');
  const results: StepResult[] = [];

  // 1. APK Lab Python test suite (pytest)
  process.stdout.write(`  ${sym.arrow} Running APK Lab tests (pytest)... `);
  const pyRes = await execCommand('uv', ['run', '--project', 'tools/apk-lab', 'pytest', '-q']);
  const pyPassed = pyRes.code === 0;
  const pySummaryMatch = pyRes.stdout.match(/([0-9]+ passed[^\n]*)/);
  const pySummary = pySummaryMatch ? pySummaryMatch[1].trim() : pyPassed ? 'All tests passed' : 'Tests failed';
  console.log(pyPassed ? sym.ok : sym.err);
  if (!pyPassed) {
    printRawCommandOutput(pyRes.stdout, pyRes.stderr);
  }
  results.push({
    title: 'APK Lab Suite (pytest)',
    tool: 'pytest',
    success: pyPassed,
    durationMs: pyRes.durationMs,
    summary: pySummary,
  });

  // 2. Cloudflare Worker test suite (Vitest)
  process.stdout.write(`  ${sym.arrow} Running Worker tests (Vitest)... `);
  const wRes = await execCommand('bun', ['run', '--cwd', 'worker', 'test']);
  const wPassed = wRes.code === 0;
  const wSummaryMatch = wRes.stdout.match(/Tests\s+([0-9]+ passed[^\n]*)/);
  const wSummary = wSummaryMatch ? wSummaryMatch[1].trim() : wPassed ? 'All tests passed' : 'Tests failed';
  console.log(wPassed ? sym.ok : sym.err);
  if (!wPassed) {
    printRawCommandOutput(wRes.stdout, wRes.stderr);
  }
  results.push({
    title: 'Worker Suite (Vitest)',
    tool: 'vitest',
    success: wPassed,
    durationMs: wRes.durationMs,
    summary: wSummary,
  });

  // 3. Astro Site test suite (Bun test)
  process.stdout.write(`  ${sym.arrow} Running Site tests (Bun test)... `);
  const sRes = await execCommand('bun', ['run', '--cwd', 'site', 'test']);
  const sPassed = sRes.code === 0;
  const sSummaryMatch = sRes.stdout.match(/([0-9]+ pass[^\n]*)/);
  const sSummary = sSummaryMatch ? sSummaryMatch[1].trim() : sPassed ? 'All tests passed' : 'Tests failed';
  console.log(sPassed ? sym.ok : sym.err);
  if (!sPassed) {
    printRawCommandOutput(sRes.stdout, sRes.stderr);
  }
  results.push({
    title: 'Site Suite (Bun test)',
    tool: 'bun test',
    success: sPassed,
    durationMs: sRes.durationMs,
    summary: sSummary,
  });

  // 4. Target Application Icons Check
  process.stdout.write(`  ${sym.arrow} Verifying application icon assets... `);
  const iconRes = await execCommand('python3', ['.github/scripts/sync_app_icons.py', '--check-only']);
  const iconPassed = iconRes.code === 0;
  console.log(iconPassed ? sym.ok : sym.err);
  if (!iconPassed) {
    printRawCommandOutput(iconRes.stdout, iconRes.stderr);
  }
  results.push({
    title: 'App Icons Verification',
    tool: 'python3',
    success: iconPassed,
    durationMs: iconRes.durationMs,
    summary: iconPassed ? iconRes.stdout.trim() : 'Missing application icons',
  });

  return printResultsSummary('Test', results);
}

// ============================================================================
// CLI DISPATCHER
// ============================================================================
async function main() {
  const args = process.argv.slice(2);
  const cmd = args[0] || 'all';

  let success = true;
  switch (cmd) {
    case 'format': {
      const checkOnly = args.includes('--check');
      success = await runFormat(checkOnly);
      break;
    }
    case 'format:check': {
      success = await runFormat(true);
      break;
    }
    case 'lint': {
      const autoFix = args.includes('--fix');
      success = await runLint(autoFix);
      break;
    }
    case 'lint:fix': {
      success = await runLint(true);
      break;
    }
    case 'typecheck': {
      success = await runTypecheck();
      break;
    }
    case 'test': {
      success = await runTest();
      break;
    }
    case 'all': {
      const fOk = await runFormat(true);
      const lOk = await runLint(false);
      const tOk = await runTypecheck();
      const testOk = await runTest();
      success = fOk && lOk && tOk && testOk;
      break;
    }
    default: {
      console.error(`Unknown command: ${cmd}`);
      console.error('Usage: bun scripts/audit.ts <format|format:check|lint|lint:fix|typecheck|test|all>');
      process.exit(1);
    }
  }

  process.exit(success ? 0 : 1);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
