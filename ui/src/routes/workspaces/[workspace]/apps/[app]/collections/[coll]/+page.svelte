<script lang="ts">
	import Skeleton from '$lib/Skeleton.svelte';
	import { page } from '$app/state';
	import { api, ApiError, type Collection, type Field, type FieldType } from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace!);
	const app = $derived(page.params.app!);
	const coll = $derived(page.params.coll!);

	// `collection` is the server's canonical snapshot. `draftFields`
	// is the user's working copy. Add field / drop / flip required-
	// or-unique all mutate `draftFields`; the schema diff is the
	// computed delta between the two. Nothing hits the server until
	// the user clicks Apply.
	let collection = $state<Collection | null>(null);
	let draftFields = $state<Field[]>([]);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	let busy = $state(false);
	let busyError: string | null = $state(null);

	// Add-field form state. Submitting appends to the draft.
	let adding = $state(false);
	let newName = $state('');
	let newKind = $state<FieldType['kind']>('text');
	let newRequired = $state(false);
	let newUnique = $state(false);
	let newRelationTarget = $state('');
	let addError: string | null = $state(null);

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
			const got = await api.get<Collection>(
				`/api/workspaces/${workspace}/apps/${app}/collections/${coll}`
			);
			collection = got;
			draftFields = [...got.schema.fields];
		} catch (e) {
			loadError = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		workspace;
		app;
		coll;
		load();
	});

	const savedFields = $derived(collection?.schema.fields ?? []);
	const savedByName = $derived(new Map<string, Field>(savedFields.map((f) => [f.name, f])));
	const draftByName = $derived(new Map<string, Field>(draftFields.map((f) => [f.name, f])));
	const added = $derived(draftFields.filter((f) => !savedByName.has(f.name)));
	const dropped = $derived(savedFields.filter((f) => !draftByName.has(f.name)));
	const modified = $derived(
		draftFields
			.filter((f) => savedByName.has(f.name))
			.filter((f) => !fieldEqual(savedByName.get(f.name)!, f))
	);
	const hasPending = $derived(added.length > 0 || dropped.length > 0 || modified.length > 0);

	function fieldEqual(a: Field, b: Field): boolean {
		if (a.name !== b.name || a.kind !== b.kind) return false;
		if (!!a.required !== !!b.required) return false;
		if (!!a.unique !== !!b.unique) return false;
		if (a.kind === 'relation' && b.kind === 'relation') {
			if (a.target !== b.target) return false;
			if (!!a.cascade_delete !== !!b.cascade_delete) return false;
		}
		const am = (a as { min?: number }).min;
		const bm = (b as { min?: number }).min;
		if (am !== bm) return false;
		const aM = (a as { max?: number }).max;
		const bM = (b as { max?: number }).max;
		if (aM !== bM) return false;
		return true;
	}

	function statusOf(name: string): 'unchanged' | 'added' | 'modified' | 'dropped' {
		if (!savedByName.has(name)) return 'added';
		if (!draftByName.has(name)) return 'dropped';
		if (modified.some((f) => f.name === name)) return 'modified';
		return 'unchanged';
	}

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
		addError = null;
	}

	function addFieldToDraft(e: SubmitEvent) {
		e.preventDefault();
		addError = null;
		const trimmed = newName.trim();
		if (!trimmed) {
			addError = 'Name is required.';
			return;
		}
		if (draftByName.has(trimmed)) {
			addError = `A field named "${trimmed}" already exists in the draft.`;
			return;
		}
		if (newKind === 'relation' && !newRelationTarget.trim()) {
			addError = 'Relation target is required.';
			return;
		}
		const field: Field = {
			name: trimmed,
			...fieldType(),
			required: newRequired || undefined,
			unique: newUnique || undefined
		};
		draftFields = [...draftFields, field];
		resetAddForm();
	}

	function dropFromDraft(f: Field) {
		draftFields = draftFields.filter((x) => x.name !== f.name);
	}

	function restoreSaved(f: Field) {
		// Insert at the saved field's original index, clamped to draft length.
		const savedIdx = savedFields.findIndex((x) => x.name === f.name);
		if (savedIdx < 0) return;
		const next = [...draftFields];
		next.splice(Math.min(savedIdx, next.length), 0, f);
		draftFields = next;
	}

	function toggleRequired(name: string) {
		draftFields = draftFields.map((f) =>
			f.name === name ? { ...f, required: !f.required || undefined } : f
		);
	}

	function toggleUnique(name: string) {
		draftFields = draftFields.map((f) =>
			f.name === name ? { ...f, unique: !f.unique || undefined } : f
		);
	}

	function discardDraft() {
		draftFields = [...savedFields];
		busyError = null;
		resetAddForm();
	}

	async function applyDraft() {
		if (!collection || !hasPending) return;
		const force = dropped.length > 0;
		busy = true;
		busyError = null;
		try {
			const updated = await api.patch<Collection>(
				`/api/workspaces/${workspace}/apps/${app}/collections/${coll}${force ? '?force=true' : ''}`,
				{
					schema: {
						id: collection.schema.id,
						kind: collection.schema.kind,
						fields: draftFields
					}
				}
			);
			collection = updated;
			draftFields = [...updated.schema.fields];
			resetAddForm();
		} catch (e) {
			busyError = e instanceof ApiError ? e.message : String(e);
		} finally {
			busy = false;
		}
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

	// Combine draft + dropped fields into a single render list so the
	// user sees their working state and every removal in one place.
	const renderRows = $derived(buildRenderRows(draftFields, dropped));

	type Row = { field: Field; status: ReturnType<typeof statusOf> };

	function buildRenderRows(draft: Field[], removed: Field[]): Row[] {
		const rows: Row[] = draft.map((f) => ({ field: f, status: statusOf(f.name) }));
		for (const f of removed) {
			rows.push({ field: f, status: 'dropped' });
		}
		return rows;
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: coll }
	]}
/>

{#if loading}
	<Skeleton rows={3} class="mt-4 space-y-2 max-w-md" />
{:else if loadError}
	<div class="error-banner">{loadError}</div>
{:else if collection}
	<div class="mb-6 flex items-end justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight text-slate-900 dark:text-slate-100">
				Collection <span class="font-mono">{collection.id}</span>
			</h1>
			<p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
				<span
					class="mr-2 inline-flex rounded-full px-2 py-0.5 text-xs font-medium {collection.kind ===
					'auth'
						? 'bg-violet-100 text-violet-800 dark:bg-violet-900/40 dark:text-violet-200'
						: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300'}"
				>
					{collection.kind}
				</span>
				{collection.schema.fields.length} field{collection.schema.fields.length === 1 ? '' : 's'} · updated
				{new Date(collection.updated_at).toLocaleString()}
			</p>
		</div>
		{#if !adding}
			<button class="btn-primary" onclick={() => (adding = true)} disabled={busy}>
				+ Add field
			</button>
		{/if}
	</div>

	{#if busyError}
		<div class="error-banner mb-4">{busyError}</div>
	{/if}

	<!-- Pending-changes banner. Sits above the table and the add-field form. -->
	{#if hasPending}
		<div
			class="mb-4 rounded-lg border border-orange-300 bg-orange-50 px-4 py-3 dark:border-orange-600 dark:bg-orange-950/30"
			role="region"
			aria-label="Pending schema changes"
		>
			<div class="flex items-center justify-between gap-4">
				<div class="text-sm">
					<strong class="text-slate-900 dark:text-slate-100">Pending schema changes</strong>
					<span class="ml-2 text-slate-700 dark:text-slate-300">
						{#if added.length}
							<span class="mr-3">
								<span class="inline-block h-2 w-2 rounded-full bg-emerald-500 align-middle"></span>
								{added.length} added
							</span>
						{/if}
						{#if modified.length}
							<span class="mr-3">
								<span class="inline-block h-2 w-2 rounded-full bg-amber-500 align-middle"></span>
								{modified.length} modified
							</span>
						{/if}
						{#if dropped.length}
							<span class="mr-3">
								<span class="inline-block h-2 w-2 rounded-full bg-red-500 align-middle"></span>
								{dropped.length} dropped
							</span>
						{/if}
					</span>
				</div>
				<div class="flex gap-2">
					<button class="btn-secondary" onclick={discardDraft} disabled={busy}> Discard </button>
					<button class="btn-primary" onclick={applyDraft} disabled={busy}>
						{busy ? 'Applying…' : `Apply ${dropped.length > 0 ? '(force)' : ''}`}
					</button>
				</div>
			</div>
			{#if dropped.length > 0}
				<p class="mt-2 text-xs text-slate-700 dark:text-slate-300">
					Dropping {dropped.length} field{dropped.length === 1 ? '' : 's'} removes the underlying SQLite
					column{dropped.length === 1 ? '' : 's'} and every value stored there. The PATCH is sent with
					<code>force=true</code>.
				</p>
			{/if}
		</div>
	{/if}

	{#if adding}
		<form
			onsubmit={addFieldToDraft}
			class="card mb-6 max-w-2xl space-y-4 border-slate-200 dark:border-slate-700"
		>
			<h2 class="text-lg font-semibold text-slate-900 dark:text-slate-100">Add field</h2>
			<p class="text-xs text-slate-500 dark:text-slate-400">
				Stages the field into the draft schema. No SQL runs until you click <strong>Apply</strong>.
			</p>
			{#if addError}
				<div class="error-banner">{addError}</div>
			{/if}
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
					/>
				</div>
				<div>
					<label class="field-label" for="fkind">Type</label>
					<select id="fkind" class="input" bind:value={newKind}>
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
						/>
					</div>
				{/if}
			</div>
			<div class="flex gap-4">
				<label class="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-300">
					<input type="checkbox" bind:checked={newRequired} />
					Required
				</label>
				<label class="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-300">
					<input type="checkbox" bind:checked={newUnique} />
					Unique
				</label>
			</div>
			<div class="flex gap-2">
				<button type="submit" class="btn-primary">Stage field</button>
				<button type="button" class="btn-secondary" onclick={resetAddForm}>Cancel</button>
			</div>
		</form>
	{/if}

	<section>
		<h2
			class="mb-3 text-sm font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400"
		>
			Fields
		</h2>
		<div
			class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm dark:border-slate-700 dark:bg-slate-900"
		>
			<table class="min-w-full divide-y divide-slate-200 text-sm dark:divide-slate-700">
				<thead
					class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500 dark:bg-slate-800 dark:text-slate-400"
				>
					<tr>
						<th class="w-8 px-3 py-2.5"></th>
						<th class="px-4 py-2.5 font-medium">Name</th>
						<th class="px-4 py-2.5 font-medium">Type</th>
						<th class="px-4 py-2.5 font-medium">Required</th>
						<th class="px-4 py-2.5 font-medium">Unique</th>
						<th class="px-4 py-2.5 font-medium">Detail</th>
						<th class="px-4 py-2.5"></th>
					</tr>
				</thead>
				<tbody class="divide-y divide-slate-200 bg-white dark:divide-slate-700 dark:bg-slate-900">
					{#if renderRows.length === 0}
						<tr>
							<td
								colspan="7"
								class="px-4 py-6 text-center text-sm text-slate-500 dark:text-slate-400"
							>
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
						{#each renderRows as row (row.field.name)}
							{@const f = row.field}
							{@const status = row.status}
							<tr
								class:bg-emerald-50={status === 'added'}
								class:bg-amber-50={status === 'modified'}
								class:bg-red-50={status === 'dropped'}
								class:dark:bg-emerald-950={status === 'added'}
								class:dark:bg-amber-950={status === 'modified'}
								class:dark:bg-red-950={status === 'dropped'}
							>
								<td class="px-3 py-2.5">
									<span
										class="inline-block h-2 w-2 rounded-full"
										class:bg-emerald-500={status === 'added'}
										class:bg-amber-500={status === 'modified'}
										class:bg-red-500={status === 'dropped'}
										class:bg-slate-300={status === 'unchanged'}
										aria-label={`Status: ${status}`}
										title={status}
									></span>
								</td>
								<td
									class="px-4 py-2.5 font-mono text-slate-900 dark:text-slate-100"
									class:line-through={status === 'dropped'}
								>
									{f.name}
								</td>
								<td class="px-4 py-2.5 text-slate-700 dark:text-slate-300">
									{KIND_LABEL[f.kind]}
								</td>
								<td class="px-4 py-2.5">
									{#if status === 'dropped'}
										<span class="text-xs text-slate-400">
											{f.required ? 'yes' : 'no'}
										</span>
									{:else}
										<input
											type="checkbox"
											class="h-4 w-4 cursor-pointer rounded border-slate-300 text-orange-600 focus:ring-orange-500"
											checked={!!f.required}
											onchange={() => toggleRequired(f.name)}
											disabled={busy}
											aria-label={`Toggle required for ${f.name}`}
										/>
									{/if}
								</td>
								<td class="px-4 py-2.5">
									{#if status === 'dropped'}
										<span class="text-xs text-slate-400">
											{f.unique ? 'yes' : 'no'}
										</span>
									{:else}
										<input
											type="checkbox"
											class="h-4 w-4 cursor-pointer rounded border-slate-300 text-orange-600 focus:ring-orange-500"
											checked={!!f.unique}
											onchange={() => toggleUnique(f.name)}
											disabled={busy}
											aria-label={`Toggle unique for ${f.name}`}
										/>
									{/if}
								</td>
								<td class="px-4 py-2.5 font-mono text-xs text-slate-500 dark:text-slate-400">
									{describe(f)}
								</td>
								<td class="px-4 py-2.5 text-right text-xs whitespace-nowrap">
									{#if status === 'dropped'}
										<button
											class="text-emerald-700 hover:text-emerald-900 dark:text-emerald-400 dark:hover:text-emerald-300"
											onclick={() => restoreSaved(f)}
											disabled={busy}
										>
											Restore
										</button>
									{:else if status === 'added'}
										<button
											class="text-slate-600 hover:text-slate-900 dark:text-slate-300 dark:hover:text-slate-100"
											onclick={() => dropFromDraft(f)}
											disabled={busy}
										>
											Remove
										</button>
									{:else}
										<button
											class="text-red-600 hover:text-red-800 dark:text-red-400 dark:hover:text-red-300"
											onclick={() => dropFromDraft(f)}
											disabled={busy}
										>
											Drop
										</button>
									{/if}
								</td>
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>
		<p class="mt-2 text-xs text-slate-500 dark:text-slate-400">
			Schema edits accumulate as a draft. Add, drop, restore, and flip flags freely — no SQL runs
			until you click <strong>Apply</strong>. Drops are force-deletes and remove the underlying
			column data; the server requires <code>force=true</code> when the diff contains any drop.
		</p>
	</section>
{/if}
