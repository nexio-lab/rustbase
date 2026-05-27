<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, type AdminUserDetail } from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);
	const id = $derived(page.params.id);

	let user = $state<AdminUserDetail | null>(null);
	let loading = $state(true);
	let loadError: string | null = $state(null);
	let busy = $state(false);

	async function load() {
		loading = true;
		loadError = null;
		try {
			user = await api.get<AdminUserDetail>(`/api/realms/${realm}/users/${id}`);
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		realm;
		id;
		load();
	});

	async function verify() {
		if (!user) return;
		busy = true;
		try {
			await api.patch(`/api/realms/${realm}/users/${id}/verify`, {});
			await load();
		} catch (e) {
			alert(e instanceof ApiError ? e.message : String(e));
		} finally {
			busy = false;
		}
	}

	async function resetTotp() {
		if (!confirm('Remove TOTP enrolment for this user?\n\nThey will be able to log in without the second factor until they re-enroll.')) return;
		busy = true;
		try {
			await api.delete(`/api/realms/${realm}/users/${id}/totp`);
			await load();
		} catch (e) {
			alert(e instanceof ApiError ? e.message : String(e));
		} finally {
			busy = false;
		}
	}

	async function deleteUser() {
		if (!user) return;
		if (
			!confirm(
				`Delete user ${user.email}?\n\nThis is irreversible. All auth-side rows (verifications, password resets, OTPs, TOTP, OAuth links) cascade-delete with the user row.`
			)
		)
			return;
		busy = true;
		try {
			await api.delete(`/api/realms/${realm}/users/${id}`);
			await goto(`/realms/${realm}/users`);
		} catch (e) {
			alert(e instanceof ApiError ? e.message : String(e));
			busy = false;
		}
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: 'Users', href: `/realms/${realm}/users` },
		{ label: user?.email ?? id.slice(0, 8) }
	]}
/>

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if loadError}
	<div class="error-banner">{loadError}</div>
{:else if user}
	<div class="mb-6 flex items-start justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight text-slate-900">{user.email}</h1>
			<p class="mt-1 font-mono text-xs text-slate-500">{user.id}</p>
		</div>
		<button class="btn-secondary border-red-300 text-red-700 hover:bg-red-50" onclick={deleteUser} disabled={busy}>
			Delete user
		</button>
	</div>

	<!-- Profile card -->
	<section class="card mb-4">
		<h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500">Profile</h2>
		<dl class="grid grid-cols-2 gap-x-6 gap-y-3 text-sm">
			<dt class="text-slate-500">Email verified</dt>
			<dd>
				{#if user.verified}
					<span class="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800">verified</span>
				{:else}
					<span class="mr-2 rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">unverified</span>
					<button class="text-xs text-orange-600 hover:text-orange-700" onclick={verify} disabled={busy}>
						Force-verify
					</button>
				{/if}
			</dd>
			<dt class="text-slate-500">Password set</dt>
			<dd>{user.has_password ? 'yes' : 'no — passwordless (OTP / OAuth)'}</dd>
			<dt class="text-slate-500">Created</dt>
			<dd>{new Date(user.created_at).toLocaleString()}</dd>
			<dt class="text-slate-500">Last login</dt>
			<dd>{user.last_login ? new Date(user.last_login).toLocaleString() : '—'}</dd>
		</dl>
	</section>

	<!-- TOTP card -->
	<section class="card mb-4">
		<div class="mb-3 flex items-center justify-between">
			<h2 class="text-sm font-semibold uppercase tracking-wider text-slate-500">TOTP</h2>
			{#if user.totp}
				<button class="text-xs text-red-600 hover:text-red-800" onclick={resetTotp} disabled={busy}>
					Reset TOTP
				</button>
			{/if}
		</div>
		{#if user.totp}
			<dl class="grid grid-cols-2 gap-x-6 gap-y-3 text-sm">
				<dt class="text-slate-500">Status</dt>
				<dd>
					{#if user.totp.enabled}
						<span class="rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800">enabled</span>
					{:else}
						<span class="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">pending</span>
					{/if}
				</dd>
				<dt class="text-slate-500">Enrolled</dt>
				<dd>{new Date(user.totp.enrolled_at).toLocaleString()}</dd>
				<dt class="text-slate-500">Confirmed</dt>
				<dd>
					{user.totp.confirmed_at ? new Date(user.totp.confirmed_at).toLocaleString() : '—'}
				</dd>
			</dl>
			<p class="mt-3 text-xs text-slate-500">
				Resetting TOTP removes the secret without touching the password. The user's next
				password login skips the second factor until they re-enroll.
			</p>
		{:else}
			<p class="text-sm text-slate-500">No TOTP enrolment.</p>
		{/if}
	</section>

	<!-- OAuth links card -->
	<section class="card">
		<h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500">
			Linked OAuth providers
		</h2>
		{#if user.oauth_links.length === 0}
			<p class="text-sm text-slate-500">None.</p>
		{:else}
			<ul class="divide-y divide-slate-200 text-sm">
				{#each user.oauth_links as link}
					<li class="flex items-center justify-between py-2">
						<span class="font-medium text-slate-900">{link.provider}</span>
						<span class="font-mono text-xs text-slate-500">{link.provider_user_id}</span>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
{/if}
