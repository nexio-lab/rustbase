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

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/workspaces/{workspace}/apps/{app}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Collections
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Policies</span>
	<a
		href="/workspaces/{workspace}/apps/{app}/hooks"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Hooks
	</a>
	<a
		href="/workspaces/{workspace}/apps/{app}/files"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Files
	</a>
	<a
		href="/workspaces/{workspace}/apps/{app}/audit"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Audit
	</a>
</div>

<PoliciesView
	apiBase={`/api/workspaces/${workspace}/apps/${app}/policies`}
	scopeLabel={`app ${workspace}/${app}`}
/>
