import { defineConfig } from 'vitepress';

// Site is hosted on GitHub Pages at <user>.github.io/rustbase. Override
// `DOCS_BASE` to deploy elsewhere (Cloudflare Pages, Netlify, custom domain).
const base = process.env.DOCS_BASE ?? '/rustbase/';

export default defineConfig({
	base,
	title: 'RustBase',
	description:
		'Multi-tenant backend. Single binary. Real isolation. A Backend-as-a-Service in Rust for builders who run multiple small apps under one tenant — SQLite per app, JS hooks without Node.js.',
	cleanUrls: true,
	lastUpdated: true,
	// localhost links in examples shouldn't fail the build.
	ignoreDeadLinks: 'localhostLinks',
	head: [
		['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}favicon.svg` }],
		['link', { rel: 'icon', type: 'image/png', sizes: '32x32', href: `${base}favicon-32.png` }],
		['link', { rel: 'icon', type: 'image/png', sizes: '192x192', href: `${base}favicon-192.png` }],
		['link', { rel: 'apple-touch-icon', href: `${base}favicon-192.png` }],
		['meta', { property: 'og:image', content: `${base}social-preview.png` }],
		['meta', { name: 'twitter:image', content: `${base}social-preview.png` }],
		['meta', { name: 'twitter:card', content: 'summary_large_image' }]
	],

	themeConfig: {
		logo: '/logo.svg',
		nav: [
			{ text: 'Guide', link: '/guide/getting-started' },
			{ text: 'Cookbook', link: '/cookbook/' },
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
						{ text: 'First app', link: '/guide/first-app' },
						{ text: 'Compare vs PocketBase / Supabase / Appwrite', link: '/guide/comparison' }
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
						{ text: 'Backups (Litestream)', link: '/guide/backups' },
						{ text: 'Observability', link: '/guide/observability' }
					]
				}
			],
			'/cookbook/': [
				{
					text: 'Cookbook',
					items: [
						{ text: 'Overview', link: '/cookbook/' },
						{ text: 'End-to-end sign-up flow', link: '/cookbook/auth-flow' },
						{ text: 'Filter and paginate records', link: '/cookbook/filter-paginate' },
						{ text: 'Upload and attach a file', link: '/cookbook/files' },
						{ text: 'Realtime with server-side filter', link: '/cookbook/realtime' },
						{ text: 'Add a custom HTTP route', link: '/cookbook/custom-route' },
						{ text: 'OAuth login (Google)', link: '/cookbook/oauth-google' }
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
						{ text: 'Positioning', link: '/concepts/positioning' },
						{ text: 'Mental model', link: '/concepts/mental-model' },
						{
							text: 'Hierarchical policies',
							link: '/concepts/hierarchical-policies'
						},
						{ text: 'Storage layout', link: '/concepts/storage-layout' },
						{ text: 'Write amplification', link: '/concepts/write-amplification' },
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
