<script lang="ts">
	import { page } from '$app/state';
	import { api, ApiError, type FileMeta } from '$lib/api';
	import { auth } from '$lib/auth.svelte';
	import Breadcrumbs from '$lib/Breadcrumbs.svelte';

	const workspace = $derived(page.params.workspace);
	const app = $derived(page.params.app);
	const apiBase = $derived(`/api/workspaces/${workspace}/apps/${app}/files`);

	let files = $state<FileMeta[]>([]);
	let loading = $state(true);
	let loadError: string | null = $state(null);

	let uploading = $state(false);
	let uploadError: string | null = $state(null);
	let dragging = $state(false);
	// 0..1 — driven by XHR progress events. Falls back to indeterminate
	// when the browser can't measure (chunked, no Content-Length).
	let progress = $state(0);

	let confirmingId = $state<string | null>(null);

	async function load() {
		loading = true;
		loadError = null;
		try {
			files = await api.get<FileMeta[]>(apiBase);
		} catch (e) {
			loadError = e instanceof ApiError ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		workspace;
		app;
		load();
	});

	/**
	 * Upload via XHR rather than fetch — fetch's upload API doesn't
	 * report progress events yet (Streams Request body is still
	 * partial). XHR has had `upload.onprogress` forever.
	 */
	function uploadOne(file: File): Promise<FileMeta> {
		return new Promise((resolve, reject) => {
			const xhr = new XMLHttpRequest();
			xhr.open('POST', apiBase, true);
			// HttpOnly session cookies — let the browser attach them.
			xhr.withCredentials = true;
			xhr.setRequestHeader('x-filename', file.name);
			xhr.setRequestHeader('content-type', file.type || 'application/octet-stream');
			xhr.upload.onprogress = (e) => {
				if (e.lengthComputable) progress = e.loaded / e.total;
			};
			xhr.onload = () => {
				if (xhr.status >= 200 && xhr.status < 300) {
					try {
						resolve(JSON.parse(xhr.responseText) as FileMeta);
					} catch (err) {
						reject(new Error(`bad response: ${err}`));
					}
				} else {
					let msg = `${xhr.status} ${xhr.statusText}`;
					try {
						const body = JSON.parse(xhr.responseText);
						if (body && body.message) msg = body.message;
					} catch {
						/* keep status line */
					}
					reject(new Error(msg));
				}
			};
			xhr.onerror = () => reject(new Error('network error'));
			xhr.send(file);
		});
	}

	async function uploadAll(list: FileList | File[]) {
		const arr = Array.from(list);
		if (arr.length === 0) return;
		uploading = true;
		uploadError = null;
		progress = 0;
		try {
			const created: FileMeta[] = [];
			for (const f of arr) {
				created.push(await uploadOne(f));
				progress = 1;
			}
			// Prepend uploads in reverse order so the latest is on top.
			files = [...created.reverse(), ...files];
		} catch (e) {
			uploadError = e instanceof Error ? e.message : String(e);
		} finally {
			uploading = false;
			progress = 0;
		}
	}

	function onPick(e: Event) {
		const input = e.target as HTMLInputElement;
		if (input.files && input.files.length > 0) {
			uploadAll(input.files);
			input.value = '';
		}
	}

	function onDragOver(e: DragEvent) {
		e.preventDefault();
		dragging = true;
	}

	function onDragLeave() {
		dragging = false;
	}

	function onDrop(e: DragEvent) {
		e.preventDefault();
		dragging = false;
		if (e.dataTransfer?.files && e.dataTransfer.files.length > 0) {
			uploadAll(e.dataTransfer.files);
		}
	}

	function downloadHref(id: string): string {
		return `${apiBase}/${encodeURIComponent(id)}`;
	}

	/**
	 * The download endpoint is admin-only — the browser's anchor can't
	 * attach the Bearer header. Fetch the bytes ourselves, then save
	 * via an object URL so the request actually carries our token.
	 */
	async function download(f: FileMeta) {
		try {
			const resp = await fetch(downloadHref(f.id), {
				credentials: 'include'
			});
			if (!resp.ok) {
				uploadError = `download failed: ${resp.status} ${resp.statusText}`;
				return;
			}
			const blob = await resp.blob();
			const url = URL.createObjectURL(blob);
			const a = document.createElement('a');
			a.href = url;
			a.download = f.filename;
			document.body.appendChild(a);
			a.click();
			a.remove();
			URL.revokeObjectURL(url);
		} catch (e) {
			uploadError = e instanceof Error ? e.message : String(e);
		}
	}

	async function confirmDelete(id: string) {
		confirmingId = null;
		try {
			await api.delete(`${apiBase}/${encodeURIComponent(id)}`);
			files = files.filter((f) => f.id !== id);
		} catch (e) {
			uploadError = e instanceof ApiError ? e.message : String(e);
		}
	}

	function fmtBytes(n: number): string {
		if (n < 1024) return `${n} B`;
		if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
		if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
		return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
	}

	const totalBytes = $derived(files.reduce((s, f) => s + f.size, 0));
</script>

<Breadcrumbs
	items={[
		{ label: 'Workspaces', href: '/workspaces' },
		{ label: workspace, href: `/workspaces/${workspace}` },
		{ label: app, href: `/workspaces/${workspace}/apps/${app}` },
		{ label: 'Files' }
	]}
/>


<div class="mb-4 flex items-end justify-between">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight text-slate-900">Files</h1>
		<p class="mt-1 text-sm text-slate-500">
			{files.length} file{files.length === 1 ? '' : 's'} · {fmtBytes(totalBytes)} total
		</p>
	</div>
	<label class="btn-primary cursor-pointer">
		<input type="file" multiple class="hidden" onchange={onPick} disabled={uploading} />
		{uploading ? 'Uploading…' : '+ Upload'}
	</label>
</div>

<!-- drag-drop zone -->
<div
	role="presentation"
	class="mb-4 rounded-lg border-2 border-dashed p-6 text-center text-sm transition-colors {dragging
		? 'border-orange-500 bg-orange-50 text-orange-700'
		: 'border-slate-300 text-slate-500'}"
	ondragover={onDragOver}
	ondragleave={onDragLeave}
	ondrop={onDrop}
>
	{#if uploading}
		<p>Uploading…</p>
		<div class="mx-auto mt-2 h-1.5 w-64 overflow-hidden rounded bg-slate-200">
			<div
				class="h-full bg-orange-500 transition-all"
				style="width: {Math.round(progress * 100)}%"
			></div>
		</div>
	{:else}
		<p>Drag files anywhere on this box to upload, or click <strong>+ Upload</strong> above.</p>
	{/if}
</div>

{#if uploadError}
	<div class="error-banner mb-4">{uploadError}</div>
{/if}

{#if loading}
	<p class="text-sm text-slate-500">Loading…</p>
{:else if loadError}
	<div class="error-banner">{loadError}</div>
{:else if files.length === 0}
	<div class="card text-center text-slate-500">
		<p>No files yet.</p>
		<p class="mt-1 text-xs">
			Anything uploaded to this app shows up here. Records can reference files by id.
		</p>
	</div>
{:else}
	<div class="overflow-hidden rounded-lg border border-slate-200 bg-white shadow-sm">
		<table class="min-w-full divide-y divide-slate-200 text-sm">
			<thead class="bg-slate-50 text-left text-xs uppercase tracking-wider text-slate-500">
				<tr>
					<th class="px-4 py-2.5 font-medium">Filename</th>
					<th class="px-4 py-2.5 font-medium">Type</th>
					<th class="px-4 py-2.5 font-medium">Size</th>
					<th class="px-4 py-2.5 font-medium">Uploaded</th>
					<th class="px-4 py-2.5 font-medium">ID</th>
					<th class="px-4 py-2.5"></th>
				</tr>
			</thead>
			<tbody class="divide-y divide-slate-200 bg-white">
				{#each files as f}
					<tr class="hover:bg-slate-50">
						<td class="px-4 py-2 text-slate-900">{f.filename}</td>
						<td class="px-4 py-2 text-xs text-slate-500">{f.mime ?? '—'}</td>
						<td class="px-4 py-2 text-xs text-slate-700">{fmtBytes(f.size)}</td>
						<td class="px-4 py-2 text-xs text-slate-500">
							{new Date(f.created_at).toLocaleString()}
						</td>
						<td class="max-w-xs truncate px-4 py-2 font-mono text-[11px] text-slate-400">
							{f.id}
						</td>
						<td class="px-4 py-2 text-right">
							<button class="text-xs text-orange-600 hover:underline" onclick={() => download(f)}>
								Download
							</button>
							{#if confirmingId === f.id}
								<button
									class="ml-2 text-xs text-red-700 hover:underline"
									onclick={() => confirmDelete(f.id)}
								>
									Confirm
								</button>
								<button
									class="ml-1 text-xs text-slate-500 hover:underline"
									onclick={() => (confirmingId = null)}
								>
									Cancel
								</button>
							{:else}
								<button
									class="ml-2 text-xs text-red-600 hover:underline"
									onclick={() => (confirmingId = f.id)}
								>
									Delete
								</button>
							{/if}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
