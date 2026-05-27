<script lang="ts">
	import { page } from '$app/state';
	import {
		api,
		ApiError,
		type Collection,
		type Field,
		type RecordListResponse,
		type RecordRow
	} from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const realm = $derived(page.params.realm);
	const app = $derived(page.params.app);
	const coll = $derived(page.params.coll);

	// Collection metadata (for the schema we render the editor against).
	let collection = $state<Collection | null>(null);

	// Records list state.
	let items = $state<RecordRow[]>([]);
	let total = $state(0);
	let totalPages = $state(0);
	let curPage = $state(1);
	const perPage = 30;

	let loading = $state(true);
	let loadError: string | null = $state(null);

	// Filter bar. The server runs this through `parse_filter` (nom),
	// which understands `field = "x"`, `&&`, `||`, `!`, comparisons,
	// `like`, `in (...)`. We keep the bar a plain text input so power
	// users can type whatever they want; bad syntax surfaces as a 400.
	let filter = $state('');
	let appliedFilter = $state('');

	// Editor (create OR edit).
	let editorOpen = $state(false);
	let editing = $state<RecordRow | null>(null); // null → create
	let formValues = $state<Record<string, string>>({}); // raw input strings
	let editorError: string | null = $state(null);
	let editorBusy = $state(false);

	async function loadCollection() {
		collection = await api.get<Collection>(
			`/api/realms/${realm}/apps/${app}/collections/${coll}`
		);
	}

	async function loadRecords() {
		loading = true;
		loadError = null;
		try {
			const q = new URLSearchParams();
			q.set('page', String(curPage));
			q.set('per_page', String(perPage));
			if (appliedFilter) q.set('filter', appliedFilter);
			const resp = await api.get<RecordListResponse>(
				`/api/realms/${realm}/apps/${app}/collections/${coll}/records?${q}`
			);
			items = resp.items;
			total = resp.total_items;
			totalPages = resp.total_pages;
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function loadAll() {
		try {
			if (!collection) await loadCollection();
			await loadRecords();
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
			loading = false;
		}
	}

	$effect(() => {
		realm;
		app;
		coll;
		// Reset to page 1 when the collection slug changes.
		curPage = 1;
		collection = null;
		loadAll();
	});

	function applyFilter(e: SubmitEvent) {
		e.preventDefault();
		appliedFilter = filter.trim();
		curPage = 1;
		loadRecords();
	}

	function clearFilter() {
		filter = '';
		appliedFilter = '';
		curPage = 1;
		loadRecords();
	}

	function gotoPage(p: number) {
		if (p < 1 || (totalPages > 0 && p > totalPages)) return;
		curPage = p;
		loadRecords();
	}

	function fieldDefaultString(f: Field): string {
		switch (f.kind) {
			case 'bool':
				return 'false';
			case 'number':
				return '';
			default:
				return '';
		}
	}

	function openCreate() {
		if (!collection) return;
		editing = null;
		formValues = Object.fromEntries(
			collection.schema.fields.map((f) => [f.name, fieldDefaultString(f)])
		);
		editorError = null;
		editorOpen = true;
	}

	function openEdit(row: RecordRow) {
		if (!collection) return;
		editing = row;
		formValues = Object.fromEntries(
			collection.schema.fields.map((f) => {
				const v = row.fields[f.name];
				if (v === undefined || v === null) return [f.name, ''];
				if (f.kind === 'bool') return [f.name, v ? 'true' : 'false'];
				if (f.kind === 'json' || f.kind === 'relation' || typeof v === 'object') {
					return [f.name, JSON.stringify(v)];
				}
				return [f.name, String(v)];
			})
		);
		editorError = null;
		editorOpen = true;
	}

	function closeEditor() {
		editorOpen = false;
		editing = null;
		editorError = null;
	}

	/** Parse a single raw input back into the JSON value the server wants. */
	function parseFieldValue(f: Field, raw: string): unknown {
		if (raw === '' && !f.required) return null;
		switch (f.kind) {
			case 'bool':
				return raw === 'true' || raw === 'on' || raw === '1';
			case 'number': {
				const n = Number(raw);
				if (Number.isNaN(n)) throw new Error(`${f.name}: not a number`);
				return n;
			}
			case 'json': {
				if (raw === '') return null;
				try {
					return JSON.parse(raw);
				} catch (e) {
					throw new Error(`${f.name}: invalid JSON (${(e as Error).message})`);
				}
			}
			default:
				return raw;
		}
	}

	async function submitEditor(e: SubmitEvent) {
		e.preventDefault();
		if (!collection) return;
		editorBusy = true;
		editorError = null;
		try {
			const body: Record<string, unknown> = {};
			for (const f of collection.schema.fields) {
				body[f.name] = parseFieldValue(f, formValues[f.name] ?? '');
			}
			if (editing) {
				await api.patch<RecordRow>(
					`/api/realms/${realm}/apps/${app}/collections/${coll}/records/${editing.id}`,
					body
				);
			} else {
				await api.post<RecordRow>(
					`/api/realms/${realm}/apps/${app}/collections/${coll}/records`,
					body
				);
			}
			closeEditor();
			await loadRecords();
		} catch (e) {
			editorError = e instanceof ApiError ? e.message : String(e);
		} finally {
			editorBusy = false;
		}
	}

	async function deleteRow(row: RecordRow) {
		if (!confirm(`Delete record ${row.id}?`)) return;
		try {
			await api.delete(
				`/api/realms/${realm}/apps/${app}/collections/${coll}/records/${row.id}`
			);
			await loadRecords();
		} catch (e) {
			alert(e instanceof ApiError ? e.message : String(e));
		}
	}

	/** Compact cell formatter for the table. */
	function cellText(v: unknown): string {
		if (v === null || v === undefined) return '—';
		if (typeof v === 'boolean') return v ? '✓' : '✗';
		if (typeof v === 'object') return JSON.stringify(v);
		const s = String(v);
		return s.length > 60 ? s.slice(0, 60) + '…' : s;
	}
</script>

<Breadcrumbs
	items={[
		{ label: 'Realms', href: '/realms' },
		{ label: realm, href: `/realms/${realm}` },
		{ label: app, href: `/realms/${realm}/apps/${app}` },
		{ label: coll, href: `/realms/${realm}/apps/${app}/collections/${coll}` }
	]}
/>

<div class="mb-2 flex gap-1 border-b border-slate-200 text-sm">
	<a
		href="/realms/{realm}/apps/{app}/collections/{coll}"
		class="border-b-2 border-transparent px-3 py-1.5 text-slate-500 hover:text-slate-700"
	>
		Schema
	</a>
	<span class="border-b-2 border-orange-500 px-3 py-1.5 font-medium text-slate-900">Records</span>
</div>

<div class="mb-4 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">
			Records in <span class="font-mono">{coll}</span>
		</h1>
		<p class="mt-1 text-sm text-slate-500">
			{total} record{total === 1 ? '' : 's'}{#if appliedFilter}
				· filtered by <code class="rounded bg-slate-100 px-1.5 py-0.5 text-xs">{appliedFilter}</code>
			{/if}
		</p>
	</div>
	<button class="btn-primary" onclick={openCreate} disabled={!collection}>+ New record</button>
</div>

<!-- Filter bar -->
<form onsubmit={applyFilter} class="mb-4 flex gap-2">
	<input
		class="input"
		bind:value={filter}
		placeholder={'e.g. title like "intro%" && pinned = true'}
	/>
	<button type="submit" class="btn-secondary">Apply</button>
	{#if appliedFilter}
		<button type="button" class="btn-secondary" onclick={clearFilter}>Clear</button>
	{/if}
</form>

{#if loadError}
	<div class="error-banner mb-4">{loadError}</div>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if !collection}
	<p class="text-sm text-slate-500">Collection metadata not loaded.</p>
{:else if items.length === 0}
	<div class="card text-center text-slate-500">
		<p>No records{appliedFilter ? ' match this filter' : ' yet'}.</p>
		{#if !appliedFilter}
			<p class="mt-1 text-xs">Click <strong>New record</strong> to add one.</p>
		{/if}
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">ID</th>
					{#each collection.schema.fields as f}
						<th class="px-4 py-2.5 font-medium">{f.name}</th>
					{/each}
					<th class="px-4 py-2.5 font-medium">Updated</th>
					<th class="px-4 py-2.5"></th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each items as row}
					<tr class="hover:bg-slate-50">
						<td class="px-4 py-2 font-mono text-xs text-slate-500" title={row.id}>
							{row.id.slice(0, 8)}…
						</td>
						{#each collection.schema.fields as f}
							<td class="max-w-xs truncate px-4 py-2 text-slate-700">
								{cellText(row.fields[f.name])}
							</td>
						{/each}
						<td class="px-4 py-2 text-xs text-slate-500">
							{new Date(row.updated_at).toLocaleString()}
						</td>
						<td class="px-4 py-2 text-right text-xs whitespace-nowrap">
							<button
								class="text-slate-600 hover:text-slate-900"
								onclick={() => openEdit(row)}>Edit</button
							>
							<span class="mx-1 text-slate-300">·</span>
							<button class="text-red-600 hover:text-red-800" onclick={() => deleteRow(row)}>
								Delete
							</button>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<!-- Pagination -->
	<div class="mt-3 flex items-center justify-between text-sm text-slate-600">
		<span>Page {curPage} of {totalPages}</span>
		<div class="flex gap-2">
			<button
				class="btn-secondary"
				onclick={() => gotoPage(curPage - 1)}
				disabled={curPage <= 1}
			>
				← Prev
			</button>
			<button
				class="btn-secondary"
				onclick={() => gotoPage(curPage + 1)}
				disabled={curPage >= totalPages}
			>
				Next →
			</button>
		</div>
	</div>
{/if}

<!-- Editor modal -->
{#if editorOpen && collection}
	<div
		class="fixed inset-0 z-10 flex items-start justify-center bg-slate-900/40 p-6"
		role="dialog"
		aria-modal="true"
	>
		<form
			onsubmit={submitEditor}
			class="mt-12 w-full max-w-xl rounded-lg border border-slate-200 bg-white p-6 shadow-xl"
		>
			<div class="mb-4 flex items-center justify-between">
				<h2 class="text-lg font-semibold text-slate-900">
					{editing ? 'Edit record' : 'New record'}
				</h2>
				<button
					type="button"
					onclick={closeEditor}
					aria-label="Close"
					class="text-slate-400 hover:text-slate-600"
				>
					✕
				</button>
			</div>

			{#if editorError}
				<div class="error-banner mb-4">{editorError}</div>
			{/if}

			{#if collection.schema.fields.length === 0}
				<p class="text-sm text-slate-500">
					This collection has no user fields. Add some on the
					<a
						href="/realms/{realm}/apps/{app}/collections/{coll}"
						class="font-medium text-orange-600 hover:text-orange-700">Schema</a
					> tab first.
				</p>
			{:else}
				<div class="max-h-[60vh] space-y-4 overflow-y-auto pr-1">
					{#each collection.schema.fields as f}
						<div>
							<label class="field-label" for={`f-${f.name}`}>
								{f.name}
								<span class="ml-1 text-xs font-normal text-slate-500">
									{f.kind}
									{#if f.required}<span class="text-orange-700">· required</span>{/if}
									{#if f.unique}<span class="text-emerald-700">· unique</span>{/if}
								</span>
							</label>
							{#if f.kind === 'bool'}
								<select
									id={`f-${f.name}`}
									class="input"
									bind:value={formValues[f.name]}
									disabled={editorBusy}
								>
									<option value="false">false</option>
									<option value="true">true</option>
								</select>
							{:else if f.kind === 'json' || f.kind === 'relation'}
								<textarea
									id={`f-${f.name}`}
									class="input font-mono text-xs"
									rows={f.kind === 'json' ? 4 : 1}
									bind:value={formValues[f.name]}
									placeholder={f.kind === 'relation' ? `"record-id"` : '{"any": "json"}'}
									disabled={editorBusy}
								></textarea>
							{:else}
								<input
									id={`f-${f.name}`}
									type={f.kind === 'email'
										? 'email'
										: f.kind === 'url'
											? 'url'
											: f.kind === 'number'
												? 'number'
												: f.kind === 'date'
													? 'datetime-local'
													: 'text'}
									class="input"
									bind:value={formValues[f.name]}
									required={f.required ?? false}
									disabled={editorBusy}
								/>
							{/if}
						</div>
					{/each}
				</div>
			{/if}

			<div class="mt-6 flex justify-end gap-2">
				<button
					type="button"
					class="btn-secondary"
					onclick={closeEditor}
					disabled={editorBusy}
				>
					Cancel
				</button>
				<button
					type="submit"
					class="btn-primary"
					disabled={editorBusy || collection.schema.fields.length === 0}
				>
					{editorBusy ? 'Saving…' : editing ? 'Save changes' : 'Create record'}
				</button>
			</div>
		</form>
	</div>
{/if}
