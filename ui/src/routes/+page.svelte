<script lang="ts">
	import { goto } from "$lib/nav";
	import { auth } from '$lib/auth.svelte';

	// Root redirect. The layout-level guard already pushes
	// unauthenticated users to /login; authenticated users land on
	// /workspaces (master) or, eventually, on their workspace dashboard.
	$effect(() => {
		if (auth.isAuthenticated) goto('/workspaces', { replaceState: true });
		else goto('/login', { replaceState: true });
	});
</script>
