<script lang="ts">
	import { page } from '$app/state';
	import Tabs from '$lib/Tabs.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);

	const base = $derived(`/workspaces/${workspace}/apps/${app}`);

	const tabs = $derived([
		// The Collections tab is the app root, but also owns the
		// collection-detail + records sub-routes.
		{
			label: 'Collections',
			href: base,
			exact: true,
			matchPrefixes: [`${base}/collections`]
		},
		{ label: 'Policies', href: `${base}/policies` },
		{ label: 'Hooks', href: `${base}/hooks` },
		{ label: 'Files', href: `${base}/files` },
		{ label: 'Audit', href: `${base}/audit` }
	]);

	let { children } = $props();
</script>

<Tabs {tabs} />

{@render children()}
