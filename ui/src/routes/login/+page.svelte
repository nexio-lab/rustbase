<script lang="ts">
	import { goto } from '$app/navigation';
	import { api, ApiError, type MasterLoginResponse } from '$lib/api';
	import { auth } from '$lib/auth.svelte';

	// Login form. Master admins land here on first boot; once authed
	// the layout guard pushes them on to /realms. If the server hasn't
	// been initialized yet (no master admin), the first /login attempt
	// returns 503 service_unavailable — we surface a banner that links
	// to /setup so the user can create the first admin in-place.
	let email = $state('');
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
				{ email, password },
				{ auth: false }
			);
			auth.setMasterSession(login);
			await goto('/realms');
		} catch (e) {
			if (e instanceof ApiError) {
				if (e.code === 'service_unavailable' || e.status === 503) {
					needsSetup = true;
					error = 'No master admin yet — finish setup first.';
				} else if (e.code === 'unauthorized' || e.status === 401) {
					error = 'Invalid email or password.';
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
		First time? <a href="/setup" class="font-medium text-orange-600 hover:text-orange-700">Create the first master admin →</a>
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
