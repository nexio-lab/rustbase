<script lang="ts">
	import { page } from '$app/state';
	import {
		api,
		ApiError,
		type HookFile,
		type HookFileBody,
		type PutHookResponse,
		type ReloadOutcome
	} from '$lib/api';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);
	const apiBase = $derived(`/api/workspaces/${workspace}/apps/${app}/hooks`);

	let files = $state<HookFile[]>([]);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	let selected = $state<string | null>(null);
	let source = $state('');
	let originalSource = $state('');
	let fileLoading = $state(false);
	let fileError: string | null = $state(null);

	let saving = $state(false);
	let saveError: string | null = $state(null);
	let reloadOutcome = $state<ReloadOutcome | null>(null);

	// Inline "new file" form. Kept separate from the editor state so a
	// half-typed filename doesn't clobber the open buffer.
	let creating = $state(false);
	let newName = $state('');
	let createError: string | null = $state(null);

	let confirmingDelete = $state(false);

	const dirty = $derived(source !== originalSource);

	async function loadList() {
		loading = true;
		loadError = null;
		try {
			files = await api.get<HookFile[]>(apiBase);
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function openFile(name: string) {
		fileLoading = true;
		fileError = null;
		try {
			const body = await api.get<HookFileBody>(`${apiBase}/${encodeURIComponent(name)}`);
			selected = body.filename;
			source = body.source;
			originalSource = body.source;
			reloadOutcome = null;
			confirmingDelete = false;
		} catch (e) {
			fileError = e instanceof ApiError ? e.message : String(e);
		} finally {
			fileLoading = false;
		}
	}

	async function save() {
		if (!selected) return;
		saving = true;
		saveError = null;
		try {
			const resp = await api.put<PutHookResponse>(
				`${apiBase}/${encodeURIComponent(selected)}`,
				{ source }
			);
			originalSource = resp.file.source;
			reloadOutcome = resp.reload;
			// Refresh the size/mtime in the sidebar.
			files = files.map((f) =>
				f.filename === resp.file.filename
					? {
							filename: resp.file.filename,
							size: resp.file.size,
							updated_at: resp.file.updated_at
						}
					: f
			);
		} catch (e) {
			saveError = e instanceof ApiError ? e.message : String(e);
		} finally {
			saving = false;
		}
	}

	async function reload() {
		saving = true;
		saveError = null;
		try {
			reloadOutcome = await api.post<ReloadOutcome>(`${apiBase}/reload`, {});
		} catch (e) {
			saveError = e instanceof ApiError ? e.message : String(e);
		} finally {
			saving = false;
		}
	}

	function openCreate() {
		creating = true;
		newName = '';
		createError = null;
	}

	function cancelCreate() {
		creating = false;
		createError = null;
	}

	async function submitCreate(e: SubmitEvent) {
		e.preventDefault();
		const trimmed = newName.trim();
		if (!trimmed) {
			createError = 'filename is required';
			return;
		}
		// Seed an empty file by writing once. The handler validates the
		// filename server-side; surfacing a 400 here keeps the editor
		// from opening a name we can't actually save back to.
		createError = null;
		try {
			await api.put<PutHookResponse>(
				`${apiBase}/${encodeURIComponent(trimmed)}`,
				{ source: '' }
			);
			creating = false;
			await loadList();
			await openFile(trimmed);
		} catch (e) {
			createError = e instanceof ApiError ? e.message : String(e);
		}
	}

	async function confirmDelete() {
		if (!selected) return;
		confirmingDelete = false;
		try {
			const r = await api.delete<ReloadOutcome>(`${apiBase}/${encodeURIComponent(selected)}`);
			reloadOutcome = r;
			selected = null;
			source = '';
			originalSource = '';
			await loadList();
		} catch (e) {
			saveError = e instanceof ApiError ? e.message : String(e);
		}
	}

	function fmtBytes(n: number): string {
		if (n < 1024) return `${n} B`;
		if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
		return `${(n / 1024 / 1024).toFixed(1)} MB`;
	}

	$effect(() => {
		workspace;
		app;
		// Reset selection when the app changes; load the list fresh.
		selected = null;
		source = '';
		originalSource = '';
		reloadOutcome = null;
		loadList();
	});
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: 'Hooks' }
	]}
/>


<div class="mb-4">
	<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Hooks</h1>
	<p class="mt-1 text-sm text-slate-500">
		JS/TS source files run inside the embedded QuickJS sandbox. Save to write the file and
		trigger a reload; compile errors surface below the editor.
	</p>
</div>

<div class="grid grid-cols-1 gap-4 md:grid-cols-[16rem_1fr]">
	<!-- file list -->
	<aside class="rounded-lg border border-slate-200 bg-white shadow-sm">
		<div class="flex items-center justify-between border-b border-slate-200 px-3 py-2 text-xs uppercase tracking-wider text-slate-500">
			<span>Files</span>
			{#if !creating}
				<button class="text-orange-600 hover:underline" onclick={openCreate}>+ New</button>
			{/if}
		</div>
		{#if creating}
			<form onsubmit={submitCreate} class="space-y-2 border-b border-slate-200 px-3 py-2">
				<input
					class="input text-sm"
					bind:value={newName}
					placeholder="hooks.ts"
					autocomplete="off"
				/>
				{#if createError}
					<div class="text-xs text-red-600">{createError}</div>
				{/if}
				<div class="flex gap-1">
					<button type="submit" class="btn-primary text-xs">Create</button>
					<button type="button" class="btn-secondary text-xs" onclick={cancelCreate}>
						Cancel
					</button>
				</div>
				<p class="text-[10px] text-slate-500">
					Must end in <code>.js</code> or <code>.ts</code>.
				</p>
			</form>
		{/if}

		{#if loading}
			<p class="px-3 py-2 text-sm text-slate-500">Loading…</p>
		{:else if loadError}
			<p class="px-3 py-2 text-sm text-red-600">{loadError}</p>
		{:else if files.length === 0}
			<p class="px-3 py-3 text-sm text-slate-500">No hooks yet.</p>
		{:else}
			<ul class="divide-y divide-slate-100">
				{#each files as f}
					<li>
						<button
							class="flex w-full items-center justify-between px-3 py-2 text-left text-sm hover:bg-slate-50 {selected ===
							f.filename
								? 'bg-orange-50 font-medium text-orange-700'
								: 'text-slate-700'}"
							onclick={() => openFile(f.filename)}
						>
							<span class="font-mono">{f.filename}</span>
							<span class="text-[10px] text-slate-400">{fmtBytes(f.size)}</span>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</aside>

	<!-- editor -->
	<section class="rounded-lg border border-slate-200 bg-white shadow-sm">
		{#if !selected}
			<div class="p-8 text-center text-sm text-slate-500">
				{#if files.length === 0}
					<p>Click <strong>+ New</strong> to scaffold your first hook file.</p>
					<p class="mt-2 text-xs">
						Try
						<code>{`$app.onRecordAfterCreate("notes", r => $app.log("created", r.id));`}</code>
					</p>
				{:else}
					<p>Select a file on the left to start editing.</p>
				{/if}
			</div>
		{:else}
			<div class="flex items-center justify-between border-b border-slate-200 px-3 py-2">
				<div class="flex items-center gap-2">
					<span class="font-mono text-sm text-slate-900">{selected}</span>
					{#if dirty}
						<span class="text-xs text-amber-600">• unsaved</span>
					{/if}
				</div>
				<div class="flex gap-2">
					<button class="btn-secondary text-xs" onclick={reload} disabled={saving}>
						Reload
					</button>
					{#if confirmingDelete}
						<button class="btn-secondary text-xs" onclick={() => (confirmingDelete = false)}>
							Cancel
						</button>
						<button
							class="rounded border border-red-500 bg-red-500 px-2.5 py-1 text-xs font-medium text-white hover:bg-red-600"
							onclick={confirmDelete}
						>
							Confirm delete
						</button>
					{:else}
						<button
							class="btn-secondary text-xs"
							onclick={() => (confirmingDelete = true)}
							disabled={saving}
						>
							Delete
						</button>
					{/if}
					<button class="btn-primary text-xs" onclick={save} disabled={saving || !dirty}>
						{saving ? 'Saving…' : 'Save'}
					</button>
				</div>
			</div>

			{#if fileLoading}
				<p class="px-4 py-3 text-sm text-slate-500">Loading…</p>
			{:else if fileError}
				<div class="error-banner m-3">{fileError}</div>
			{:else}
				<textarea
					class="block min-h-[24rem] w-full resize-y border-0 bg-slate-900 p-3 font-mono text-sm text-slate-100 focus:outline-none"
					bind:value={source}
					spellcheck="false"
					autocapitalize="off"
					autocorrect="off"
				></textarea>
			{/if}

			{#if saveError}
				<div class="error-banner m-3">{saveError}</div>
			{/if}

			{#if reloadOutcome}
				<div class="border-t border-slate-200 px-3 py-2 text-xs">
					{#if reloadOutcome.errors.length === 0}
						<span class="text-emerald-700">
							Loaded {reloadOutcome.loaded} file{reloadOutcome.loaded === 1 ? '' : 's'}
							cleanly.
						</span>
					{:else}
						<div class="text-red-700">
							{reloadOutcome.errors.length}
							error{reloadOutcome.errors.length === 1 ? '' : 's'} during reload:
							<ul class="mt-1 list-disc space-y-0.5 pl-4 font-mono">
								{#each reloadOutcome.errors as e}
									<li>{e}</li>
								{/each}
							</ul>
						</div>
					{/if}
				</div>
			{/if}
		{/if}
	</section>
</div>
