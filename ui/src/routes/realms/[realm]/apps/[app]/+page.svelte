<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		api,
		ApiError,
		type Collection,
		type CollectionKind,
		type Schema
	} from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);
	const app = $derived(page.params.app);

	let collections = $state<Collection[]>([]);
	let loading = $state(true);
	let listError: string | null = $state(null);

	let creating = $state(false);
	let newId = $state('');
	let newKind = $state<CollectionKind>('base');
	let createError: string | null = $state(null);
	let submitting = $state(false);

	async function load() {
		loading = true;
		listError = null;
		try {
			collections = await api.get<Collection[]>(
				`/api/realms/${realm}/apps/${app}/collections`
			);
		} catch (e) {
			listError = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		realm;
		app;
		load();
	});

	function openCreate() {
		creating = true;
		newId = '';
		newKind = 'base';
		createError = null;
	}

	function cancelCreate() {
		creating = false;
		createError = null;
	}

	async function submitCreate(e: SubmitEvent) {
		e.preventDefault();
		submitting = true;
		createError = null;
		try {
			// Start with the minimum: id + kind, no fields. The schema
			// editor on the next page lets the user grow it from there.
			const schema: Schema = { id: newId, kind: newKind, fields: [] };
			const created = await api.post<Collection>(
				`/api/realms/${realm}/apps/${app}/collections`,
				{ schema }
			);
			collections = [...collections, created].sort((a, b) => a.id.localeCompare(b.id));
			creating = false;
			// Drop the user directly into the schema editor for the
			// fresh collection — that's where they're heading anyway.
			goto(`/realms/${realm}/apps/${app}/collections/${created.id}`);
		} catch (e) {
			createError = e instanceof ApiError ? e.message : String(e);
		} finally {
			submitting = false;
		}
	}

	function openCollection(c: Collection) {
		goto(`/realms/${realm}/apps/${app}/collections/${c.id}`);
	}

	function kindBadge(k: CollectionKind): string {
		switch (k) {
			case 'auth':
				return 'bg-violet-100 text-violet-800';
			case 'view':
				return 'bg-amber-100 text-amber-800';
			default:
				return 'bg-slate-100 text-slate-700';
		}
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: app }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">
		Collections
	</span>
	<a
		href="/realms/{realm}/apps/{app}/policies"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Policies
	</a>
	<a
		href="/realms/{realm}/apps/{app}/hooks"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Hooks
	</a>
	<a
		href="/realms/{realm}/apps/{app}/files"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Files
	</a>
	<a
		href="/realms/{realm}/apps/{app}/audit"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Audit
	</a>
</div>

<div class="mb-6 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
			App <span class="font-mono">{app}</span>
		</h1>
		<p class="mt-1 text-sm text-slate-500">
			Collections are the schema's tables. Each one holds records of a fixed shape.
		</p>
	</div>
	{#if !creating}
		<button class="btn-primary" onclick={openCreate}>+ New collection</button>
	{/if}
</div>

{#if creating}
	<form onsubmit={submitCreate} class="card mb-6 max-w-lg space-y-4">
		<h2 class="text-lg font-semibold text-slate-900">Create collection</h2>
		{#if createError}
			<div class="error-banner">{createError}</div>
		{/if}
		<div>
			<label class="field-label" for="id">ID</label>
			<input
				id="id"
				type="text"
				class="input"
				bind:value={newId}
				placeholder="notes"
				pattern="[a-z][a-z0-9_]*"
				required
				disabled={submitting}
			/>
			<p class="mt-1 text-xs text-slate-500">
				lowercase letters, digits, underscores. Names starting with <code>_</code> are
				reserved.
			</p>
		</div>
		<div>
			<label class="field-label" for="kind">Kind</label>
			<select
				id="kind"
				class="input"
				bind:value={newKind}
				disabled={submitting}
			>
				<option value="base">base — plain records</option>
				<option value="auth">auth — users (auto email + password fields)</option>
				<option value="view" disabled>view — SQL-backed (read-only, coming soon)</option>
			</select>
		</div>
		<div class="flex gap-2">
			<button type="submit" class="btn-primary" disabled={submitting}>
				{submitting ? 'Creating…' : 'Create'}
			</button>
			<button type="button" class="btn-secondary" onclick={cancelCreate} disabled={submitting}>
				Cancel
			</button>
		</div>
	</form>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if listError}
	<div class="error-banner">{listError}</div>
{:else if collections.length === 0}
	<div class="card text-center text-slate-500">
		<p>No collections yet.</p>
		<p class="mt-1 text-xs">Click <strong>New collection</strong> to scaffold one.</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">ID</th>
					<th class="px-4 py-2.5 font-medium">Kind</th>
					<th class="px-4 py-2.5 font-medium">Fields</th>
					<th class="px-4 py-2.5 font-medium">Updated</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each collections as coll}
					<tr class="cursor-pointer hover:bg-slate-50" onclick={() => openCollection(coll)}>
						<td class="px-4 py-2.5 font-mono text-slate-900">{coll.id}</td>
						<td class="px-4 py-2.5">
							<span
								class="inline-flex rounded-full px-2 py-0.5 text-xs font-medium {kindBadge(
									coll.kind
								)}"
							>
								{coll.kind}
							</span>
						</td>
						<td class="px-4 py-2.5 text-slate-700">{coll.schema.fields.length}</td>
						<td class="px-4 py-2.5 text-slate-500">
							{new Date(coll.updated_at).toLocaleString()}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
