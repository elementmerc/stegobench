// Stegobench documentation site.
//
// This site is the HARNESS only. The corpus it produces, Pentimento, has readers of its own
// (people downloading a dataset rather than running a benchmark) and its own site in its own
// REPOSITORY. One site serving both meant a person after the licence terms had to read past an
// explanation of pairing discipline to reach them, and a citation that pointed at a benchmark
// harness rather than at the dataset being cited.
//
// Built under the same standing rule as the rest of the house: a project with readers who are
// not its author ships built documentation. npm lives in this directory and nowhere else;
// nothing here reaches the Python generators or the Rust crates.
//
// `ignoreDeadLinks` is NOT set, so the build fails on any broken internal link. It used to carry
// an exception for `/pentimento/`, a sibling site this build could not see. That site now lives in
// its own repository and is linked as an external URL, which VitePress does not try to resolve, so
// the exception is gone and nothing here is unchecked.

const BASE = process.env.DOCS_BASE || '/stegobench/'
// og:image has to be absolute: a social card served from a relative path is fetched by a crawler
// with no page context, so it simply does not resolve and the preview silently falls back to
// nothing.
const SITE = process.env.DOCS_SITE || 'https://elementmerc.github.io'

export default {
  title: 'Stegobench',
  description:
    'A reproducible benchmark for image steganalysis: build a labelled corpus, run '
    + 'detectors over identical bytes, and report numbers somebody else can check.',
  lang: 'en-GB',
  base: BASE,
  cleanUrls: true,
  lastUpdated: true,

  // The sibling corpus site, which this build does not produce. See above.
  // Markdown under docs/ that is not part of the site. `private/` is excluded from the
  // repository already; the pattern stays as a second guard, because this site is public and a
  // build that renders whatever it finds is one careless `mv` away from publishing a note.
  srcExclude: ['design/**', 'private/**', 'brand/**', 'node_modules/**'],

  head: [
    ['link', { rel: 'icon', href: `${BASE}favicon.svg`, type: 'image/svg+xml' }],
    // The Apple neutrals ground, which tints the browser chrome on mobile. A value that does not
    // match the page reads as a rendering fault rather than a choice.
    ['meta', { name: 'theme-color', content: '#F5F5F7' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Stegobench' }],
    ['meta', {
      property: 'og:description',
      content: 'A reproducible benchmark for image steganalysis, and Pentimento, '
        + 'a corpus with its licences attached.',
    }],
    ['meta', { property: 'og:image', content: `${SITE}${BASE}social-card.png` }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],

  themeConfig: {
    siteTitle: 'Stegobench',

    nav: [
      { text: 'Guide', link: '/guide/what-it-is', activeMatch: '/guide/' },
      { text: 'Pentimento, the corpus', link: 'https://github.com/elementmerc/pentimento' },
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
          text: 'The discipline',
          items: [
            { text: 'Pairing, and what breaks it', link: '/guide/pairing' },
            { text: 'Scores, not verdicts', link: '/guide/scores' },
          ],
        },
        {
          text: 'Going further',
          items: [
            { text: 'Limitations', link: '/guide/limits' },
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
