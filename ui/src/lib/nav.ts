import { goto as svelteKitGoto } from '$app/navigation';
import { base } from '$app/paths';

/**
 * `goto` wrapper that prefixes SvelteKit's configured `paths.base` to
 * absolute-path hrefs.
 *
 * The embedded dashboard mounts at `/_/`, so a raw `goto('/login')` would
 * navigate to `http://host/login` (404 on the API server). This helper
 * rewrites it to `${base}/login` so the navigation stays under the mount.
 *
 * External URLs and relative hrefs are passed through untouched.
 */
export function goto(href: string, opts?: Parameters<typeof svelteKitGoto>[1]) {
	const target = href.startsWith('/') ? `${base}${href}` : href;
	return svelteKitGoto(target, opts);
}
