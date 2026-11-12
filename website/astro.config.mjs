// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://5queezer.github.io',
  base: '/chartgen',
  integrations: [
    starlight({
      title: 'chartgen',
      logo: {
        light: './src/assets/logo-light.svg',
        dark: './src/assets/logo-dark.svg',
        replacesTitle: true,
      },
      // The g of the wordmark, the same files as the web app's. Starlight prefixes
      // `favicon` with the base; `head` entries are written as they are.
      favicon: '/favicon.svg',
      head: [{ tag: 'link', attrs: { rel: 'icon', href: '/chartgen/favicon.ico', sizes: '32x32' } }],
      description:
        'Chart generator, web app and MCP server for Yahoo Finance and Binance data, written in Rust.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/5queezer/chartgen',
        },
      ],
      sidebar: [
        {
          label: 'Guides',
          items: [
            { label: 'Quick Start', slug: 'guides/quickstart' },
            { label: 'Trading', slug: 'guides/trading' },
            { label: 'Push notifications', slug: 'guides/notifications' },
            { label: 'Scripted indicators', slug: 'guides/scripted-indicators' },
            { label: 'Setups', slug: 'guides/setups' },
            { label: 'Replay over real history', slug: 'guides/replay' },
            { label: 'Web frontend', slug: 'guides/web' },
            { label: 'TradingView parity', slug: 'guides/tradingview-parity' },
            { label: 'Testing', slug: 'guides/testing' },
            { label: 'TypeScript Types for MCP', slug: 'guides/types' },
            { label: 'Deployment', slug: 'guides/deploy' },
            { label: 'Contributing', slug: 'guides/contributing' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'CLI', slug: 'reference/cli' },
            { label: 'Indicators', slug: 'reference/indicators' },
            { label: 'MCP Integration', slug: 'reference/mcp' },
            { label: 'OAuth 2.1 PKCE', slug: 'reference/oauth' },
            { label: 'Persistence', slug: 'reference/persistence' },
          ],
        },
        {
          label: 'Decisions',
          autogenerate: { directory: 'decisions' },
        },
        {
          label: 'Proposals',
          autogenerate: { directory: 'proposals' },
        },
      ],
    }),
  ],
});
