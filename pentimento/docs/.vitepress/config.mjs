// Pentimento documentation site.
//
// Separate from Stegobench's site on purpose. Stegobench is a tool you run; Pentimento is a
// corpus you download. The two have different readers, and one site serving both meant a person
// looking for the licence terms had to read past an explanation of pairing discipline to reach
// them.
//
// Everything under this directory is self-contained, so the whole `pentimento/` folder moves to
// a repository of its own without touching anything else.
//
// `ignoreDeadLinks` is deliberately NOT set, so the build fails on a broken internal link.

const BASE = process.env.DOCS_BASE || '/pentimento/'
const SITE = process.env.DOCS_SITE || 'https://elementmerc.github.io'

export default {
  title: 'Pentimento',
  description:
    'A steganalysis corpus of 10,000 permissively licensed cover photographs and '
    + '344,348 matched stego pairs, where every image carries its own licence.',
  lang: 'en-GB',
  base: BASE,
  cleanUrls: true,
  lastUpdated: true,

  srcExclude: ['private/**', 'node_modules/**'],

  head: [
    ['link', { rel: 'icon', href: `${BASE}favicon.svg`, type: 'image/svg+xml' }],
    ['meta', { name: 'theme-color', content: '#F5F5F7' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Pentimento' }],
    ['meta', {
      property: 'og:description',
      content: 'A steganalysis corpus with its licences attached.',
    }],
    ['meta', { property: 'og:image', content: `${SITE}${BASE}social-card.png` }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],

  themeConfig: {
    siteTitle: 'Pentimento',

    nav: [
      { text: 'Guide', link: '/guide/what-it-is', activeMatch: '/guide/' },
      { text: 'Get it', link: '/guide/get-it' },
      {
        text: 'Project',
        items: [
          { text: 'Licence (CC BY 4.0)', link: '/guide/licence' },
          { text: 'Stegobench, the harness that built it', link: 'https://github.com/elementmerc/stegobench' },
        ],
      },
    ],

    sidebar: {
      '/': [
        {
          text: 'Start here',
          items: [
            { text: 'What it is', link: '/guide/what-it-is' },
            { text: 'Get it', link: '/guide/get-it' },
            { text: 'What is in it', link: '/guide/whats-in-it' },
          ],
        },
        {
          text: 'Using it',
          items: [
            { text: 'Loading and splitting', link: '/guide/using-it' },
            { text: 'Licence and attribution', link: '/guide/licence' },
            { text: 'Limitations', link: '/guide/limitations' },
          ],
        },
        {
          text: 'Going further',
          items: [
            { text: 'Rebuilding it yourself', link: '/guide/rebuilding' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/elementmerc/stegobench' },
    ],

    outline: { level: [2, 3], label: 'On this page' },

    footer: {
      message: 'The corpus is published under CC BY 4.0. Each file also carries its own licence.',
      copyright: '© 2026 Daniel Iwugo',
    },

    search: { provider: 'local' },

    docFooter: { prev: 'Previous', next: 'Next' },
  },
}
