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
        'Trading chart generator and MCP server in Rust. 33 indicators, Yahoo Finance + Binance, Claude.ai integration.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/5queezer/chartgen',
        },
      ],
      sidebar: [
        { label: 'Quick Start', slug: 'guides/quickstart' },
        {
          label: 'Reference',
          items: [
            { label: 'CLI', slug: 'reference/cli' },
            { label: 'Indicators', slug: 'reference/indicators' },
            { label: 'MCP Integration', slug: 'reference/mcp' },
          ],
        },
      ],
    }),
  ],
});
