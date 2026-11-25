import { describe, expect, test } from 'bun:test';
import {
  VALID_SCOPES,
  domainScopes,
  parseScopes,
  selects,
  type Selection,
} from './audit';

const explicit = (...scopes: string[]): Selection => ({
  domain: 'all',
  scopes: parseScopes(scopes.join(',')),
});

describe('audit scope selection', () => {
  test('parses comma-separated and repeated scopes', () => {
    expect(parseScopes('docs, worker')).toEqual(['docs', 'worker']);
    expect(parseScopes('rust:apk-lab')).toEqual(['rust:apk-lab']);
  });

  test('rejects unknown or empty scopes instead of silently skipping checks', () => {
    expect(() => parseScopes('docs,nope')).toThrow('Unknown scope "nope"');
    expect(() => parseScopes(' , ')).toThrow(
      '--scope requires at least one scope',
    );
  });

  test('explicit scopes do not leak into neighbouring checks', () => {
    const selection = explicit('docs');
    expect(selects(selection, 'docs')).toBe(true);
    expect(selects(selection, 'worker')).toBe(false);
    expect(selects(selection, 'android')).toBe(false);
  });

  test('a metadata-only change runs the metadata consumers', () => {
    const selection = explicit('metadata');
    expect(selects(selection, 'worker', 'metadata')).toBe(true);
    expect(selects(selection, 'icons')).toBe(false);
  });

  test('domain mode keeps local command behaviour', () => {
    const web: Selection = { domain: 'web', scopes: null };
    expect(selects(web, 'docs', 'worker', 'site', 'scripts', 'ci')).toBe(true);
    expect(selects(web, 'android')).toBe(false);

    const rust: Selection = { domain: 'rust', scopes: null };
    expect(selects(rust, 'rust:apk-lab')).toBe(true);
    expect(selects(rust, 'rust:nhl-adblock-native')).toBe(true);
    expect(selects(rust, 'native')).toBe(false);
  });

  test('every Rust project has a valid scope', () => {
    const rustScopes = domainScopes('rust');
    expect(rustScopes).toHaveLength(4);
    for (const scope of rustScopes) {
      expect(VALID_SCOPES).toContain(scope);
    }
  });
});
