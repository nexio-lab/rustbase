<script lang="ts">
	import { goto } from '$app/navigation';

	// The System scope only has a Policies tab in this branch; the
	// other future surfaces (audit, server-wide settings) plug in
	// here as additional tabs in follow-up branches. Sending root
	// straight through avoids a barren landing page.
	$effect(() => {
		goto('/system/policies', { replaceState: true });
	});
</script>
