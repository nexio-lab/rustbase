<script lang="ts">
	import type { PolicySpec } from './api';

	let {
		spec = $bindable(),
		disabled = false
	}: {
		spec: PolicySpec;
		disabled?: boolean;
	} = $props();

	// EnumSet text input — comma-separated, edited as a raw string and
	// flushed back to `spec.allowed` whenever the user pauses typing.
	let enumRaw = $state(spec.kind === 'enum_set' ? spec.allowed.join(', ') : '');

	function changeKind(next: PolicySpec['kind']) {
		// Pick reasonable defaults per kind so the form is immediately
		// usable; the parent can override before save.
		if (next === 'range') spec = { kind: 'range', min: 0, max: 100 };
		else if (next === 'toggle') spec = { kind: 'toggle', state: 'open', default: false };
		else if (next === 'enum_set') {
			spec = { kind: 'enum_set', allowed: [] };
			enumRaw = '';
		} else if (next === 'free') spec = { kind: 'free' };
	}

	function updateEnumAllowed() {
		if (spec.kind !== 'enum_set') return;
		const items = enumRaw
			.split(',')
			.map((s) => s.trim())
			.filter((s) => s.length > 0);
		spec = { kind: 'enum_set', allowed: items };
	}

	function toggleStateChange(s: 'open' | 'locked') {
		if (spec.kind !== 'toggle') return;
		spec =
			s === 'open'
				? { kind: 'toggle', state: 'open', default: spec.state === 'open' ? spec.default : false }
				: { kind: 'toggle', state: 'locked', value: spec.state === 'locked' ? spec.value : false };
	}
</script>

<div class="space-y-3">
	<div>
		<label class="field-label" for="kind">Kind</label>
		<select
			id="kind"
			class="input"
			value={spec.kind}
			onchange={(e) => changeKind((e.target as HTMLSelectElement).value as PolicySpec['kind'])}
			{disabled}
		>
			<option value="range">range — numeric min/max</option>
			<option value="toggle">toggle — boolean, with optional lock</option>
			<option value="enum_set">enum_set — allowed string values</option>
			<option value="free">free — no constraints</option>
		</select>
	</div>

	{#if spec.kind === 'range'}
		<div class="grid grid-cols-2 gap-3">
			<div>
				<label class="field-label" for="min">min</label>
				<input
					id="min"
					type="number"
					class="input"
					value={spec.min}
					oninput={(e) => {
						if (spec.kind === 'range')
							spec = { ...spec, min: Number((e.target as HTMLInputElement).value) };
					}}
					{disabled}
				/>
			</div>
			<div>
				<label class="field-label" for="max">max</label>
				<input
					id="max"
					type="number"
					class="input"
					value={spec.max}
					oninput={(e) => {
						if (spec.kind === 'range')
							spec = { ...spec, max: Number((e.target as HTMLInputElement).value) };
					}}
					{disabled}
				/>
			</div>
		</div>
		<p class="text-xs text-slate-500">
			Children must pick a sub-range. The cascade auto-clamps existing values that fall outside.
		</p>
	{:else if spec.kind === 'toggle'}
		<div>
			<label class="field-label" for="state">state</label>
			<select
				id="state"
				class="input"
				value={spec.state}
				onchange={(e) =>
					toggleStateChange((e.target as HTMLSelectElement).value as 'open' | 'locked')}
				{disabled}
			>
				<option value="open">open — children may flip freely</option>
				<option value="locked">locked — children must use this value</option>
			</select>
		</div>
		<div>
			<label class="field-label" for="val">
				{spec.state === 'open' ? 'default' : 'value'}
			</label>
			<select
				id="val"
				class="input"
				value={String(spec.state === 'open' ? spec.default : spec.value)}
				onchange={(e) => {
					const v = (e.target as HTMLSelectElement).value === 'true';
					if (spec.kind === 'toggle' && spec.state === 'open') spec = { ...spec, default: v };
					else if (spec.kind === 'toggle' && spec.state === 'locked') spec = { ...spec, value: v };
				}}
				{disabled}
			>
				<option value="false">false</option>
				<option value="true">true</option>
			</select>
		</div>
	{:else if spec.kind === 'enum_set'}
		<div>
			<label class="field-label" for="allowed">allowed (comma-separated)</label>
			<input
				id="allowed"
				class="input"
				bind:value={enumRaw}
				oninput={updateEnumAllowed}
				onblur={updateEnumAllowed}
				placeholder="google, github, email"
				{disabled}
			/>
			{#if spec.allowed.length > 0}
				<div class="mt-2 flex flex-wrap gap-1">
					{#each spec.allowed as item}
						<span class="rounded-full bg-slate-100 px-2 py-0.5 text-xs text-slate-700">{item}</span>
					{/each}
				</div>
			{/if}
		</div>
	{:else}
		<p class="rounded-md border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-600">
			<strong>Free</strong> imposes no shape constraints. Used when a parent wants to record a default
			but not lock anything down for children.
		</p>
	{/if}
</div>
