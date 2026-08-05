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
		Link,
		Metric,
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
	import { damageDelay, equippedTotals, itemPath } from '$lib/items';
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
	const totals = $derived(equippedTotals(groups.equipped));

	const num = (value: number | null | undefined) => (value ? String(value) : '');

	const equippedColumns: Column<InventoryEntry>[] = [
		{ key: 'location', label: 'Slot', sortable: true },
		{ key: 'name', label: 'Item', sortable: true },
		{ key: 'ac', label: 'AC', align: 'right', sortable: true, get: (e) => num(e.item?.stats.ac) },
		{ key: 'hp', label: 'HP', align: 'right', sortable: true, get: (e) => num(e.item?.stats.hp) },
		{
			key: 'mana',
			label: 'Mana',
			align: 'right',
			sortable: true,
			get: (e) => num(e.item?.stats.mana)
		},
		{ key: 'dmgdelay', label: 'Dmg/Delay', align: 'right', get: (e) => damageDelay(e.item?.stats) },
		{
			key: 'weight',
			label: 'Weight',
			align: 'right',
			sortable: true,
			get: (e) => num(e.item?.stats.weight)
		}
	];

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
			<Cluster justify="space-between">
				<Heading level={3}>Equipped</Heading>
				<Text variant="caption" tone="faint">
					{totals.known} of {totals.known + totals.unknown} items in the item database
				</Text>
			</Cluster>

			<Cluster gap="var(--sp-3)">
				<Metric label="Equipped AC" value={totals.ac} icon="lock" tone="info" />
				<Metric label="Equipped HP" value={totals.hp} icon="heart" tone="ok" />
				<Metric label="Equipped Mana" value={totals.mana} icon="star" tone="info" />
				<Metric label="Equipped Weight" value={totals.weight} icon="archive" />
			</Cluster>

			<Card padding="none">
				<DataTable
					columns={equippedColumns}
					rows={groups.equipped}
					{rowKey}
					cellSnippets={{ name: itemName }}
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
							cellSnippets={{ name: itemName }}
							empty="General inventory is empty."
							stickyHeader
						/>
					{:else}
						<DataTable
							{columns}
							rows={bank}
							{rowKey}
							cellSnippets={{ name: itemName }}
							empty="Bank is empty."
							stickyHeader
						/>
					{/if}
				</Card>
			{/snippet}
		</Tabs>
	{/if}
</Stack>

{#snippet itemName(entry: InventoryEntry)}
	{#if entry.item}
		<Cluster gap="var(--sp-2)">
			<Link href={itemPath(entry.item.name)}>{entry.name}</Link>
			{#if entry.item.stats.effects.length}
				<Badge tone="info" size="sm">{entry.item.stats.effects[0].kind}</Badge>
			{/if}
		</Cluster>
	{:else if entry.name === 'Empty'}
		<Text tone="faint">Empty</Text>
	{:else}
		<Text>{entry.name}</Text>
	{/if}
{/snippet}
