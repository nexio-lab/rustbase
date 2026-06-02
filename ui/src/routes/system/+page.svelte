<script lang="ts">
	import { goto } from "$lib/nav";

	// System root has no content of its own — bounce through to the
	// Policies tab so the user lands on something usable.
	$effect(() => {
		goto('/system/policies', { replaceState: true });
	});
</script>
