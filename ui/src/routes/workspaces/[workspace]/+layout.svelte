<script lang="ts">
	import { page } from '$app/state';
	import Tabs from '$lib/Tabs.svelte';

	const workspace = $derived(page.params.workspace!);
	const base = $derived(`/workspaces/${workspace}`);

	// Don't render workspace-level tabs while the user is inside a
	// specific app — the per-app layout owns that section's chrome.
	const inApp = $derived(page.url.pathname.startsWith(`${base}/apps/`));

	const tabs = $derived([
		// "Apps" is the workspace root; also owns deeper /apps/<app>/...
		// routes so the tab stays lit while drilling into an app.
		{
			label: 'Apps',
			href: base,
			exact: true,
			matchPrefixes: [`${base}/apps`]
		},
		{ label: 'Users', href: `${base}/users` },
		{ label: 'OAuth providers', href: `${base}/oauth` },
		{ label: 'Policies', href: `${base}/policies` },
		{ label: 'Audit', href: `${base}/audit` }
	]);

	let { children } = $props();
</script>

{#if !inApp}
	<Tabs {tabs} />
{/if}

{@render children()}
