<script lang="ts">
	import { goto } from '$app/navigation';

	// System root has no content of its own — bounce through to the
	// Policies tab so the user lands on something usable.
	$effect(() => {
		goto('/system/policies', { replaceState: true });
	});
</script>
