import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

let cachedAvailableIcons: Set<string> | null = null;

function getAvailableIcons(): Set<string> {
  if (cachedAvailableIcons) return cachedAvailableIcons;

  const currentDir = path.dirname(fileURLToPath(import.meta.url));
  const iconsDir = path.resolve(currentDir, '../../public/icons');

  const set = new Set<string>();
  if (fs.existsSync(iconsDir)) {
    const files = fs.readdirSync(iconsDir);
    for (const file of files) {
      if (file.endsWith('.png')) {
        set.add(file.slice(0, -4));
      }
    }
  }

  cachedAvailableIcons = set;
  return set;
}

/**
 * Returns the public URL for an application icon if it exists in public/icons/<packageName>.png.
 * Returns null if the package is empty or the icon file is not present.
 */
export function getAppIconUrl(packageName: string): string | null {
  if (!packageName) return null;

  const available = getAvailableIcons();
  if (available.has(packageName)) {
    return `/icons/${encodeURIComponent(packageName)}.png`;
  }

  return null;
}
