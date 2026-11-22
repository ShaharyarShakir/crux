// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	site: 'https://shaharyarshakir.github.io',
	base: '/crux',
	integrations: [
		starlight({
			title: 'Crux',
			description: 'A source-first, deterministic C/C++ package manager in Rust.',
			customCss: ['./src/styles/theme.css'],
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/ShaharyarShakir/crux' }
			],
			sidebar: [
				{
					label: 'Guides',
					items: [
						{ label: 'Getting Started', slug: 'guides/getting-started' },
						{ label: 'Installation', slug: 'guides/installation' },
						{ label: 'Direct Sources', slug: 'guides/direct-sources' },
						{ label: 'Terminal Output', slug: 'guides/terminal-output' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'CLI Reference', slug: 'reference/cli' },
					],
				},
				{
					label: 'Registry',
					items: [
						{ label: 'Seed Packages', slug: 'registry/packages' },
					],
				},
				{
					label: 'Architecture & Security',
					items: [
						{ label: 'Architecture', slug: 'design/architecture' },
						{ label: 'Security Model', slug: 'design/security' },
					],
				},
				{
					label: 'Project',
					items: [
						{ label: 'Roadmap', slug: 'project/roadmap' },
						{ label: 'Changelog', slug: 'project/changelog' },
					],
				},
			],
		}),
	],
});
