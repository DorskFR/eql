<script lang="ts">
	import { page } from '$app/state';
	import {
		Badge,
		Button,
		Card,
		Cluster,
		DataTable,
		EmptyState,
		Heading,
		Spinner,
		Stack,
		Tabs,
		Text,
		Timestamp,
		type Column,
		type TabItem
	} from '@dorsk/tsumikit';
	import type { InventoryEntry } from '$lib/api';
	import { filled, groupEntries } from '$lib/inventory';
	import { useInventory } from '$lib/queries';

	const server = $derived(page.params.server ?? '');
	const name = $derived(page.params.name ?? '');

	const inventory = useInventory(
		() => server,
		() => name
	);

	const groups = $derived(groupEntries(inventory.data?.entries ?? []));
	const general = $derived(filled(groups.general));
	const bank = $derived(filled(groups.bank));

	const columns: Column<InventoryEntry>[] = [
		{ key: 'location', label: 'Location', sortable: true },
		{ key: 'name', label: 'Item', sortable: true },
		{ key: 'count', label: 'Count', align: 'right', sortable: true },
		{ key: 'id', label: 'Item ID', align: 'right', sortable: true }
	];

	const rowKey = (entry: InventoryEntry) => `${entry.location}:${entry.id}`;

	const tabs: TabItem[] = $derived([
		{ id: 'general', label: `General (${general.length})`, icon: 'archive' },
		{ id: 'bank', label: `Bank (${bank.length})`, icon: 'lock' }
	]);

	let tab = $state('general');
</script>

<Stack gap="var(--sp-4)">
	<Cluster justify="space-between">
		<Cluster gap="var(--sp-3)">
			<Button href="/" variant="ghost" size="sm">Back</Button>
			<Heading level={2}>{name}</Heading>
			<Badge tone="info">{server}</Badge>
		</Cluster>
		{#if inventory.data}
			<Badge tone="neutral">
				last synced <Timestamp value={inventory.data.captured_at} mode="relative" details={false} />
			</Badge>
		{/if}
	</Cluster>

	{#if inventory.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading inventory…</Text>
			</Cluster>
		</Card>
	{:else if inventory.isError}
		<EmptyState
			title="No inventory for this character"
			description={inventory.error.message}
			icon="alert-circle"
			tone="danger"
			actionLabel="Retry"
			onAction={() => inventory.refetch()}
		/>
	{:else}
		<Stack gap="var(--sp-2)">
			<Heading level={3}>Equipped</Heading>
			<Card padding="none">
				<DataTable
					{columns}
					rows={groups.equipped}
					{rowKey}
					empty="No equipped slots in this snapshot."
					stickyHeader
				/>
			</Card>
		</Stack>

		<Tabs {tabs} bind:value={tab} label="Inventory containers">
			{#snippet panel(id)}
				<Card padding="none">
					{#if id === 'general'}
						<DataTable
							{columns}
							rows={general}
							{rowKey}
							empty="General inventory is empty."
							stickyHeader
						/>
					{:else}
						<DataTable {columns} rows={bank} {rowKey} empty="Bank is empty." stickyHeader />
					{/if}
				</Card>
			{/snippet}
		</Tabs>
	{/if}
</Stack>
