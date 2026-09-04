<script lang="ts">
	import Skeleton from '$lib/Skeleton.svelte';
	import { api, ApiError, type AuditListResponse, type AuditEntry } from '$lib/api';

	let { apiBase, scopeLabel }: { apiBase: string; scopeLabel: string } = $props();

	let items = $state<AuditEntry[]>([]);
	let total = $state(0);
	let totalPages = $state(0);
	let curPage = $state(1);
	const perPage = 30;

	// Filters — kept locally; applied on submit so typing doesn't refetch
	// the whole page on every keystroke.
	let actionDraft = $state('');
	let actorDraft = $state('');
	let appliedAction = $state('');
	let appliedActor = $state('');

	let loading = $state(true);
	let loadError: string | null = $state(null);

	let expanded = $state<Record<number, boolean>>({});

	async function load() {
		loading = true;
		loadError = null;
		try {
			const params = new URLSearchParams();
			params.set('page', String(curPage));
			params.set('per_page', String(perPage));
			if (appliedAction) params.set('action', appliedAction);
			if (appliedActor) params.set('actor', appliedActor);
			const resp = await api.get<AuditListResponse>(`${apiBase}?${params}`);
			items = resp.items;
			total = resp.total_items;
			totalPages = resp.total_pages;
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		apiBase;
		curPage = 1;
		appliedAction = '';
		appliedActor = '';
		actionDraft = '';
		actorDraft = '';
		load();
	});

	function search(e: SubmitEvent) {
		e.preventDefault();
		appliedAction = actionDraft.trim();
		appliedActor = actorDraft.trim();
		curPage = 1;
		load();
	}

	function clear() {
		actionDraft = '';
		actorDraft = '';
		appliedAction = '';
		appliedActor = '';
		curPage = 1;
		load();
	}

	function gotoPage(p: number) {
		if (p < 1 || (totalPages > 0 && p > totalPages)) return;
		curPage = p;
		load();
	}

	function toggle(id: number) {
		expanded = { ...expanded, [id]: !expanded[id] };
	}

	function actionBadge(a: string): string {
		if (a.includes('delete')) return 'bg-red-100 text-red-800';
		if (a.includes('clamp')) return 'bg-amber-100 text-amber-800';
		if (a.includes('set') || a.includes('create')) return 'bg-emerald-100 text-emerald-800';
		return 'bg-slate-100 text-slate-700';
	}
</script>

<div class="mb-3 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Audit log</h1>
		<p class="mt-1 text-sm text-slate-500">
			{total} entr{total === 1 ? 'y' : 'ies'} for
			<code>{scopeLabel}</code>{#if appliedAction || appliedActor}
				· filtered{/if}
		</p>
	</div>
</div>

<form onsubmit={search} class="mb-4 flex flex-wrap gap-2">
	<input
		class="input flex-1 min-w-[12rem]"
		bind:value={actionDraft}
		placeholder="action substring (policy_clamped, …)"
	/>
	<input
		class="input flex-1 min-w-[10rem]"
		bind:value={actorDraft}
		placeholder="actor (admin id)"
	/>
	<button type="submit" class="btn-secondary">Search</button>
	{#if appliedAction || appliedActor}
		<button type="button" class="btn-secondary" onclick={clear}>Clear</button>
	{/if}
</form>

{#if loadError}
	<div class="error-banner mb-4">{loadError}</div>
{/if}

{#if loading}
	<Skeleton rows={3} class="mt-4 space-y-2 max-w-md" />
{:else if items.length === 0}
	<div class="card text-center text-slate-500">
		<p>No audit entries{appliedAction || appliedActor ? ' match this filter' : ' yet'}.</p>
		<p class="mt-1 text-xs">Policy changes, admin actions, and cascade clamps all land here.</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">When</th>
					<th class="px-4 py-2.5 font-medium">Action</th>
					<th class="px-4 py-2.5 font-medium">Target</th>
					<th class="px-4 py-2.5 font-medium">Actor</th>
					<th class="px-4 py-2.5"></th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each items as e}
					<tr class="hover:bg-slate-50">
						<td class="whitespace-nowrap px-4 py-2 text-xs text-slate-500">
							{new Date(e.ts).toLocaleString()}
						</td>
						<td class="px-4 py-2">
							<span
								class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {actionBadge(
									e.action
								)}"
							>
								{e.action}
							</span>
						</td>
						<td class="px-4 py-2 font-mono text-xs text-slate-700">{e.target ?? '—'}</td>
						<td class="max-w-[12rem] truncate px-4 py-2 font-mono text-xs text-slate-500">
							{e.actor ?? 'system'}
						</td>
						<td class="px-4 py-2 text-right">
							<button class="text-xs text-orange-600 hover:underline" onclick={() => toggle(e.id)}>
								{expanded[e.id] ? 'Hide' : 'Details'}
							</button>
						</td>
					</tr>
					{#if expanded[e.id]}
						<tr class="bg-slate-50">
							<td colspan="5" class="px-4 py-2">
								<pre
									class="overflow-x-auto whitespace-pre-wrap break-all rounded bg-slate-900 p-3 font-mono text-xs text-slate-100">{JSON.stringify(
										e.details,
										null,
										2
									)}</pre>
							</td>
						</tr>
					{/if}
				{/each}
			</tbody>
		</table>
	</div>

	<div class="mt-3 flex items-center justify-between text-sm text-slate-600">
		<span>Page {curPage} of {totalPages}</span>
		<div class="flex gap-2">
			<button class="btn-secondary" onclick={() => gotoPage(curPage - 1)} disabled={curPage <= 1}
				>← Prev</button
			>
			<button
				class="btn-secondary"
				onclick={() => gotoPage(curPage + 1)}
				disabled={curPage >= totalPages}>Next →</button
			>
		</div>
	</div>
{/if}
