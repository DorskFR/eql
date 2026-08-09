<script lang="ts">
	import { page } from '$app/state';
	import {
		Badge,
		Button,
		Card,
		Cluster,
		CopyButton,
		EmptyState,
		Field,
		Heading,
		Input,
		Spinner,
		Stack,
		Text
	} from '@dorsk/tsumikit';
	import { bundleUrl, endpoints, type LayoutStyle, type Rect } from '$lib/api';
	import { clamp, GRID, snap, tokenStore, validate } from '$lib/layout';
	import { useLayout } from '$lib/queries';

	const name = $derived(page.params.name ?? '');
	const query = useLayout(() => name);

	const CANVAS_W = 960;

	let style = $state<LayoutStyle>({});
	let section = $state('');
	let layout = $state<Record<string, Rect>>({});
	let screenW = $state(3840);
	let screenH = $state(2160);
	let selected = $state<string | null>(null);
	let loaded = $state('');
	let token = $state(tokenStore.load());
	let skin = $state('');
	let saving = $state(false);
	let message = $state('');
	let failed = $state(false);

	$effect(() => {
		const data = query.data;
		if (!data || loaded === data.updated_at) return;
		loaded = data.updated_at;
		layout = structuredClone(data.layout);
		style = structuredClone(data.style ?? {});
		screenW = data.screen_w;
		screenH = data.screen_h;
		skin ||= data.name;
		selected ??= Object.keys(data.layout)[0] ?? null;
	});

	const scale = $derived(CANVAS_W / screenW);
	const canvasH = $derived(Math.round(screenH * scale));
	const windows = $derived(Object.entries(layout).sort(([a], [b]) => a.localeCompare(b)));
	const hidden = $derived(style.hidden ?? []);
	const bare = $derived(style.bare ?? []);
	const problems = $derived(validate(layout, screenW, screenH, hidden));
	const current = $derived(selected ? layout[selected] : undefined);
	const extras = $derived(hidden.filter((window) => !(window in layout)));

	function toggle(key: 'hidden' | 'bare', window: string) {
		const list = style[key] ?? [];
		style = {
			...style,
			[key]: list.includes(window)
				? list.filter((name) => name !== window)
				: [...list, window].sort()
		};
	}

	function addSection() {
		const name = section.trim();
		if (!name || hidden.includes(name)) return;
		style = { ...style, hidden: [...hidden, name].sort() };
		section = '';
	}

	let drag = $state<{ name: string; mode: 'move' | 'resize'; x: number; y: number; from: Rect } | null>(
		null
	);

	function begin(event: PointerEvent, window: string, mode: 'move' | 'resize') {
		event.preventDefault();
		event.stopPropagation();
		selected = window;
		drag = { name: window, mode, x: event.clientX, y: event.clientY, from: [...layout[window]] };
		(event.currentTarget as Element).setPointerCapture(event.pointerId);
	}

	function move(event: PointerEvent) {
		if (!drag) return;
		const dx = snap((event.clientX - drag.x) / scale);
		const dy = snap((event.clientY - drag.y) / scale);
		const [x, y, w, h] = drag.from;
		const next: Rect =
			drag.mode === 'move' ? [x + dx, y + dy, w, h] : [x, y, w + dx, h + dy];
		layout = { ...layout, [drag.name]: clamp(next, screenW, screenH) };
	}

	function end() {
		drag = null;
	}

	function nudge(event: KeyboardEvent) {
		if (!selected || !current) return;
		const target = event.target as HTMLElement | null;
		if (target && ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)) return;
		const step = event.shiftKey ? GRID * 10 : GRID;
		const deltas: Record<string, [number, number]> = {
			ArrowLeft: [-step, 0],
			ArrowRight: [step, 0],
			ArrowUp: [0, -step],
			ArrowDown: [0, step]
		};
		const delta = deltas[event.key];
		if (!delta) return;
		event.preventDefault();
		const [x, y, w, h] = current;
		layout = {
			...layout,
			[selected]: clamp([x + delta[0], y + delta[1], w, h], screenW, screenH)
		};
	}

	function setField(index: 0 | 1 | 2 | 3, raw: string) {
		if (!selected || !current) return;
		const value = Number(raw);
		if (!Number.isFinite(value)) return;
		const next = [...current] as Rect;
		next[index] = Math.round(value);
		layout = { ...layout, [selected]: clamp(next, screenW, screenH) };
	}

	async function save() {
		saving = true;
		failed = false;
		message = '';
		try {
			tokenStore.save(token);
			const saved = await endpoints.saveLayout(token, name, {
				screen_w: screenW,
				screen_h: screenH,
				layout,
				style
			});
			loaded = saved.updated_at;
			message = saved.problems.length
				? `Saved with ${saved.problems.length} problem(s)`
				: 'Saved';
		} catch (cause) {
			failed = true;
			message = cause instanceof Error ? cause.message : String(cause);
		} finally {
			saving = false;
		}
	}

	const tone = (window: string) =>
		problems.some((problem) => problem.startsWith(`${window} `) || problem.endsWith(window))
			? 'var(--danger)'
			: 'var(--accent)';
</script>

<svelte:window on:pointermove={move} on:pointerup={end} on:keydown={nudge} />

<Stack gap="var(--sp-4)">
	<Cluster justify="space-between">
		<Cluster gap="var(--sp-3)">
			<Button href="/layouts" variant="ghost" size="sm">Back</Button>
			<Heading level={2}>{name}</Heading>
		</Cluster>
		{#if query.isFetching}<Spinner label="Refreshing" />{/if}
	</Cluster>

	{#if query.isPending}
		<Card>
			<Cluster gap="var(--sp-2)"><Spinner /><Text tone="muted">Loading layout…</Text></Cluster>
		</Card>
	{:else if query.isError}
		<EmptyState
			title="Could not load this layout"
			description={query.error.message}
			icon="alert-circle"
			tone="danger"
			actionLabel="Retry"
			onAction={() => query.refetch()}
		/>
	{:else}
		<Card>
			<Cluster justify="space-between" gap="var(--sp-3)">
				<Cluster gap="var(--sp-2)">
					<Field label="Skin name" for="skin-name">
						<Input id="skin-name" bind:value={skin} mono size="sm" />
					</Field>
					<Field
						label="Machine token"
						for="editor-token"
						hint="Stopgap: kept in this browser only."
					>
						<Input id="editor-token" type="password" bind:value={token} mono size="sm" />
					</Field>
				</Cluster>
				<Cluster gap="var(--sp-2)">
					<Button variant="primary" disabled={saving || !token} onclick={save}>
						{saving ? 'Saving…' : 'Save'}
					</Button>
					<Button as="a" href={bundleUrl(name, skin)}>Download bundle</Button>
					<CopyButton text={`/loadskin ${skin}`} label="Copy /loadskin" />
				</Cluster>
			</Cluster>
			{#if message}
				<Text tone={failed ? 'danger' : 'muted'}>{message}</Text>
			{/if}
		</Card>

		<Cluster gap="var(--sp-4)" align="flex-start">
			<Card padding="none">
				<!-- The one raw-markup surface: a drag/resize canvas has no tsumikit equivalent. -->
				<svg
					class="canvas"
					width={CANVAS_W}
					height={canvasH}
					viewBox="0 0 {screenW} {screenH}"
					aria-label="Layout canvas"
				>
					<rect x="0" y="0" width={screenW} height={screenH} class="screen" />
					{#each windows as [window, rect] (window)}
						<g class:selected={selected === window} class:hidden={hidden.includes(window)}>
							<rect
								x={rect[0]}
								y={rect[1]}
								width={rect[2]}
								height={rect[3]}
								class="window"
								style="--tone: {tone(window)}"
								role="button"
								tabindex="-1"
								aria-label={window}
								onpointerdown={(event) => begin(event, window, 'move')}
							/>
							<text x={rect[0] + 12} y={rect[1] + 44} class="label">{window}</text>
							<rect
								x={rect[0] + rect[2] - 36}
								y={rect[1] + rect[3] - 36}
								width="36"
								height="36"
								class="handle"
								role="button"
								tabindex="-1"
								aria-label="{window} resize"
								onpointerdown={(event) => begin(event, window, 'resize')}
							/>
						</g>
					{/each}
				</svg>
			</Card>

			<Stack gap="var(--sp-3)" style="min-width: 18rem">
				<Card>
					<Stack gap="var(--sp-2)">
						<Heading level={3} size="sm">{selected ?? 'No selection'}</Heading>
						{#if current}
							<Cluster gap="var(--sp-2)">
								<Field label="X" for="rect-x">
									<Input
										id="rect-x"
										type="number"
										step={GRID}
										value={current[0]}
										size="sm"
										oninput={(event) => setField(0, event.currentTarget.value)}
									/>
								</Field>
								<Field label="Y" for="rect-y">
									<Input
										id="rect-y"
										type="number"
										step={GRID}
										value={current[1]}
										size="sm"
										oninput={(event) => setField(1, event.currentTarget.value)}
									/>
								</Field>
							</Cluster>
							<Cluster gap="var(--sp-2)">
								<Field label="Width" for="rect-w">
									<Input
										id="rect-w"
										type="number"
										step={GRID}
										value={current[2]}
										size="sm"
										oninput={(event) => setField(2, event.currentTarget.value)}
									/>
								</Field>
								<Field label="Height" for="rect-h">
									<Input
										id="rect-h"
										type="number"
										step={GRID}
										value={current[3]}
										size="sm"
										oninput={(event) => setField(3, event.currentTarget.value)}
									/>
								</Field>
							</Cluster>
							<Cluster gap="var(--sp-2)">
								<Button
									size="sm"
									variant={hidden.includes(selected!) ? 'primary' : 'ghost'}
									onclick={() => toggle('hidden', selected!)}
								>
									{hidden.includes(selected!) ? 'Hidden' : 'Hide'}
								</Button>
								<Button
									size="sm"
									variant={bare.includes(selected!) ? 'primary' : 'ghost'}
									onclick={() => toggle('bare', selected!)}
								>
									{bare.includes(selected!) ? 'Transparent' : 'Make transparent'}
								</Button>
							</Cluster>
						{:else}
							<Text tone="muted">Pick a window on the canvas.</Text>
						{/if}
					</Stack>
				</Card>

				<Card>
					<Stack gap="var(--sp-2)">
						<Heading level={3} size="sm">Other hidden panels</Heading>
						<Text tone="muted" size="sm">
							Ini sections the layout does not place — toolbars, the map, the stance bar.
						</Text>
						{#if extras.length}
							<Cluster gap="var(--sp-1)">
								{#each extras as window (window)}
									<Button size="sm" variant="ghost" onclick={() => toggle('hidden', window)}>
										{window} ✕
									</Button>
								{/each}
							</Cluster>
						{/if}
						<Cluster gap="var(--sp-2)">
							<Field label="Section" for="hide-section">
								<Input
									id="hide-section"
									bind:value={section}
									mono
									size="sm"
									placeholder="EQMainWnd"
									onkeydown={(event: KeyboardEvent) => event.key === 'Enter' && addSection()}
								/>
							</Field>
							<Button size="sm" onclick={addSection} disabled={!section.trim()}>Hide</Button>
						</Cluster>
					</Stack>
				</Card>

				<Card>
					<Stack gap="var(--sp-2)">
						<Cluster justify="space-between">
							<Heading level={3} size="sm">Problems</Heading>
							<Badge tone={problems.length ? 'danger' : 'ok'} mono>{problems.length}</Badge>
						</Cluster>
						{#if problems.length === 0}
							<Text tone="muted">No overlaps, nothing offscreen.</Text>
						{:else}
							<Cluster gap="var(--sp-1)">
								{#each problems as problem (problem)}
									<Badge tone="danger">{problem}</Badge>
								{/each}
							</Cluster>
						{/if}
					</Stack>
				</Card>

				<Card>
					<Cluster gap="var(--sp-2)">
						<Field label="Screen width" for="screen-w">
							<Input id="screen-w" type="number" bind:value={screenW} size="sm" />
						</Field>
						<Field label="Screen height" for="screen-h">
							<Input id="screen-h" type="number" bind:value={screenH} size="sm" />
						</Field>
					</Cluster>
				</Card>
			</Stack>
		</Cluster>
	{/if}
</Stack>

<style>
	.canvas {
		display: block;
		touch-action: none;
		background: var(--bg-sunken);
		border-radius: var(--radius-2);
	}

	.screen {
		fill: none;
		stroke: var(--border);
		stroke-width: 4;
	}
	.window {
		fill: color-mix(in srgb, var(--tone) 18%, transparent);
		stroke: var(--tone);
		stroke-width: 4;
		cursor: move;
	}
	g.selected .window {
		stroke-width: 10;
	}
	g.hidden .window {
		fill: none;
		stroke-dasharray: 24 16;
	}
	g.hidden .label {
		opacity: 0.5;
	}
	.label {
		fill: var(--fg);
		font-size: 40px;
		pointer-events: none;
		user-select: none;
	}
	.handle {
		fill: var(--tone, var(--accent));
		opacity: 0.7;
		cursor: nwse-resize;
	}
</style>
