<script lang="ts">
	import { goto } from "$lib/nav";
	import { api, ApiError, type MasterLoginResponse } from '$lib/api';
	import { auth } from '$lib/auth.svelte';

	// First-run wizard. The server has already auto-seeded the master
	// admin row (username `admin`, NULL password) on first boot; the
	// wizard's only job is to set that password. If the server has
	// already been initialized, the endpoint returns 409 conflict and
	// we redirect the user back to /login.
	let password = $state('');
	let submitting = $state(false);
	let error: string | null = $state(null);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		submitting = true;
		error = null;
		try {
			// 1. Set the master admin password.
			await api.post('/_/setup', { password }, { auth: false });
			// 2. Immediately log in with the canonical "admin" username so
			//    the user lands on /workspaces without retyping anything.
			const login = await api.post<MasterLoginResponse>(
				'/_/auth/admin/login',
				{ username: 'admin', password },
				{ auth: false }
			);
			auth.setMasterSession({ admin: login.admin });
			await goto('/workspaces');
		} catch (e) {
			if (e instanceof ApiError) {
				if (e.code === 'conflict' || e.status === 409) {
					error = 'Setup has already been completed. Use the login form.';
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
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Set up RustBase</h1>
		<p class="mt-1 text-sm text-slate-500">
			Set the password for the master <code>admin</code> account
		</p>
	</div>

	<form onsubmit={submit} class="card space-y-4">
		{#if error}
			<div class="error-banner">{error}</div>
		{/if}

		<div>
			<label class="field-label" for="username">Username</label>
			<input
				id="username"
				type="text"
				class="input bg-slate-50 text-slate-500"
				value="admin"
				disabled
				readonly
			/>
			<p class="mt-1 text-xs text-slate-500">
				Fixed at boot. Use this name to sign in afterwards.
			</p>
		</div>

		<div>
			<label class="field-label" for="password">Password</label>
			<input
				id="password"
				type="password"
				class="input"
				autocomplete="new-password"
				bind:value={password}
				minlength={8}
				required
				disabled={submitting}
			/>
			<p class="mt-1 text-xs text-slate-500">8 characters minimum.</p>
		</div>

		<button type="submit" class="btn-primary w-full" disabled={submitting}>
			{submitting ? 'Saving…' : 'Set master password'}
		</button>
	</form>

	<p class="mt-6 text-center text-xs text-slate-500">
		Already initialized? <a href="/login" class="font-medium text-orange-600 hover:text-orange-700">Sign in →</a>
	</p>
</div>
