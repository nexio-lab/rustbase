<script lang="ts">
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
	import PoliciesView from '$lib/PoliciesView.svelte';
</script>

<Breadcrumbs items={[{ label: 'System', href: '/system' }, { label: 'Policies' }]} />

<PoliciesView apiBase="/api/system/policies" scopeLabel="system (master scope)" />
