<script lang="ts">
	import { page } from '$app/state';
	import Tabs from '$lib/Tabs.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);
	const coll = $derived(page.params.coll);
	const base = $derived(`/workspaces/${workspace}/apps/${app}/collections/${coll}`);

	const tabs = $derived([
		{ label: 'Schema', href: base, exact: true },
		{ label: 'Records', href: `${base}/records` }
	]);

	let { children } = $props();
</script>

<Tabs {tabs} />

{@render children()}
