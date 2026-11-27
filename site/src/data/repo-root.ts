import fs from 'node:fs';
import path from 'node:path';

/**
 * Resolves the repository root at build time. Astro bundles data modules into
 * generated chunks, so `import.meta.url` cannot locate files outside the site.
 */
export function resolveRepositoryRoot(): string {
  let directory = process.cwd();

  while (true) {
    if (fs.existsSync(path.join(directory, 'patches-list.json'))) {
      return directory;
    }

    const parent = path.dirname(directory);
    if (parent === directory) {
      throw new Error(
        'Unable to locate the repository root containing patches-list.json',
      );
    }
    directory = parent;
  }
}
