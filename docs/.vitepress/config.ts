import { defineConfig } from 'vitepress';

// Site is hosted on GitHub Pages at <user>.github.io/rustbase. Override
// `DOCS_BASE` to deploy elsewhere (Cloudflare Pages, Netlify, custom domain).
const base = process.env.DOCS_BASE ?? '/rustbase/';

export default defineConfig({
	base,
	title: 'RustBaas',
	description:
		'A single-binary, single-file Backend-as-a-Service in Rust — realms, apps, collections, auth, hooks, files, realtime.',
	cleanUrls: true,
	lastUpdated: true,
	// localhost links in examples shouldn't fail the build.
	ignoreDeadLinks: 'localhostLinks',
	head: [['link', { rel: 'icon', href: `${base}favicon.svg` }]],

	themeConfig: {
		logo: '/logo.svg',
		nav: [
			{ text: 'Guide', link: '/guide/getting-started' },
			{ text: 'Reference', link: '/reference/rest-api' },
			{ text: 'Concepts', link: '/concepts/mental-model' },
			{
				text: 'Releases',
				link: 'https://github.com/pjonaszik/rustbase/releases'
			}
		],

		sidebar: {
			'/guide/': [
				{
					text: 'Get started',
					items: [
						{ text: 'Introduction', link: '/guide/introduction' },
						{ text: 'Getting started', link: '/guide/getting-started' },
						{ text: 'First app', link: '/guide/first-app' }
					]
				},
				{
					text: 'Features',
					items: [
						{ text: 'Authentication', link: '/guide/authentication' },
						{ text: 'Collections & records', link: '/guide/collections' },
						{ text: 'Hooks (JS/TS)', link: '/guide/hooks' },
						{ text: 'Files', link: '/guide/files' },
						{ text: 'Realtime', link: '/guide/realtime' },
						{ text: 'Policies', link: '/guide/policies' },
						{ text: 'Audit log', link: '/guide/audit' }
					]
				},
				{
					text: 'Operate',
					items: [
						{ text: 'Configuration', link: '/guide/configuration' },
						{ text: 'Deployment', link: '/guide/deployment' },
						{ text: 'Backups (Litestream)', link: '/guide/backups' }
					]
				}
			],
			'/reference/': [
				{
					text: 'Reference',
					items: [
						{ text: 'REST API', link: '/reference/rest-api' },
						{ text: 'Filter syntax', link: '/reference/filters' },
						{ text: 'Error codes', link: '/reference/errors' },
						{ text: '$app API (hooks)', link: '/reference/hooks-api' }
					]
				}
			],
			'/concepts/': [
				{
					text: 'Concepts',
					items: [
						{ text: 'Mental model', link: '/concepts/mental-model' },
						{
							text: 'Hierarchical policies',
							link: '/concepts/hierarchical-policies'
						},
						{ text: 'Storage layout', link: '/concepts/storage-layout' },
						{ text: 'Architecture', link: '/concepts/architecture' }
					]
				}
			]
		},

		socialLinks: [
			{ icon: 'github', link: 'https://github.com/pjonaszik/rustbase' }
		],

		footer: {
			message: 'Released under the MIT OR Apache-2.0 license.',
			copyright: ''
		},

		search: {
			provider: 'local'
		},

		editLink: {
			pattern:
				'https://github.com/pjonaszik/rustbase/edit/main/docs/:path',
			text: 'Edit this page on GitHub'
		}
	}
});
