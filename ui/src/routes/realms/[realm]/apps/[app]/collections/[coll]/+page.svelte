<script lang="ts">
	import { page } from '$app/state';
	import {
		api,
		ApiError,
		type Collection,
		type Field,
		type FieldType
	} from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);
	const app = $derived(page.params.app);
	const coll = $derived(page.params.coll);

	let collection = $state<Collection | null>(null);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	let busy = $state(false);
	let busyError: string | null = $state(null);

	// Add-field form state.
	let adding = $state(false);
	let newName = $state('');
	let newKind = $state<FieldType['kind']>('text');
	let newRequired = $state(false);
	let newUnique = $state(false);
	let newRelationTarget = $state('');

	const KIND_LABEL: Record<FieldType['kind'], string> = {
		text: 'Text',
		number: 'Number',
		bool: 'Boolean',
		email: 'Email',
		url: 'URL',
		date: 'Date',
		json: 'JSON',
		relation: 'Relation',
		file: 'File'
	};

	async function load() {
		loading = true;
		loadError = null;
		try {
			collection = await api.get<Collection>(
				`/api/realms/${realm}/apps/${app}/collections/${coll}`
			);
		} catch (e) {
			loadError = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		realm;
		app;
		coll;
		load();
	});

	function fieldType(): FieldType {
		switch (newKind) {
			case 'relation':
				return { kind: 'relation', target: newRelationTarget };
			default:
				return { kind: newKind } as FieldType;
		}
	}

	function resetAddForm() {
		adding = false;
		newName = '';
		newKind = 'text';
		newRequired = false;
		newUnique = false;
		newRelationTarget = '';
	}

	/**
	 * Save a new schema for the collection. The server's PATCH endpoint
	 * runs `patch_collection` which only allows additive + drop changes
	 * (no in-place type swaps); dropping a field requires `force=true`.
	 * We surface 409s back to the user as-is.
	 */
	async function patchSchema(nextFields: Field[], force = false) {
		if (!collection) return;
		busy = true;
		busyError = null;
		try {
			const updated = await api.patch<Collection>(
				`/api/realms/${realm}/apps/${app}/collections/${coll}${force ? '?force=true' : ''}`,
				{
					schema: {
						id: collection.schema.id,
						kind: collection.schema.kind,
						fields: nextFields
					}
				}
			);
			collection = updated;
			resetAddForm();
		} catch (e) {
			busyError = e instanceof ApiError ? e.message : String(e);
		} finally {
			busy = false;
		}
	}

	async function addField(e: SubmitEvent) {
		e.preventDefault();
		if (!collection) return;
		const field: Field = {
			name: newName,
			...fieldType(),
			required: newRequired || undefined,
			unique: newUnique || undefined
		};
		await patchSchema([...collection.schema.fields, field]);
	}

	async function dropField(f: Field) {
		if (!collection) return;
		const ok = confirm(
			`Drop field "${f.name}"?\n\nThis is a force-delete. Existing data in this column will be lost.`
		);
		if (!ok) return;
		await patchSchema(
			collection.schema.fields.filter((x) => x.name !== f.name),
			true
		);
	}

	function describe(f: Field): string {
		const bits: string[] = [];
		if (f.kind === 'text' || f.kind === 'number') {
			if (f.min !== undefined) bits.push(`min=${f.min}`);
			if (f.max !== undefined) bits.push(`max=${f.max}`);
		} else if (f.kind === 'relation') {
			bits.push(`→ ${f.target}`);
			if (f.cascade_delete) bits.push('cascade');
		} else if (f.kind === 'file') {
			if (f.max_size !== undefined) bits.push(`≤${f.max_size}B`);
			if (f.mime_types?.length) bits.push(f.mime_types.join(','));
		}
		return bits.join(' ');
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: app, href: `/realms/${realm}/apps/${app}` },
		{ label: coll }
	]}
/>

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if loadError}
	<div class="error-banner">{loadError}</div>
{:else if collection}
	<div class="mb-6 flex items-end justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
				Collection <span class="font-mono">{collection.id}</span>
			</h1>
			<p class="mt-1 text-sm text-slate-500">
				<span
					class="mr-2 inline-flex rounded-full px-2 py-0.5 text-xs font-medium {collection.kind ===
					'auth'
						? 'bg-violet-100 text-violet-800'
						: 'bg-slate-100 text-slate-700'}"
				>
					{collection.kind}
				</span>
				{collection.schema.fields.length} field{collection.schema.fields.length === 1 ? '' : 's'} ·
				updated {new Date(collection.updated_at).toLocaleString()}
			</p>
		</div>
		{#if !adding}
			<button class="btn-primary" onclick={() => (adding = true)}>+ Add field</button>
		{/if}
	</div>

	{#if busyError}
		<div class="error-banner mb-4">{busyError}</div>
	{/if}

	{#if adding}
		<form onsubmit={addField} class="card mb-6 max-w-2xl space-y-4">
			<h2 class="text-lg font-semibold text-slate-900">Add field</h2>
			<div class="grid grid-cols-2 gap-4">
				<div>
					<label class="field-label" for="fname">Name</label>
					<input
						id="fname"
						class="input"
						bind:value={newName}
						placeholder="title"
						pattern="[a-z_][a-z0-9_]*"
						required
						disabled={busy}
					/>
				</div>
				<div>
					<label class="field-label" for="fkind">Type</label>
					<select id="fkind" class="input" bind:value={newKind} disabled={busy}>
						{#each Object.entries(KIND_LABEL) as [k, label]}
							<option value={k}>{label}</option>
						{/each}
					</select>
				</div>
				{#if newKind === 'relation'}
					<div class="col-span-2">
						<label class="field-label" for="ftarget">Target collection</label>
						<input
							id="ftarget"
							class="input"
							bind:value={newRelationTarget}
							placeholder="users"
							required
							disabled={busy}
						/>
					</div>
				{/if}
			</div>
			<div class="flex gap-4">
				<label class="flex items-center gap-2 text-sm text-slate-700">
					<input type="checkbox" bind:checked={newRequired} disabled={busy} />
					Required
				</label>
				<label class="flex items-center gap-2 text-sm text-slate-700">
					<input type="checkbox" bind:checked={newUnique} disabled={busy} />
					Unique
				</label>
			</div>
			<div class="flex gap-2">
				<button type="submit" class="btn-primary" disabled={busy}>
					{busy ? 'Saving…' : 'Add field'}
				</button>
				<button
					type="button"
					class="btn-secondary"
					onclick={resetAddForm}
					disabled={busy}
				>
					Cancel
				</button>
			</div>
		</form>
	{/if}

	<section>
		<h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500">Fields</h2>
		<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
			<table class="min-w-full divide-y divide-slate-200 text-sm">
				<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
					<tr>
						<th class="px-4 py-2.5 font-medium">Name</th>
						<th class="px-4 py-2.5 font-medium">Type</th>
						<th class="px-4 py-2.5 font-medium">Constraints</th>
						<th class="px-4 py-2.5 font-medium">Detail</th>
						<th class="px-4 py-2.5"></th>
					</tr>
				</thead>
				<tbody class="divide-y divide-slate-200 bg-white">
					{#if collection.schema.fields.length === 0}
						<tr>
							<td colspan="5" class="px-4 py-6 text-center text-sm text-slate-500">
								No user-defined fields yet.
								{#if collection.kind === 'auth'}
									<p class="mt-1 text-xs">
										(Auth collections still carry the built-in <code>email</code>,
										<code>password_hash</code>, <code>verified</code> columns.)
									</p>
								{/if}
							</td>
						</tr>
					{:else}
						{#each collection.schema.fields as f}
							<tr>
								<td class="px-4 py-2.5 font-mono text-slate-900">{f.name}</td>
								<td class="px-4 py-2.5 text-slate-700">{KIND_LABEL[f.kind]}</td>
								<td class="px-4 py-2.5 text-slate-700">
									{#if f.required}
										<span class="mr-1 rounded bg-orange-50 px-1.5 py-0.5 text-xs text-orange-800">required</span>
									{/if}
									{#if f.unique}
										<span class="rounded bg-emerald-50 px-1.5 py-0.5 text-xs text-emerald-800">unique</span>
									{/if}
								</td>
								<td class="px-4 py-2.5 font-mono text-xs text-slate-500">{describe(f)}</td>
								<td class="px-4 py-2.5 text-right">
									<button
										class="text-xs text-red-600 hover:text-red-800"
										onclick={() => dropField(f)}
										disabled={busy}
									>
										Drop
									</button>
								</td>
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>
		<p class="mt-2 text-xs text-slate-500">
			Schema evolution: adding fields is online and non-blocking. Dropping a field is a
			force-delete — existing data in that column is removed.
		</p>
	</section>
{/if}
