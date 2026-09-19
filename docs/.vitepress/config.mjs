// stegobench documentation site.
//
// Built under the same standing rule as the rest of the house: a project with readers who are
// not its author ships built documentation. npm lives in this directory and nowhere else;
// nothing here reaches the Python generators or the Rust crates.
//
// `ignoreDeadLinks` is deliberately NOT set, so the build fails on a broken internal link. A
// build that warns instead of failing is a build whose output nobody reads.

const BASE = process.env.DOCS_BASE || '/stegobench/'
// og:image has to be absolute: a social card served from a relative path is fetched by a crawler
// with no page context, so it simply does not resolve and the preview silently falls back to
// nothing.
const SITE = process.env.DOCS_SITE || 'https://elementmerc.github.io'

export default {
  title: 'stegobench',
  description:
    'A reproducible benchmark for image steganalysis: build a labelled corpus, run '
    + 'detectors over identical bytes, and report numbers somebody else can check.',
  lang: 'en-GB',
  base: BASE,
  cleanUrls: true,
  lastUpdated: true,

  // Markdown under docs/ that is not part of the site. `private/` is excluded from the
  // repository already; the pattern stays as a second guard, because this site is public and a
  // build that renders whatever it finds is one careless `mv` away from publishing a note.
  srcExclude: ['private/**', 'brand/**', 'node_modules/**'],

  head: [
    ['link', { rel: 'icon', href: `${BASE}favicon.svg`, type: 'image/svg+xml' }],
    // The Apple neutrals ground, which tints the browser chrome on mobile. A value that does not
    // match the page reads as a rendering fault rather than a choice.
    ['meta', { name: 'theme-color', content: '#F5F5F7' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'stegobench' }],
    ['meta', {
      property: 'og:description',
      content: 'A reproducible benchmark for image steganalysis, and Pentimento, '
        + 'a corpus with its licences attached.',
    }],
    ['meta', { property: 'og:image', content: `${SITE}${BASE}social-card.png` }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],

  themeConfig: {
    siteTitle: 'stegobench',

    nav: [
      { text: 'Guide', link: '/guide/what-it-is', activeMatch: '/guide/' },
      { text: 'Corpus', link: '/pentimento', activeMatch: '/pentimento' },
      {
        // NOT a version number. Baseline section 16 keeps unreleased versions out of public
        // artefacts, and naming one in the nav of a published site is a promise about something
        // that does not exist yet.
        text: 'Project',
        items: [
          { text: 'Licence (AGPL-3.0-or-later)', link: 'https://github.com/elementmerc/stegobench/blob/dev/LICENSE' },
          { text: 'Source on GitHub', link: 'https://github.com/elementmerc/stegobench' },
        ],
      },
    ],

    sidebar: {
      '/': [
        {
          text: 'Start here',
          items: [
            { text: 'What it is', link: '/guide/what-it-is' },
            { text: 'Quickstart', link: '/guide/quickstart' },
            { text: 'Build a corpus', link: '/guide/build-a-corpus' },
          ],
        },
        {
          // The two ideas the harness is built around, and the one that voided a whole round of
          // measurements here before it was enforced rather than described.
          text: 'The discipline',
          items: [
            { text: 'Pairing, and what breaks it', link: '/guide/pairing' },
            { text: 'Scores, not verdicts', link: '/guide/scores' },
            { text: 'Tiers, and why they nest', link: '/distribution' },
          ],
        },
        {
          text: 'The corpus',
          items: [
            { text: 'Pentimento', link: '/pentimento' },
            { text: 'Where the covers come from', link: '/cover-source-licensing' },
            { text: 'Read this before quoting a number', link: '/guide/limits' },
          ],
        },
        {
          text: 'Related work',
          items: [
            { text: 'REVEAL, and what this adds', link: '/reveal' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/elementmerc/stegobench' },
    ],

    editLink: {
      pattern: 'https://github.com/elementmerc/stegobench/edit/dev/docs/:path',
      text: 'Suggest a change to this page',
    },

    outline: { level: [2, 3], label: 'On this page' },

    footer: {
      message: 'AGPL-3.0-or-later. The corpus is published separately, under CC BY 4.0.',
      copyright: '© 2026 Daniel Iwugo',
    },

    search: { provider: 'local' },

    docFooter: { prev: 'Previous', next: 'Next' },
  },
}
