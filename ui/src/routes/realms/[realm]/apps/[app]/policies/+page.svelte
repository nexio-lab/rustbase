<script lang="ts">
	import { page } from '$app/state';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';

	const realm = $derived(page.params.realm);
	const app = $derived(page.params.app);
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: app, href: `/realms/${realm}/apps/${app}` },
		{ label: 'Policies' }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/realms/{realm}/apps/{app}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Collections
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Policies</span>
	<a
		href="/realms/{realm}/apps/{app}/hooks"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Hooks
	</a>
	<a
		href="/realms/{realm}/apps/{app}/files"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Files
	</a>
</div>

<PoliciesView
	apiBase={`/api/realms/${realm}/apps/${app}/policies`}
	scopeLabel={`app ${realm}/${app}`}
/>
