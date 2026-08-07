<script lang="ts">
	import {
		Badge,
		Button,
		Card,
		Cluster,
		EmptyState,
		Field,
		Heading,
		Input,
		Spinner,
		Stack,
		Text
	} from '@dorsk/tsumikit';
	import { endpoints, type DeviceSummary, type SessionLog, type SessionSummary } from '$lib/api';
	import { tokenStore } from '$lib/layout';

	let token = $state(tokenStore.load());
	let devices = $state<DeviceSummary[]>([]);
	let sessions = $state<SessionSummary[]>([]);
	let log = $state<SessionLog | null>(null);
	let device = $state('');
	let filter = $state('');
	let busy = $state(false);
	let error = $state('');

	async function run<T>(work: () => Promise<T>): Promise<T | undefined> {
		busy = true;
		error = '';
		try {
			return await work();
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
		} finally {
			busy = false;
		}
	}

	async function load() {
		tokenStore.save(token);
		sessions = [];
		log = null;
		device = '';
		devices = (await run(() => endpoints.devices(token))) ?? [];
	}

	async function openDevice(name: string) {
		device = name;
		log = null;
		sessions = (await run(() => endpoints.deviceSessions(token, name))) ?? [];
	}

	async function openSession(session: string) {
		log = (await run(() => endpoints.deviceSession(token, device, session))) ?? null;
	}

	const when = (iso: string) => new Date(iso).toLocaleString();
	const lines = $derived(
		log ? log.lines.filter((line) => !filter || line.toLowerCase().includes(filter.toLowerCase())) : []
	);
</script>

<Stack gap="var(--sp-4)">
	<Heading level={2}>Devices</Heading>

	<Card>
		<Cluster gap="var(--sp-2)" align="flex-end">
			<Field label="Machine token" for="devices-token" hint="Kept in this browser only.">
				<Input id="devices-token" type="password" bind:value={token} mono size="sm" />
			</Field>
			<Button variant="primary" disabled={!token || busy} onclick={load}>
				{busy ? 'Loading…' : 'Load'}
			</Button>
		</Cluster>
		{#if error}<Text tone="danger">{error}</Text>{/if}
	</Card>

	{#if devices.length === 0}
		<EmptyState
			title="No devices yet"
			description="Load with the machine token, or wait for an eqld with [log] upload on to check in."
			icon="info"
		/>
	{:else}
		<Cluster gap="var(--sp-2)">
			{#each devices as row (row.device)}
				<Button
					size="sm"
					variant={device === row.device ? 'primary' : 'ghost'}
					onclick={() => openDevice(row.device)}
				>
					{row.device}
					<Badge mono>{row.sessions}</Badge>
				</Button>
			{/each}
		</Cluster>
		<Text tone="muted" size="sm">
			Last seen {when(devices.find((row) => row.device === device)?.last_at ?? devices[0].last_at)}
		</Text>
	{/if}

	{#if sessions.length}
		<Card>
			<Stack gap="var(--sp-2)">
				<Heading level={3} size="sm">Sessions on {device}</Heading>
				{#each sessions as row (row.session)}
					<Cluster justify="space-between" gap="var(--sp-2)">
						<Button size="sm" variant="ghost" onclick={() => openSession(row.session)}>
							{when(row.started_at)}
						</Button>
						<Cluster gap="var(--sp-1)">
							<Badge mono>{row.lines} lines</Badge>
							{#if row.dropped > 0}<Badge tone="danger" mono>{row.dropped} dropped</Badge>{/if}
						</Cluster>
					</Cluster>
				{/each}
			</Stack>
		</Card>
	{/if}

	{#if log}
		<Card>
			<Stack gap="var(--sp-2)">
				<Cluster justify="space-between">
					<Heading level={3} size="sm">{log.session}</Heading>
					<Field label="Filter" for="log-filter">
						<Input id="log-filter" bind:value={filter} mono size="sm" placeholder="warn" />
					</Field>
				</Cluster>
				{#if log.dropped > 0}
					<Text tone="danger" size="sm">
						{log.dropped} line(s) were dropped before upload; this session is incomplete.
					</Text>
				{/if}
				<pre class="log">{lines.join('\n')}</pre>
			</Stack>
		</Card>
	{/if}
</Stack>

<style>
	.log {
		max-height: 60vh;
		overflow: auto;
		margin: 0;
		padding: var(--sp-2);
		background: var(--bg-sunken);
		border-radius: var(--radius-2);
		font-family: var(--font-mono);
		font-size: 0.8rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-word;
	}
</style>
