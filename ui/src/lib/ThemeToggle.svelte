<script lang="ts">
	import { theme, type ThemeChoice } from '$lib/theme.svelte';

	/** Three-state cycle button: Auto → Light → Dark → Auto. The
	 *  icon reflects the *choice* (not the resolved theme), so
	 *  a user who picked "Light" always sees the sun even on an OS
	 *  that's set to dark. Auto shows a compass. */
	function label(c: ThemeChoice): string {
		switch (c) {
			case 'light':
				return 'Theme: light';
			case 'dark':
				return 'Theme: dark';
			case 'auto':
				return 'Theme: auto (follow OS)';
		}
	}

	function icon(c: ThemeChoice): string {
		switch (c) {
			case 'light':
				return '☀';
			case 'dark':
				return '☾';
			case 'auto':
				return '◐';
		}
	}
</script>

<button
	onclick={() => theme.cycle()}
	class="nav-link inline-flex items-center justify-center gap-1 px-2 text-base leading-none"
	aria-label={label(theme.choice)}
	title={label(theme.choice)}
	type="button"
>
	<span aria-hidden="true">{icon(theme.choice)}</span>
</button>
