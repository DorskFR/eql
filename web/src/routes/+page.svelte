<script lang="ts">
	import { goto } from '$app/navigation';
	import {
		Badge,
		Card,
		Cluster,
		DataTable,
		EmptyState,
		Heading,
		Spinner,
		Stack,
		Text,
		Timestamp,
		type Column
	} from '@dorsk/tsumikit';
	import type { CharacterSummary } from '$lib/api';
	import { useCharacters } from '$lib/queries';

	const characters = useCharacters();

	const columns: Column<CharacterSummary>[] = [
		{ key: 'name', label: 'Character', sortable: true },
		{ key: 'server', label: 'Server', sortable: true },
		{ key: 'last_snapshot_at', label: 'Last synced', sortable: true },
		{ key: 'snapshot_count', label: 'Snapshots', align: 'right', sortable: true }
	];

	const detailPath = (row: CharacterSummary) =>
		`/c/${encodeURIComponent(row.server)}/${encodeURIComponent(row.name)}`;
</script>

<Stack gap="var(--sp-4)">
	<Cluster justify="space-between">
		<Heading level={2}>Characters</Heading>
		{#if characters.isFetching}
			<Spinner label="Refreshing" />
		{/if}
	</Cluster>

	{#if characters.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading characters…</Text>
			</Cluster>
		</Card>
	{:else if characters.isError}
		<EmptyState
			title="Could not load characters"
			description={characters.error.message}
			icon="alert-circle"
			tone="danger"
			actionLabel="Retry"
			onAction={() => characters.refetch()}
		/>
	{:else}
		<Card padding="none">
			<DataTable
				{columns}
				rows={characters.data}
				rowKey={(row) => `${row.server}/${row.name}`}
				onrowclick={(row) => goto(detailPath(row))}
				cellSnippets={{ last_snapshot_at: lastSynced, snapshot_count: snapshotCount }}
				empty="No characters have synced yet."
			/>
		</Card>
	{/if}
</Stack>

{#snippet lastSynced(row: CharacterSummary)}
	{#if row.last_snapshot_at}
		<Timestamp value={row.last_snapshot_at} mode="relative" />
	{:else}
		<Text tone="faint">never</Text>
	{/if}
{/snippet}

{#snippet snapshotCount(row: CharacterSummary)}
	<Badge tone={row.snapshot_count > 0 ? 'ok' : 'neutral'} mono>{row.snapshot_count}</Badge>
{/snippet}
