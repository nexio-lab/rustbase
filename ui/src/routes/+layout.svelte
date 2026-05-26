<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { auth } from '$lib/auth.svelte';

	let { children } = $props();

	const PUBLIC_ROUTES = new Set(['/login', '/setup']);

	// Route guard: anything outside the public list requires a session.
	// Runs on every navigation thanks to runes; redirects don't loop
	// because the redirect target IS one of the public routes.
	$effect(() => {
		const path = page.url.pathname;
		if (!auth.isAuthenticated && !PUBLIC_ROUTES.has(path)) {
			goto('/login', { replaceState: true });
		}
	});

	async function logout() {
		auth.clear();
		await goto('/login', { replaceState: true });
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
				{#if auth.admin}
					<span class="ml-3 text-xs text-slate-500">
						{auth.admin.email}
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
