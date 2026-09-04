<script lang="ts">
	import { page } from '$app/state';
	import AuditView from '$lib/AuditView.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace!);
	const app = $derived(page.params.app!);
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: 'Audit' }
	]}
/>

<AuditView
	apiBase={`/api/workspaces/${workspace}/apps/${app}/audit`}
	scopeLabel={`app ${workspace}/${app}`}
/>
