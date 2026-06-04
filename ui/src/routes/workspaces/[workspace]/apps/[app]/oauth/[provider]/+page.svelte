<script lang="ts">
	import { goto } from "$lib/nav";
	import { page } from '$app/state';
	import {
		api,
		ApiError,
		type OAuthProvider,
		type OAuthProviderConfig,
		type OAuthProviderPut
	} from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);
	const slug = $derived(page.params.provider);
	const isNew = $derived(slug === 'new');

	let loading = $state(true);
	let loadError: string | null = $state(null);
	let busy = $state(false);
	let formError: string | null = $state(null);

	// Form state. The provider slug is editable only on create; the
	// secret field is empty on edit (the server preserves the ciphertext
	// when we don't send one) and required on create.
	let providerSlug = $state('');
	let clientId = $state('');
	let clientSecret = $state('');
	let scopesRaw = $state('openid email');
	let cfg = $state<OAuthProviderConfig>({
		auth_url: '',
		token_url: '',
		userinfo_url: '',
		scopes: [],
		userinfo_id_field: '/sub',
		userinfo_email_field: '/email'
	});

	type Preset = {
		label: string;
		config: Partial<OAuthProviderConfig>;
		scopes: string;
		hint?: string;
	};

	const PRESETS: Record<string, Preset> = {
		google: {
			label: 'Google',
			config: {
				auth_url: 'https://accounts.google.com/o/oauth2/v2/auth',
				token_url: 'https://oauth2.googleapis.com/token',
				userinfo_url: 'https://openidconnect.googleapis.com/v1/userinfo',
				userinfo_id_field: '/sub',
				userinfo_email_field: '/email'
			},
			scopes: 'openid email profile'
		},
		github: {
			label: 'GitHub',
			config: {
				auth_url: 'https://github.com/login/oauth/authorize',
				token_url: 'https://github.com/login/oauth/access_token',
				userinfo_url: 'https://api.github.com/user',
				userinfo_id_field: '/id',
				userinfo_email_field: '/email'
			},
			scopes: 'read:user user:email',
			hint: 'GitHub returns `id` (number) and may not include `email` unless the user has set one as public.'
		},
		microsoft: {
			label: 'Microsoft',
			config: {
				auth_url: 'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',
				token_url: 'https://login.microsoftonline.com/common/oauth2/v2.0/token',
				userinfo_url: 'https://graph.microsoft.com/oidc/userinfo',
				userinfo_id_field: '/sub',
				userinfo_email_field: '/email'
			},
			scopes: 'openid email profile'
		}
	};

	async function load() {
		loading = true;
		loadError = null;
		try {
			if (isNew) {
				// Brand-new — keep the defaults already in `cfg`.
				return;
			}
			const got = await api.get<OAuthProvider>(
				`/api/workspaces/${workspace}/apps/${app}/auth/oauth/providers/${slug}`
			);
			providerSlug = got.provider;
			clientId = got.client_id;
			cfg = got.config;
			scopesRaw = got.config.scopes.join(' ');
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		workspace;
		app;
		slug;
		load();
	});

	function applyPreset(name: string) {
		const p = PRESETS[name];
		if (!p) return;
		cfg = { ...cfg, ...p.config };
		scopesRaw = p.scopes;
		if (isNew && !providerSlug) providerSlug = name;
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		busy = true;
		formError = null;
		try {
			const finalSlug = isNew ? providerSlug.trim() : slug;
			if (!finalSlug) {
				formError = 'Provider slug is required.';
				return;
			}
			const body: OAuthProviderPut = {
				client_id: clientId,
				config: {
					...cfg,
					scopes: scopesRaw
						.split(/\s+/)
						.map((s) => s.trim())
						.filter((s) => s.length > 0)
				}
			};
			if (clientSecret) body.client_secret = clientSecret;
			else if (isNew) {
				formError = 'Client secret is required when creating a provider.';
				return;
			}
			await api.put<OAuthProvider>(
				`/api/workspaces/${workspace}/apps/${app}/auth/oauth/providers/${finalSlug}`,
				body
			);
			await goto(`/workspaces/${workspace}/apps/${app}/oauth`);
		} catch (e) {
			formError = e instanceof ApiError ? e.message : String(e);
		} finally {
			busy = false;
		}
	}

	async function remove() {
		if (isNew) return;
		if (!confirm(`Delete OAuth provider "${slug}"?\n\nExisting users linked to this provider keep their login records, but no new sign-ins will work until you add the provider back.`)) return;
		busy = true;
		formError = null;
		try {
			await api.delete(`/api/workspaces/${workspace}/apps/${app}/auth/oauth/providers/${slug}`);
			await goto(`/workspaces/${workspace}/apps/${app}/oauth`);
		} catch (e) {
			formError = e instanceof ApiError ? e.message : String(e);
			busy = false;
		}
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: 'OAuth providers', href: `/workspaces/${workspace}/apps/${app}/oauth` },
		{ label: isNew ? 'New' : slug }
	]}
/>

{#if loading && !isNew}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if loadError}
	<div class="error-banner">{loadError}</div>
{:else}
	<div class="mb-6 flex items-start justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
				{isNew ? 'New OAuth provider' : `Provider ${slug}`}
			</h1>
			{#if !isNew}
				<p class="mt-1 text-xs text-slate-500">
					Redirect URI for the upstream:
					<code class="rounded bg-slate-100 px-1.5 py-0.5">
						https://&lt;your-host&gt;/_/auth/oauth/{slug}/callback
					</code>
				</p>
			{/if}
		</div>
		{#if !isNew}
			<button
				class="btn-secondary border-red-300 text-red-700 hover:bg-red-50"
				onclick={remove}
				disabled={busy}
			>
				Delete
			</button>
		{/if}
	</div>

	{#if formError}
		<div class="error-banner mb-4">{formError}</div>
	{/if}

	{#if isNew}
		<div class="mb-4 flex items-center gap-2 text-sm text-slate-600">
			<span>Preset:</span>
			{#each Object.entries(PRESETS) as [k, p]}
				<button class="btn-secondary py-1 text-xs" onclick={() => applyPreset(k)}>
					{p.label}
				</button>
			{/each}
			<span class="ml-2 text-xs text-slate-400">— autofills the URLs + scopes</span>
		</div>
	{/if}

	<form onsubmit={submit} class="card max-w-2xl space-y-4">
		<div class="grid grid-cols-2 gap-4">
			<div>
				<label class="field-label" for="provider">Provider slug</label>
				<input
					id="provider"
					class="input"
					bind:value={providerSlug}
					placeholder="google"
					pattern="[a-z][a-z0-9-]*"
					required={isNew}
					disabled={!isNew || busy}
				/>
				<p class="mt-1 text-xs text-slate-500">
					Used in the URL path: <code>/auth/oauth/{providerSlug || '...'}/authorize</code>
				</p>
			</div>
			<div>
				<label class="field-label" for="client_id">Client ID</label>
				<input
					id="client_id"
					class="input"
					bind:value={clientId}
					placeholder="abc.apps.googleusercontent.com"
					required
					disabled={busy}
				/>
			</div>
		</div>

		<div>
			<label class="field-label" for="client_secret">
				Client secret {isNew
					? '(required)'
					: '(leave blank to keep existing)'}
			</label>
			<input
				id="client_secret"
				type="password"
				class="input font-mono"
				bind:value={clientSecret}
				autocomplete="new-password"
				required={isNew}
				disabled={busy}
				placeholder={isNew ? '' : '••••••••••••••••'}
			/>
		</div>

		<hr class="border-slate-200" />

		<div>
			<label class="field-label" for="auth_url">Authorize URL</label>
			<input
				id="auth_url"
				type="url"
				class="input font-mono text-xs"
				bind:value={cfg.auth_url}
				required
				disabled={busy}
			/>
		</div>
		<div>
			<label class="field-label" for="token_url">Token URL</label>
			<input
				id="token_url"
				type="url"
				class="input font-mono text-xs"
				bind:value={cfg.token_url}
				required
				disabled={busy}
			/>
		</div>
		<div>
			<label class="field-label" for="userinfo_url">Userinfo URL</label>
			<input
				id="userinfo_url"
				type="url"
				class="input font-mono text-xs"
				bind:value={cfg.userinfo_url}
				required
				disabled={busy}
			/>
		</div>

		<div>
			<label class="field-label" for="scopes">Scopes (space-separated)</label>
			<input
				id="scopes"
				class="input font-mono text-xs"
				bind:value={scopesRaw}
				placeholder="openid email"
				disabled={busy}
			/>
		</div>

		<div class="grid grid-cols-2 gap-4">
			<div>
				<label class="field-label" for="id_field">User-info ID field (JSON pointer)</label>
				<input
					id="id_field"
					class="input font-mono text-xs"
					bind:value={cfg.userinfo_id_field}
					placeholder="/sub"
					required
					disabled={busy}
				/>
				<p class="mt-1 text-xs text-slate-500">Most providers: <code>/sub</code> (OIDC) or <code>/id</code> (GitHub).</p>
			</div>
			<div>
				<label class="field-label" for="email_field">User-info email field</label>
				<input
					id="email_field"
					class="input font-mono text-xs"
					bind:value={cfg.userinfo_email_field}
					placeholder="/email"
					required
					disabled={busy}
				/>
			</div>
		</div>

		<div class="flex gap-2">
			<button type="submit" class="btn-primary" disabled={busy}>
				{busy ? 'Saving…' : isNew ? 'Create provider' : 'Save changes'}
			</button>
			<a href="/workspaces/{workspace}/apps/{app}/oauth" class="btn-secondary">Cancel</a>
		</div>
	</form>
{/if}
