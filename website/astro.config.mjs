// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://5queezer.github.io',
  base: '/chartgen',
  integrations: [
    starlight({
      title: 'chartgen',
      description:
        'Trading chart generator, MCP server, and live trading engine in Rust. 38 indicators, Yahoo Finance + Binance, Claude.ai integration.',
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
            { label: 'Web frontend', slug: 'guides/web' },
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
      ],
    }),
  ],
});
