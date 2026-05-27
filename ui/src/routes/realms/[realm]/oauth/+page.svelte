<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, type OAuthProvider } from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);

	let providers = $state<OAuthProvider[]>([]);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	async function load() {
		loading = true;
		loadError = null;
		try {
			providers = await api.get<OAuthProvider[]>(`/api/realms/${realm}/auth/oauth/providers`);
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		realm;
		load();
	});

	function open(p: OAuthProvider) {
		goto(`/realms/${realm}/oauth/${p.provider}`);
	}

	function openNew() {
		goto(`/realms/${realm}/oauth/new`);
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: 'OAuth providers' }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/realms/{realm}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Apps
	</a>
	<a
		href="/realms/{realm}/users"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Users
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">
		OAuth providers
	</span>
</div>

<div class="mb-6 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">OAuth providers</h1>
		<p class="mt-1 text-sm text-slate-500">
			Upstream identity providers for this realm. The client secret is encrypted at rest under
			the server's KEK; admin reads never echo it back.
		</p>
	</div>
	<button class="btn-primary" onclick={openNew}>+ New provider</button>
</div>

{#if loadError}
	<div class="error-banner mb-4">{loadError}</div>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if providers.length === 0}
	<div class="card text-center text-slate-500">
		<p>No providers configured.</p>
		<p class="mt-1 text-xs">
			Add Google, GitHub, or any OIDC-compatible provider to enable
			<code>/auth/oauth/&lt;provider&gt;/authorize</code> for this realm.
		</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">Provider</th>
					<th class="px-4 py-2.5 font-medium">Client ID</th>
					<th class="px-4 py-2.5 font-medium">Scopes</th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each providers as p}
					<tr class="cursor-pointer hover:bg-slate-50" onclick={() => open(p)}>
						<td class="px-4 py-2 font-mono text-slate-900">{p.provider}</td>
						<td class="max-w-xs truncate px-4 py-2 font-mono text-xs text-slate-700">
							{p.client_id}
						</td>
						<td class="px-4 py-2 text-xs text-slate-500">
							{p.config.scopes.join(' ')}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
