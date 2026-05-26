<script lang="ts">
	import { goto } from '$app/navigation';
	import { api, ApiError, type MasterLoginResponse } from '$lib/api';
	import { auth } from '$lib/auth.svelte';

	// First-run wizard. Mirrors the `POST /_/setup` contract:
	// one-shot creation of the first master admin. If the server has
	// already been initialized, the endpoint returns 409 conflict and
	// we redirect the user back to /login.
	let email = $state('');
	let password = $state('');
	let name = $state('');
	let submitting = $state(false);
	let error: string | null = $state(null);

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		submitting = true;
		error = null;
		try {
			// 1. Create the master admin.
			await api.post('/_/setup', { email, password, name: name || null }, { auth: false });
			// 2. Immediately log in with the same credentials so the user
			//    lands on /realms without retyping anything.
			const login = await api.post<MasterLoginResponse>(
				'/_/auth/admin/login',
				{ email, password },
				{ auth: false }
			);
			auth.setMasterSession(login);
			await goto('/realms');
		} catch (e) {
			if (e instanceof ApiError) {
				if (e.code === 'conflict' || e.status === 409) {
					error = 'A master admin already exists. Use the login form.';
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
		<p class="mt-1 text-sm text-slate-500">Create the first master admin</p>
	</div>

	<form onsubmit={submit} class="card space-y-4">
		{#if error}
			<div class="error-banner">{error}</div>
		{/if}

		<div>
			<label class="field-label" for="email">Email</label>
			<input
				id="email"
				type="email"
				class="input"
				autocomplete="email"
				bind:value={email}
				required
				disabled={submitting}
			/>
		</div>

		<div>
			<label class="field-label" for="name">Name (optional)</label>
			<input
				id="name"
				type="text"
				class="input"
				autocomplete="name"
				bind:value={name}
				disabled={submitting}
			/>
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
			{submitting ? 'Creating…' : 'Create master admin'}
		</button>
	</form>

	<p class="mt-6 text-center text-xs text-slate-500">
		Already initialized? <a href="/login" class="font-medium text-orange-600 hover:text-orange-700">Sign in →</a>
	</p>
</div>
