<script lang="ts">
	import { page } from '$app/state';
	import AuditView from '$lib/AuditView.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: 'Audit' }
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
		href="/realms/{realm}/policies"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Policies
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Audit</span>
</div>

<AuditView apiBase={`/api/realms/${realm}/audit`} scopeLabel={`realm ${realm}`} />
