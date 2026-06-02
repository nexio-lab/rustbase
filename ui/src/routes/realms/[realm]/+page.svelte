<script lang="ts">
	import { goto } from "$lib/nav";
	import { page } from '$app/state';
	import { api, ApiError, type App } from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);

	let apps = $state<App[]>([]);
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
			apps = await api.get<App[]>(`/api/realms/${realm}/apps`);
		} catch (e) {
			listError = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	// Reruns whenever the URL realm slug changes (so /realms/acme → /realms/widgets
	// without a full page reload still refreshes the list).
	$effect(() => {
		realm;
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
			const created = await api.post<App>(`/api/realms/${realm}/apps`, {
				id: newId,
				name: newName
			});
			apps = [...apps, created].sort((a, b) => a.id.localeCompare(b.id));
			creating = false;
		} catch (e) {
			createError = e instanceof ApiError ? e.message : String(e);
		} finally {
			submitting = false;
		}
	}

	function openApp(app: App) {
		goto(`/realms/${realm}/apps/${app.id}`);
	}
</script>

<Breadcrumbs items={[{ label: 'Realms', href: '/realms' }, { label: realm }]} />

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Apps</span>
	<a
		href="/realms/{realm}/policies"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Policies
	</a>
	<a
		href="/realms/{realm}/audit"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Audit
	</a>
</div>

<div class="mb-6 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
			Realm <span class="font-mono">{realm}</span>
		</h1>
		<p class="mt-1 text-sm text-slate-500">
			Apps are the data products inside this realm. Each app owns its own collections, records, files, and hooks.
		</p>
	</div>
	{#if !creating}
		<button class="btn-primary" onclick={openCreate}>+ New app</button>
	{/if}
</div>

{#if creating}
	<form onsubmit={submitCreate} class="card mb-6 max-w-lg space-y-4">
		<h2 class="text-lg font-semibold text-slate-900">Create app</h2>
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
				placeholder="mobile"
				pattern="[a-z][a-z0-9-]*"
				required
				disabled={submitting}
			/>
			<p class="mt-1 text-xs text-slate-500">lowercase letters, digits, hyphens.</p>
		</div>
		<div>
			<label class="field-label" for="name">Name</label>
			<input
				id="name"
				type="text"
				class="input"
				bind:value={newName}
				placeholder="Mobile"
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
{:else if apps.length === 0}
	<div class="card text-center text-slate-500">
		<p>No apps yet.</p>
		<p class="mt-1 text-xs">Click <strong>New app</strong> to create one.</p>
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
				{#each apps as app}
					<tr class="cursor-pointer hover:bg-slate-50" onclick={() => openApp(app)}>
						<td class="px-4 py-2.5 font-mono text-slate-900">{app.id}</td>
						<td class="px-4 py-2.5 text-slate-700">{app.name}</td>
						<td class="px-4 py-2.5 text-slate-500">
							{new Date(app.created_at).toLocaleString()}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
