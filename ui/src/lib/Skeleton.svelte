<script lang="ts">
	type Props = {
		/** Number of skeleton rows to render. Default: 3. */
		rows?: number;
		/** Tailwind classes applied per row — override for non-default
		 *  shapes (e.g. avatar circles, table cells). */
		rowClass?: string;
		/** Wrapper classes (gap, padding, max width, etc.). */
		class?: string;
	};
	let {
		rows = 3,
		rowClass = 'h-3 rounded bg-slate-200 dark:bg-slate-700',
		class: cls = 'space-y-2'
	}: Props = $props();

	const stripes = Array.from({ length: rows });
</script>

<div class={cls} role="status" aria-busy="true" aria-live="polite">
	{#each stripes as _, i}
		<div class="{rowClass} animate-pulse" style="opacity: {1 - i / (rows + 2)};"></div>
	{/each}
	<span class="sr-only">Loading…</span>
</div>
