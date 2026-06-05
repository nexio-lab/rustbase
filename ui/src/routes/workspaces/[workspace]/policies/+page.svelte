<script lang="ts">
	import { page } from '$app/state';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';

	const workspace = $derived(page.params.workspace);
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: 'Policies' }
	]}
/>

<PoliciesView apiBase={`/api/workspaces/${workspace}/policies`} scopeLabel={`workspace ${workspace}`} />
