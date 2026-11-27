import fs from 'node:fs';
import path from 'node:path';
import { resolveRepositoryRoot } from './repo-root';
import { getPatchSourceMap } from './source-links';

export interface PatchCodeEntry {
  patchName: string;
  fileName: string;
  relativePath: string;
  code: string;
}

let cachedCodeMap: Record<string, PatchCodeEntry> | null = null;

export function getPatchCodeMap(): Record<string, PatchCodeEntry> {
  if (cachedCodeMap) return cachedCodeMap;

  const repoRoot = resolveRepositoryRoot();
  const sourceMap = getPatchSourceMap();

  const result: Record<string, PatchCodeEntry> = {};

  for (const [key, relPath] of Object.entries(sourceMap)) {
    const fullPath = path.resolve(repoRoot, relPath);
    if (fs.existsSync(fullPath)) {
      const raw = fs.readFileSync(fullPath, 'utf-8');
      const fileName = path.basename(relPath);
      const patchName = key.includes(':') ? key.split(':')[1] : key;
      const entry: PatchCodeEntry = {
        patchName,
        fileName,
        relativePath: relPath,
        code: raw.trim(),
      };
      result[key] = entry;
      if (!result[patchName]) {
        result[patchName] = entry;
      }
    }
  }

  cachedCodeMap = result;
  return cachedCodeMap;
}
