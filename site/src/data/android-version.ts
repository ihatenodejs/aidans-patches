/**
 * Formats an Android SDK integer level into its consumer-facing Android OS version.
 * e.g. 32 -> "Android 12L", 31 -> "Android 12", 29 -> "Android 10", 23 -> "Android 6.0"
 */
export function formatAndroidVersion(
  minSdk?: number | null,
  suffix = '+',
): string {
  if (!minSdk || typeof minSdk !== 'number' || minSdk < 1) {
    return 'Varies';
  }

  const map: Record<number, string> = {
    36: 'Android 16',
    35: 'Android 15',
    34: 'Android 14',
    33: 'Android 13',
    32: 'Android 12L',
    31: 'Android 12',
    30: 'Android 11',
    29: 'Android 10',
    28: 'Android 9',
    27: 'Android 8.1',
    26: 'Android 8.0',
    25: 'Android 7.1',
    24: 'Android 7.0',
    23: 'Android 6.0',
    22: 'Android 5.1',
    21: 'Android 5.0',
    19: 'Android 4.4',
    18: 'Android 4.3',
    17: 'Android 4.2',
    16: 'Android 4.1',
    15: 'Android 4.0.3',
    14: 'Android 4.0',
  };

  const name = map[minSdk] || `Android (API ${minSdk})`;
  return `${name}${suffix}`;
}
