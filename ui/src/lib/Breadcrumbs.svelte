<script lang="ts">
	type Crumb = { label: string; href?: string };
	let { items }: { items: Crumb[] } = $props();
</script>

<nav class="mb-3 flex items-center gap-1.5 text-sm text-slate-500" aria-label="Breadcrumb">
	{#each items as item, i}
		{#if i > 0}
			<span class="text-slate-300">/</span>
		{/if}
		{#if item.href}
			<a href={item.href} class="hover:text-slate-700">{item.label}</a>
		{:else}
			<span class="text-slate-700">{item.label}</span>
		{/if}
	{/each}
</nav>
