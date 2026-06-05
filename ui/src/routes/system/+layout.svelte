<script lang="ts">
	import Tabs from '$lib/Tabs.svelte';

	const tabs = [
		{ label: 'Policies', href: '/system/policies' },
		{ label: 'Audit', href: '/system/audit' }
	];

	let { children } = $props();
</script>

<Tabs {tabs} />

{@render children()}
