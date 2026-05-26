<script lang="ts">
	import { page } from '$app/state';
	const realm = $derived(page.params.realm);
</script>

<div class="mb-6">
	<a href="/realms" class="text-sm text-slate-500 hover:text-slate-700">← All realms</a>
	<h1 class="mt-1 text-2xl font-semibold tracking-tight text-slate-900">
		Realm: <span class="font-mono">{realm}</span>
	</h1>
</div>

<div class="card text-center text-slate-500">
	<p>The per-realm dashboard (apps, users, OAuth providers, policies, hooks, files) lands in the next branch.</p>
	<p class="mt-2 text-xs">For now, drive everything else from the API directly.</p>
</div>
