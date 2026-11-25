/**
 * Offline semantic-release config for `--dry-run` only.
 *
 * Keep this limited to the plugins that resolve a version and release notes.
 * Any prepare/publish/success plugin (git, github, gradle, exec, changelog,
 * backmerge) would perform side effects and must never be added here.
 *
 * Usage: ./node_modules/.bin/semantic-release --dry-run \
 *   --extends ./release.dryrun.config.mjs
 *
 * The release rules and notes preset mirror `.releaserc`; keep them in sync.
 */
export default {
  branches: [
    'release/main',
    { name: 'release/dev', prerelease: 'dev', channel: 'dev' },
  ],
  plugins: [
    [
      '@semantic-release/commit-analyzer',
      {
        releaseRules: [
          { header: 'initial commit', release: 'patch' },
          { type: 'fix', release: 'patch' },
          { type: 'feat', release: 'minor' },
          { type: 'bump', release: 'patch' },
          { type: 'perf', release: 'patch' },
          { type: 'build', scope: 'Needs bump', release: 'patch' },
          { scope: 'site', release: false },
          { scope: 'worker', release: false },
          { scope: 'apk-lab', release: false },
          { scope: 'audit', release: false },
          { scope: 'ci', release: false },
          { scope: 'docs', release: false },
          { scope: 'release', release: false },
          { scope: 'icons', release: false },
          { scope: 'deps', release: false },
          { scope: 'play-metadata', release: false },
        ],
      },
    ],
    [
      '@semantic-release/release-notes-generator',
      {
        preset: 'conventionalcommits',
        presetConfig: {
          types: [
            { type: 'fix', scope: 'site', hidden: true },
            { type: 'fix', scope: 'worker', hidden: true },
            { type: 'fix', scope: 'apk-lab', hidden: true },
            { type: 'fix', scope: 'audit', hidden: true },
            { type: 'fix', scope: 'ci', hidden: true },
            { type: 'fix', scope: 'docs', hidden: true },
            { type: 'fix', scope: 'release', hidden: true },
            { type: 'fix', scope: 'icons', hidden: true },
            { type: 'fix', scope: 'deps', hidden: true },
            { type: 'fix', scope: 'play-metadata', hidden: true },
            { type: 'feat', scope: 'site', hidden: true },
            { type: 'feat', scope: 'worker', hidden: true },
            { type: 'feat', scope: 'apk-lab', hidden: true },
            { type: 'feat', scope: 'audit', hidden: true },
            { type: 'feat', scope: 'ci', hidden: true },
            { type: 'feat', scope: 'docs', hidden: true },
            { type: 'feat', scope: 'release', hidden: true },
            { type: 'feat', scope: 'icons', hidden: true },
            { type: 'feat', scope: 'deps', hidden: true },
            { type: 'feat', scope: 'play-metadata', hidden: true },
            { type: 'bump', scope: 'site', hidden: true },
            { type: 'bump', scope: 'worker', hidden: true },
            { type: 'bump', scope: 'apk-lab', hidden: true },
            { type: 'bump', scope: 'audit', hidden: true },
            { type: 'bump', scope: 'ci', hidden: true },
            { type: 'bump', scope: 'docs', hidden: true },
            { type: 'bump', scope: 'release', hidden: true },
            { type: 'bump', scope: 'icons', hidden: true },
            { type: 'bump', scope: 'deps', hidden: true },
            { type: 'bump', scope: 'play-metadata', hidden: true },
            { type: 'perf', scope: 'site', hidden: true },
            { type: 'perf', scope: 'worker', hidden: true },
            { type: 'perf', scope: 'apk-lab', hidden: true },
            { type: 'perf', scope: 'audit', hidden: true },
            { type: 'perf', scope: 'ci', hidden: true },
            { type: 'perf', scope: 'docs', hidden: true },
            { type: 'perf', scope: 'release', hidden: true },
            { type: 'perf', scope: 'icons', hidden: true },
            { type: 'perf', scope: 'deps', hidden: true },
            { type: 'perf', scope: 'play-metadata', hidden: true },
            { type: 'fix', section: '🐛 Bug Fixes', hidden: false },
            { type: 'feat', section: '✨ New Features', hidden: false },
            { type: 'bump', section: '🚀 Updated App Support', hidden: false },
            { type: 'perf', section: '🔧 Improvements', hidden: false },
            { type: 'build', hidden: true },
          ],
        },
      },
    ],
  ],
};
