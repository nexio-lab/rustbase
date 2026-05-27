<script lang="ts">
	import { goto } from '$app/navigation';
	import { api, ApiError, type Realm } from '$lib/api';

	// State the page owns directly: the list of realms + the
	// inline-create form. Keeping the create form inline (rather than
	// a separate /realms/new route) means the user never loses the
	// list context.

	let realms = $state<Realm[]>([]);
	let loading = $state(true);
	let listError: string | null = $state(null);

	let creating = $state(false);
	let newId = $state('');
	let newName = $state('');
	let createError: string | null = $state(null);
	let submitting = $state(false);

	async function load() {
		loading = true;
		listError = null;
		try {
			realms = await api.get<Realm[]>('/api/realms');
		} catch (e) {
			listError = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
	});

	function openCreate() {
		creating = true;
		newId = '';
		newName = '';
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
			const created = await api.post<Realm>('/api/realms', { id: newId, name: newName });
			realms = [...realms, created].sort((a, b) => a.id.localeCompare(b.id));
			creating = false;
		} catch (e) {
			if (e instanceof ApiError) {
				createError = e.message;
			} else {
				createError = String(e);
			}
		} finally {
			submitting = false;
		}
	}

	function openRealm(realm: Realm) {
		// Phase 2 wires per-realm pages. For now, just stash the chosen
		// realm in the URL — a follow-up branch picks it up.
		goto(`/realms/${realm.id}`);
	}
</script>

<div class="mb-6 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Realms</h1>
		<p class="mt-1 text-sm text-slate-500">
			Identity boundaries. Each realm owns its own user pool, OAuth providers, and apps.
		</p>
	</div>
	{#if !creating}
		<button class="btn-primary" onclick={openCreate}>+ New realm</button>
	{/if}
</div>

{#if creating}
	<form onsubmit={submitCreate} class="card mb-6 max-w-lg space-y-4">
		<h2 class="text-lg font-semibold text-slate-900">Create realm</h2>
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
				placeholder="acme"
				pattern="[a-z][a-z0-9-]*"
				required
				disabled={submitting}
			/>
			<p class="mt-1 text-xs text-slate-500">
				lowercase letters, digits, hyphens. The master realm <code>master</code> is reserved.
			</p>
		</div>
		<div>
			<label class="field-label" for="name">Name</label>
			<input
				id="name"
				type="text"
				class="input"
				bind:value={newName}
				placeholder="Acme Inc."
				required
				disabled={submitting}
			/>
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
{:else if realms.length === 0}
	<div class="card text-center text-slate-500">
		<p>No realms yet.</p>
		<p class="mt-1 text-xs">Click <strong>New realm</strong> to create one.</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">ID</th>
					<th class="px-4 py-2.5 font-medium">Name</th>
					<th class="px-4 py-2.5 font-medium">Created</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each realms as realm}
					<tr
						class="cursor-pointer hover:bg-slate-50"
						onclick={() => openRealm(realm)}
					>
						<td class="px-4 py-2.5 font-mono text-slate-900">{realm.id}</td>
						<td class="px-4 py-2.5 text-slate-700">{realm.name}</td>
						<td class="px-4 py-2.5 text-slate-500">
							{new Date(realm.created_at).toLocaleString()}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
