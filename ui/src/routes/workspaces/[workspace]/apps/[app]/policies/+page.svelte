<script lang="ts">
	import { page } from '$app/state';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: 'Policies' }
	]}
/>

<PoliciesView
	apiBase={`/api/workspaces/${workspace}/apps/${app}/policies`}
	scopeLabel={`app ${workspace}/${app}`}
/>
