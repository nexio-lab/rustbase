<script lang="ts">
	import {
		api,
		ApiError,
		type ClampOutcome,
		type PolicyResponse,
		type PolicySpec,
		type PutPolicyResponse
	} from './api';
	import PolicyEditor from './PolicyEditor.svelte';

	/**
	 * Scope-agnostic policies list + editor. Used by the three policy
	 * pages (system / workspace / app), each of which passes the right
	 * REST endpoint base — everything below just appends `/{field}`
	 * for the per-row endpoints.
	 *
	 * `apiBase` examples:
	 *   /api/system/policies
	 *   /api/workspaces/acme/policies
	 *   /api/workspaces/acme/apps/mobile/policies
	 */
	let { apiBase, scopeLabel }: { apiBase: string; scopeLabel: string } = $props();

	let rows = $state<PolicyResponse[]>([]);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	// Editor state — opened by clicking a row, by "+ New", or after a
	// cascade put response (so the user can see the clamp outcomes
	// before they leave the page).
	let editing = $state<{
		field: string;
		spec: PolicySpec;
		isNew: boolean;
		cascade: ClampOutcome[];
	} | null>(null);
	let saving = $state(false);
	let editError: string | null = $state(null);

	async function load() {
		loading = true;
		loadError = null;
		try {
			rows = await api.get<PolicyResponse[]>(apiBase);
			rows.sort((a, b) => a.field.localeCompare(b.field));
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		apiBase;
		load();
	});

	function openNew() {
		editing = {
			field: '',
			spec: { kind: 'range', min: 0, max: 100 },
			isNew: true,
			cascade: []
		};
		editError = null;
	}

	function openEdit(r: PolicyResponse) {
		editing = {
			field: r.field,
			spec: structuredClone(r.spec) as PolicySpec,
			isNew: false,
			cascade: []
		};
		editError = null;
	}

	function close() {
		editing = null;
		editError = null;
	}

	async function save() {
		if (!editing) return;
		const f = editing.field.trim();
		if (!f) {
			editError = 'Field name is required.';
			return;
		}
		saving = true;
		editError = null;
		try {
			const resp = await api.put<PutPolicyResponse>(
				`${apiBase}/${encodeURIComponent(f)}`,
				editing.spec
			);
			editing = { ...editing, cascade: resp.cascaded, isNew: false };
			await load();
		} catch (e) {
			editError = e instanceof ApiError ? e.message : String(e);
		} finally {
			saving = false;
		}
	}

	async function remove(r: PolicyResponse) {
		if (
			!confirm(
				`Delete policy ${r.field}?\n\nChildren currently bounded by this policy will keep their stored values; the constraint just stops being checked.`
			)
		)
			return;
		try {
			await api.delete(`${apiBase}/${encodeURIComponent(r.field)}`);
			await load();
		} catch (e) {
			alert(e instanceof ApiError ? e.message : String(e));
		}
	}

	function summarise(s: PolicySpec): string {
		switch (s.kind) {
			case 'range':
				return `range [${s.min}, ${s.max}]`;
			case 'toggle':
				return s.state === 'open'
					? `toggle open (default=${s.default})`
					: `toggle locked = ${s.value}`;
			case 'enum_set':
				return `enum_set {${s.allowed.join(', ')}}`;
			case 'free':
				return 'free';
		}
	}
</script>

<div class="mb-6 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
			Policies — {scopeLabel}
		</h1>
		<p class="mt-1 text-sm text-slate-500">
			Hierarchical knobs. Master sets the outer bound; workspaces tighten within master; apps
			tighten within their workspace. Cascade auto-clamps existing children when a parent
			narrows.
		</p>
	</div>
	<button class="btn-primary" onclick={openNew}>+ New policy</button>
</div>

{#if loadError}
	<div class="error-banner mb-4">{loadError}</div>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if rows.length === 0}
	<div class="card text-center text-slate-500">
		<p>No policies at this scope yet.</p>
		<p class="mt-1 text-xs">
			Try fields like <code>password.length</code>, <code>mailer.daily_quota</code>,
			<code>oauth.providers</code>.
		</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">Field</th>
					<th class="px-4 py-2.5 font-medium">Spec</th>
					<th class="px-4 py-2.5 font-medium">Updated</th>
					<th class="px-4 py-2.5"></th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each rows as r}
					<tr class="hover:bg-slate-50">
						<td class="px-4 py-2 font-mono text-slate-900">{r.field}</td>
						<td class="px-4 py-2 font-mono text-xs text-slate-700">{summarise(r.spec)}</td>
						<td class="px-4 py-2 text-xs text-slate-500">
							{new Date(r.updated_at).toLocaleString()}
						</td>
						<td class="px-4 py-2 text-right text-xs whitespace-nowrap">
							<button class="text-slate-600 hover:text-slate-900" onclick={() => openEdit(r)}>
								Edit
							</button>
							<span class="mx-1 text-slate-300">·</span>
							<button class="text-red-600 hover:text-red-800" onclick={() => remove(r)}>
								Delete
							</button>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

{#if editing}
	<div
		class="fixed inset-0 z-10 flex items-start justify-center bg-slate-900/40 p-6"
		role="dialog"
		aria-modal="true"
	>
		<div class="mt-12 w-full max-w-xl rounded-lg border border-slate-200 bg-white p-6 shadow-xl">
			<div class="mb-4 flex items-center justify-between">
				<h2 class="text-lg font-semibold text-slate-900">
					{editing.isNew ? 'New policy' : `Edit ${editing.field}`}
				</h2>
				<button onclick={close} aria-label="Close" class="text-slate-400 hover:text-slate-600">✕</button>
			</div>

			{#if editError}
				<div class="error-banner mb-4">{editError}</div>
			{/if}

			{#if editing.cascade.length > 0}
				<div class="mb-4 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm">
					<p class="font-medium text-amber-900">
						Tightened the parent bound — {editing.cascade.length}
						child{editing.cascade.length === 1 ? '' : 'ren'} auto-clamped:
					</p>
					<ul class="mt-1 list-disc pl-5 text-xs text-amber-800">
						{#each editing.cascade as c}
							<li>
								<span class="font-mono">{c.workspace}{c.app ? `/${c.app}` : ''}</span>
								— {c.field}
							</li>
						{/each}
					</ul>
				</div>
			{/if}

			<div class="space-y-4">
				<div>
					<label class="field-label" for="field">Field</label>
					<input
						id="field"
						class="input font-mono"
						bind:value={editing.field}
						placeholder="mailer.daily_quota"
						pattern="[a-z][a-z0-9_.]*"
						required
						disabled={!editing.isNew || saving}
					/>
					<p class="mt-1 text-xs text-slate-500">
						Convention: dot-separated, e.g. <code>password.length</code>,
						<code>mailer.daily_quota</code>.
					</p>
				</div>

				<PolicyEditor bind:spec={editing.spec} disabled={saving} />
			</div>

			<div class="mt-6 flex justify-end gap-2">
				<button class="btn-secondary" onclick={close} disabled={saving}>Cancel</button>
				<button class="btn-primary" onclick={save} disabled={saving}>
					{saving ? 'Saving…' : editing.isNew ? 'Create' : 'Save'}
				</button>
			</div>
		</div>
	</div>
{/if}
