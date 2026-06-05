<script lang="ts">
	import Skeleton from '$lib/Skeleton.svelte';
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

	const workspace = $derived(page.params.workspace);
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

	// Bulk selection. IDs of every selected row on the current page;
	// pagination / filter / scope changes clear the set because rows
	// outside the visible page have no checkbox to communicate state.
	let selected = $state<Set<string>>(new Set());
	let bulkBusy = $state(false);
	let bulkError: string | null = $state(null);

	const allOnPageSelected = $derived(
		items.length > 0 && items.every((r) => selected.has(r.id))
	);
	const someOnPageSelected = $derived(
		!allOnPageSelected && items.some((r) => selected.has(r.id))
	);

	function toggleRow(id: string, on: boolean) {
		const next = new Set(selected);
		if (on) next.add(id);
		else next.delete(id);
		selected = next;
	}

	function toggleAllOnPage() {
		const next = new Set(selected);
		if (allOnPageSelected) {
			for (const r of items) next.delete(r.id);
		} else {
			for (const r of items) next.add(r.id);
		}
		selected = next;
	}

	function clearSelection() {
		selected = new Set();
		bulkError = null;
	}

	async function loadCollection() {
		collection = await api.get<Collection>(
			`/api/workspaces/${workspace}/apps/${app}/collections/${coll}`
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
				`/api/workspaces/${workspace}/apps/${app}/collections/${coll}/records?${q}`
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
		workspace;
		app;
		coll;
		// Reset to page 1 when the collection slug changes.
		curPage = 1;
		collection = null;
		clearSelection();
		loadAll();
	});

	function applyFilter(e: SubmitEvent) {
		e.preventDefault();
		appliedFilter = filter.trim();
		curPage = 1;
		clearSelection();
		loadRecords();
	}

	function clearFilter() {
		filter = '';
		appliedFilter = '';
		curPage = 1;
		clearSelection();
		loadRecords();
	}

	function gotoPage(p: number) {
		if (p < 1 || (totalPages > 0 && p > totalPages)) return;
		curPage = p;
		clearSelection();
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
				// Optimistic update: swap the row into the table while the
				// PATCH is in flight. On error, restore the original row
				// and surface the message inside the modal.
				const original = editing;
				const optimistic: RecordRow = {
					...original,
					fields: { ...original.fields, ...body },
					updated_at: new Date().toISOString()
				};
				items = items.map((r) => (r.id === original.id ? optimistic : r));
				try {
					const server = await api.patch<RecordRow>(
						`/api/workspaces/${workspace}/apps/${app}/collections/${coll}/records/${original.id}`,
						body
					);
					// Replace the optimistic row with the server's authoritative copy.
					items = items.map((r) => (r.id === server.id ? server : r));
					closeEditor();
				} catch (e) {
					items = items.map((r) => (r.id === original.id ? original : r));
					throw e;
				}
			} else {
				await api.post<RecordRow>(
					`/api/workspaces/${workspace}/apps/${app}/collections/${coll}/records`,
					body
				);
				closeEditor();
				await loadRecords();
			}
		} catch (e) {
			editorError = e instanceof ApiError ? e.message : String(e);
		} finally {
			editorBusy = false;
		}
	}

	async function deleteRow(row: RecordRow) {
		if (!confirm(`Delete record ${row.id}?`)) return;
		// Optimistic delete: drop the row immediately, snapshot for rollback.
		const prevItems = items;
		const prevTotal = total;
		items = items.filter((r) => r.id !== row.id);
		total = Math.max(0, total - 1);
		if (selected.has(row.id)) {
			const next = new Set(selected);
			next.delete(row.id);
			selected = next;
		}
		try {
			await api.delete(
				`/api/workspaces/${workspace}/apps/${app}/collections/${coll}/records/${row.id}`
			);
		} catch (e) {
			items = prevItems;
			total = prevTotal;
			alert(e instanceof ApiError ? e.message : String(e));
		}
	}

	async function bulkDelete() {
		if (selected.size === 0) return;
		const ids = Array.from(selected);
		const noun = ids.length === 1 ? 'record' : 'records';
		if (!confirm(`Delete ${ids.length} ${noun}?\n\nThis is irreversible.`)) return;
		bulkBusy = true;
		bulkError = null;
		const prevItems = items;
		const prevTotal = total;
		// Optimistically drop every selected row from the visible list.
		const toDelete = new Set(ids);
		items = items.filter((r) => !toDelete.has(r.id));
		total = Math.max(0, total - ids.length);
		const results = await Promise.allSettled(
			ids.map((id) =>
				api.delete(
					`/api/workspaces/${workspace}/apps/${app}/collections/${coll}/records/${id}`
				)
			)
		);
		const failed: string[] = [];
		for (let i = 0; i < results.length; i++) {
			if (results[i].status === 'rejected') failed.push(ids[i]);
		}
		if (failed.length === 0) {
			selected = new Set();
		} else if (failed.length === ids.length) {
			// Nothing actually deleted — restore everything verbatim.
			items = prevItems;
			total = prevTotal;
			selected = new Set(failed);
			bulkError = `Failed to delete ${failed.length} ${noun}.`;
		} else {
			// Partial — let the server tell us the real state.
			selected = new Set(failed);
			bulkError = `Failed to delete ${failed.length} of ${ids.length} ${noun}.`;
			await loadRecords();
		}
		bulkBusy = false;
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
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: coll, href: `/workspaces/${workspace}/apps/${app}/collections/${coll}` }
	]}
/>


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

{#if bulkError}
	<div class="error-banner mb-4">{bulkError}</div>
{/if}

{#if selected.size > 0}
	<div
		class="mb-3 flex items-center justify-between rounded-lg border border-orange-300 bg-orange-50 px-4 py-2 text-sm dark:border-orange-600 dark:bg-orange-950/30"
		role="region"
		aria-label="Bulk actions"
	>
		<span class="font-medium text-slate-900 dark:text-slate-100">
			{selected.size} selected
		</span>
		<div class="flex gap-2">
			<button
				class="btn-secondary border-red-300 text-red-700 hover:bg-red-50 dark:border-red-600 dark:text-red-300 dark:hover:bg-red-950/40"
				onclick={bulkDelete}
				disabled={bulkBusy}
			>
				{bulkBusy ? 'Deleting…' : `Delete ${selected.size}`}
			</button>
			<button class="btn-secondary" onclick={clearSelection} disabled={bulkBusy}>
				Clear
			</button>
		</div>
	</div>
{/if}

{#if loading}
	<Skeleton rows={3} class="mt-4 space-y-2 max-w-md" />
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
					<th class="w-10 px-4 py-2.5 font-medium">
						<input
							type="checkbox"
							class="h-4 w-4 cursor-pointer rounded border-slate-300 text-orange-600 focus:ring-orange-500"
							checked={allOnPageSelected}
							indeterminate={someOnPageSelected}
							onchange={toggleAllOnPage}
							aria-label="Select all records on this page"
						/>
					</th>
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
					<tr class="hover:bg-slate-50" class:bg-orange-50={selected.has(row.id)}>
						<td class="px-4 py-2">
							<input
								type="checkbox"
								class="h-4 w-4 cursor-pointer rounded border-slate-300 text-orange-600 focus:ring-orange-500"
								checked={selected.has(row.id)}
								onchange={(e) => toggleRow(row.id, e.currentTarget.checked)}
								aria-label={`Select record ${row.id.slice(0, 8)}`}
							/>
						</td>
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
						href="/workspaces/{workspace}/apps/{app}/collections/{coll}"
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
