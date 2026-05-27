<script lang="ts">
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';
</script>

<Breadcrumbs items={[{ label: 'System', href: '/system' }, { label: 'Policies' }]} />

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Policies</span>
	<a
		href="/system/audit"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Audit
	</a>
</div>

<PoliciesView apiBase="/api/system/policies" scopeLabel="system (master scope)" />
