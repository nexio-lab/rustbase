<script lang="ts">
	import { page } from '$app/state';
	import AuditView from '$lib/AuditView.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace);
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: 'Audit' }
	]}
/>

<AuditView apiBase={`/api/workspaces/${workspace}/audit`} scopeLabel={`workspace ${workspace}`} />
