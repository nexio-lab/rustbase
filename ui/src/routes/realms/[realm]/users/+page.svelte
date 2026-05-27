<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, type AdminUser, type AdminUserListResponse } from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);

	let items = $state<AdminUser[]>([]);
	let total = $state(0);
	let totalPages = $state(0);
	let curPage = $state(1);
	const perPage = 30;

	// Search bar — substring match on email, runs through ?q=…
	let q = $state('');
	let appliedQ = $state('');

	let loading = $state(true);
	let loadError: string | null = $state(null);

	async function load() {
		loading = true;
		loadError = null;
		try {
			const params = new URLSearchParams();
			params.set('page', String(curPage));
			params.set('per_page', String(perPage));
			if (appliedQ) params.set('q', appliedQ);
			const resp = await api.get<AdminUserListResponse>(
				`/api/realms/${realm}/users?${params}`
			);
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
		realm;
		curPage = 1;
		load();
	});

	function search(e: SubmitEvent) {
		e.preventDefault();
		appliedQ = q.trim();
		curPage = 1;
		load();
	}

	function clear() {
		q = '';
		appliedQ = '';
		curPage = 1;
		load();
	}

	function gotoPage(p: number) {
		if (p < 1 || (totalPages > 0 && p > totalPages)) return;
		curPage = p;
		load();
	}

	function openUser(u: AdminUser) {
		goto(`/realms/${realm}/users/${u.id}`);
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: 'Users' }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/realms/{realm}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Apps
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Users</span>
	<a
		href="/realms/{realm}/oauth"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		OAuth providers
	</a>
	<a
		href="/realms/{realm}/policies"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Policies
	</a>
</div>

<div class="mb-4">
	<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Users</h1>
	<p class="mt-1 text-sm text-slate-500">
		{total} user{total === 1 ? '' : 's'} in this realm{#if appliedQ}
			· matching <code>{appliedQ}</code>
		{/if}
	</p>
</div>

<form onsubmit={search} class="mb-4 flex gap-2">
	<input class="input" bind:value={q} placeholder="search by email substring…" />
	<button type="submit" class="btn-secondary">Search</button>
	{#if appliedQ}
		<button type="button" class="btn-secondary" onclick={clear}>Clear</button>
	{/if}
</form>

{#if loadError}
	<div class="error-banner mb-4">{loadError}</div>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if items.length === 0}
	<div class="card text-center text-slate-500">
		<p>No users{appliedQ ? ' match this search' : ' yet'}.</p>
		<p class="mt-1 text-xs">
			Users register themselves via <code>/auth/users/register</code>, or sign up via OTP /
			OAuth.
		</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">Email</th>
					<th class="px-4 py-2.5 font-medium">Status</th>
					<th class="px-4 py-2.5 font-medium">Created</th>
					<th class="px-4 py-2.5 font-medium">Last login</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each items as u}
					<tr class="cursor-pointer hover:bg-slate-50" onclick={() => openUser(u)}>
						<td class="px-4 py-2 font-mono text-slate-900">{u.email}</td>
						<td class="px-4 py-2">
							{#if u.verified}
								<span
									class="mr-1 inline-flex rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800"
									>verified</span
								>
							{:else}
								<span
									class="mr-1 inline-flex rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800"
									>unverified</span
								>
							{/if}
							{#if !u.has_password}
								<span
									class="inline-flex rounded-full bg-violet-100 px-2 py-0.5 text-xs font-medium text-violet-800"
									>passwordless</span
								>
							{/if}
						</td>
						<td class="px-4 py-2 text-xs text-slate-500">
							{new Date(u.created_at).toLocaleString()}
						</td>
						<td class="px-4 py-2 text-xs text-slate-500">
							{u.last_login ? new Date(u.last_login).toLocaleString() : '—'}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<div class="mt-3 flex items-center justify-between text-sm text-slate-600">
		<span>Page {curPage} of {totalPages}</span>
		<div class="flex gap-2">
			<button
				class="btn-secondary"
				onclick={() => gotoPage(curPage - 1)}
				disabled={curPage <= 1}>← Prev</button
			>
			<button
				class="btn-secondary"
				onclick={() => gotoPage(curPage + 1)}
				disabled={curPage >= totalPages}>Next →</button
			>
		</div>
	</div>
{/if}
