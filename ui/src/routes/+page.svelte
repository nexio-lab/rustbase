<script lang="ts">
	import { goto } from "$lib/nav";
	import { auth } from '$lib/auth.svelte';

	// Root redirect. The layout-level guard already pushes
	// unauthenticated users to /login; authenticated users land on
	// /realms (master) or, eventually, on their realm dashboard.
	$effect(() => {
		if (auth.isAuthenticated) goto('/realms', { replaceState: true });
		else goto('/login', { replaceState: true });
	});
</script>
