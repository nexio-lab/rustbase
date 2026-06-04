<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import { auth } from '$lib/auth.svelte';

	let { children } = $props();

	const PUBLIC_ROUTES = new Set(['/login', '/setup']);

	// Public docs site URL. Override at build time with VITE_DOCS_URL
	// for self-hosted docs; defaults to the canonical site.
	const DOCS_URL = import.meta.env.VITE_DOCS_URL ?? 'https://pjonaszik.github.io/rustbase/';

	// Route guard: anything outside the public list requires a session.
	// Runs on every navigation thanks to runes; redirects don't loop
	// because the redirect target IS one of the public routes.
	//
	// `page.url.pathname` includes the configured base (e.g. `/_/login`
	// when the dashboard is embedded under `/_/`). Strip it so the
	// PUBLIC_ROUTES set stays base-agnostic, and prefix `base` back
	// onto every `goto()` target so the navigation doesn't accidentally
	// escape the dashboard mount.
	$effect(() => {
		const raw = page.url.pathname;
		const path = base && raw.startsWith(base) ? raw.slice(base.length) || '/' : raw;
		if (!auth.isAuthenticated && !PUBLIC_ROUTES.has(path)) {
			goto(`${base}/login`, { replaceState: true });
		}
	});

	async function logout() {
		auth.clear();
		await goto(`${base}/login`, { replaceState: true });
	}
</script>

<svelte:head>
	<title>RustBase</title>
	<link rel="icon" href={favicon} />
</svelte:head>

{#if auth.isAuthenticated}
	<header class="border-b border-slate-200 bg-white">
		<div class="mx-auto flex max-w-6xl items-center justify-between px-6 py-3">
			<a href="/" class="flex items-center gap-2 text-slate-900">
				<span class="inline-block h-2.5 w-2.5 rounded-sm bg-orange-500"></span>
				<span class="text-sm font-semibold tracking-tight">RustBase</span>
			</a>
			<nav class="flex items-center gap-1 text-sm">
				<a class="nav-link" href="/realms">Realms</a>
				{#if auth.isMaster}
					<a class="nav-link" href="/system">System</a>
				{/if}
				<a class="nav-link" href={DOCS_URL} target="_blank" rel="noopener noreferrer">
					Docs ↗
				</a>
				{#if auth.admin}
					<span class="ml-3 text-xs text-slate-500">
						{auth.admin.username}
					</span>
				{/if}
				<button onclick={logout} class="nav-link ml-2 cursor-pointer">Sign out</button>
			</nav>
		</div>
	</header>
{/if}

<main class="mx-auto max-w-6xl px-6 py-8">
	{@render children()}
</main>
