import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://datem-dev.github.io',
  base: '/datem',
  integrations: [
    starlight({
      title: 'datem',
      logo: {
        src: './src/assets/logo-mark.svg',
        replacesTitle: false,
      },
      favicon: '/favicon.svg',
      customCss: ['./src/styles/starlight-custom.css'],
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/datem-dev/datem' },
      ],
      sidebar: [
        { label: 'Overview', link: '/docs/' },
        { label: 'API Reference', link: '/docs/api/' },
        { label: 'Stripe Billing Integration', link: '/docs/billing-integration/' },
      ],
    }),
  ],
});
