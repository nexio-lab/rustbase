<script lang="ts">
	import { goto } from "$lib/nav";
	import { api, ApiError, type MasterLoginResponse } from '$lib/api';
	import { auth } from '$lib/auth.svelte';

	// Master-admin login. The realm-admin form lives elsewhere and
	// keeps email-based credentials; this one accepts the master
	// admin's username (default "admin"). If the server hasn't been
	// initialized yet, the setup gate returns 503 and we link to /setup.
	let username = $state('admin');
	let password = $state('');
	let submitting = $state(false);
	let error: string | null = $state(null);
	let needsSetup = $state(false);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		submitting = true;
		error = null;
		needsSetup = false;
		try {
			const login = await api.post<MasterLoginResponse>(
				'/_/auth/admin/login',
				{ username, password },
				{ auth: false }
			);
			auth.setMasterSession(login);
			await goto('/realms');
		} catch (e) {
			if (e instanceof ApiError) {
				if (e.code === 'service_unavailable' || e.status === 503) {
					needsSetup = true;
					error = 'Setup hasn’t been completed yet.';
				} else if (e.code === 'unauthorized' || e.status === 401) {
					error = 'Invalid username or password.';
				} else {
					error = e.message;
				}
			} else {
				error = String(e);
			}
		} finally {
			submitting = false;
		}
	}
</script>

<div class="mx-auto mt-12 max-w-sm">
	<div class="mb-8 text-center">
		<div class="mx-auto mb-2 inline-block h-3 w-3 rounded-sm bg-orange-500"></div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Sign in to RustBaas</h1>
		<p class="mt-1 text-sm text-slate-500">Master admin credentials</p>
	</div>

	<form onsubmit={submit} class="card space-y-4">
		{#if error}
			<div class="error-banner">
				{error}
				{#if needsSetup}
					<a href="/setup" class="ml-1 font-medium underline">Run setup</a>
				{/if}
			</div>
		{/if}

		<div>
			<label class="field-label" for="username">Username</label>
			<input
				id="username"
				type="text"
				class="input"
				autocomplete="username"
				bind:value={username}
				required
				disabled={submitting}
			/>
		</div>

		<div>
			<label class="field-label" for="password">Password</label>
			<input
				id="password"
				type="password"
				class="input"
				autocomplete="current-password"
				bind:value={password}
				required
				disabled={submitting}
			/>
		</div>

		<button type="submit" class="btn-primary w-full" disabled={submitting}>
			{submitting ? 'Signing in…' : 'Sign in'}
		</button>
	</form>

	<p class="mt-6 text-center text-xs text-slate-500">
		First time? <a href="/setup" class="font-medium text-orange-600 hover:text-orange-700">Set the master password →</a>
	</p>
	<p class="mt-2 text-center text-xs text-slate-400">
		<a
			href={import.meta.env.VITE_DOCS_URL ?? 'https://pjonaszik.github.io/rustbase/'}
			target="_blank"
			rel="noopener noreferrer"
			class="hover:text-slate-600"
		>
			Documentation ↗
		</a>
	</p>
</div>
