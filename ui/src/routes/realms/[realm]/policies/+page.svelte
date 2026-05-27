<script lang="ts">
	import { page } from '$app/state';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';

	const realm = $derived(page.params.realm);
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: 'Policies' }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/realms/{realm}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Apps
	</a>
	<a
		href="/realms/{realm}/users"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Users
	</a>
	<a
		href="/realms/{realm}/oauth"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		OAuth providers
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Policies</span>
	<a
		href="/realms/{realm}/audit"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Audit
	</a>
</div>

<PoliciesView apiBase={`/api/realms/${realm}/policies`} scopeLabel={`realm ${realm}`} />
