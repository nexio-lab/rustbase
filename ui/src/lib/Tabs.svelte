<script lang="ts">
	import { page } from '$app/state';

	type Tab = {
		/** Display label. */
		label: string;
		/** Absolute href. The link target and the default active-match
		 *  source. */
		href: string;
		/** When true, the tab matches only on an exact pathname
		 *  equality. Use for the "root" tab of a section so a deep
		 *  child route doesn't also light it up. Default: prefix match. */
		exact?: boolean;
		/** Extra path prefixes that should also count as active for
		 *  this tab. Useful when a "section landing" tab also owns
		 *  deeper detail routes (e.g. the `Collections` tab on the
		 *  app layout owns `/collections/<coll>/…` too). */
		matchPrefixes?: string[];
	};

	let { tabs }: { tabs: Tab[] } = $props();

	function isActive(tab: Tab): boolean {
		const cur = page.url.pathname;
		const extras = tab.matchPrefixes ?? [];
		for (const prefix of extras) {
			if (cur === prefix || cur.startsWith(prefix + '/')) return true;
		}
		if (tab.exact) return cur === tab.href;
		return cur === tab.href || cur.startsWith(tab.href + '/');
	}
</script>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	{#each tabs as t}
		{#if isActive(t)}
			<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">
				{t.label}
			</span>
		{:else}
			<a
				href={t.href}
				class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
			>
				{t.label}
			</a>
		{/if}
	{/each}
</div>
