<script lang="ts">
	import AuditView from '$lib/AuditView.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
</script>

<Breadcrumbs items={[{ label: 'System', href: '/system' }, { label: 'Audit' }]} />

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/system/policies"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Policies
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Audit</span>
</div>

<AuditView apiBase="/api/system/audit" scopeLabel="system (master scope)" />
