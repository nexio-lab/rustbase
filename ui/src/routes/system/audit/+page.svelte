<script lang="ts">
	import AuditView from '$lib/AuditView.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';
</script>

<Breadcrumbs items={[{ label: 'System', href: '/system' }, { label: 'Audit' }]} />

<AuditView apiBase="/api/system/audit" scopeLabel="system (master scope)" />
