<script lang="ts">
	import { page } from '$app/state';
	import {
		AutoGrid,
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
	import type { InventoryEntry, ItemClasses, LogEvent, WeaponSummary } from '$lib/api';
	import { describeEvent, eventTone } from '$lib/events';
	import { filled, groupEntries } from '$lib/inventory';
	import {
		ATTRIBUTES,
		damageDelay,
		equippedTotals,
		gearPairs,
		itemPath,
		RESISTS,
		type StatRow,
		weaponDamageDelay,
		weaponRatio
	} from '$lib/items';
	import { useEvents, useInventory, useStats } from '$lib/queries';

	const server = $derived(page.params.server ?? '');
	const name = $derived(page.params.name ?? '');

	const inventory = useInventory(
		() => server,
		() => name
	);

	const gear = useStats(
		() => server,
		() => name
	);

	const events = useEvents(
		() => server,
		() => name
	);

	const eventRows = $derived(events.data?.pages.flat() ?? []);

	const eventColumns: Column<LogEvent>[] = [
		{ key: 'at', label: 'When' },
		{ key: 'kind', label: 'Event' },
		{ key: 'detail', label: 'Detail', get: describeEvent }
	];

	const stats = $derived(gear.data?.stats);
	const attributes = $derived(stats ? gearPairs(stats, ATTRIBUTES) : []);
	const resists = $derived(stats ? gearPairs(stats, RESISTS) : []);
	const restricted = $derived(stats?.item_classes.filter((item) => item.classes.length) ?? []);

	const signed = (value: number) => (value > 0 ? `+${value}` : String(value));

	const statColumns: Column<StatRow>[] = [
		{ key: 'label', label: 'Stat' },
		{ key: 'value', label: 'From gear', align: 'right', get: (row) => signed(row.value) }
	];

	const classColumns: Column<ItemClasses>[] = [
		{ key: 'location', label: 'Slot', sortable: true },
		{ key: 'name', label: 'Item', sortable: true },
		{ key: 'classes', label: 'Usable by', get: (row) => row.classes.join(' ') }
	];

	const coverLabel = (needed: number | null) =>
		needed === null
			? 'No three classes cover this gear'
			: needed === 0
				? 'No class-restricted items'
				: `${needed} class${needed > 1 ? 'es' : ''} needed for this gear`;

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
		{ id: 'stats', label: 'Stats', icon: 'star' },
		{ id: 'general', label: `General (${general.length})`, icon: 'archive' },
		{ id: 'bank', label: `Bank (${bank.length})`, icon: 'lock' },
		{ id: 'events', label: 'Events', icon: 'clock' }
	]);

	let tab = $state('stats');
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
				{#if id === 'stats'}
					{@render statsPanel()}
				{:else if id === 'events'}
					{@render eventsPanel()}
				{:else}
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
				{/if}
			{/snippet}
		</Tabs>
	{/if}
</Stack>

{#snippet weaponCard(label: string, weapon: WeaponSummary | null)}
	<Stack gap="var(--sp-1)">
		<Cluster gap="var(--sp-2)">
			<Text variant="caption" tone="muted">{label}</Text>
			{#if weapon}
				<Link href={itemPath(weapon.name)}>{weapon.name}</Link>
				{#if weapon.item_type}
					<Badge size="sm" tone="info">{weapon.item_type}</Badge>
				{/if}
			{:else}
				<Text tone="faint">empty</Text>
			{/if}
		</Cluster>
		{#if weapon}
			<Cluster gap="var(--sp-2)">
				<Badge mono>{weaponDamageDelay(weapon)}</Badge>
				<Text variant="caption" tone="muted">ratio {weaponRatio(weapon)}</Text>
			</Cluster>
		{/if}
	</Stack>
{/snippet}

{#snippet statsPanel()}
	{#if gear.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading stats…</Text>
			</Cluster>
		</Card>
	{:else if gear.isError || !stats}
		<EmptyState
			title="No derived stats"
			description={gear.error?.message ?? 'This character has no snapshot yet.'}
			icon="alert-circle"
			tone="warn"
			actionLabel="Retry"
			onAction={() => gear.refetch()}
		/>
	{:else}
		<Stack gap="var(--sp-3)">
			<Text variant="caption" tone="faint">
				From gear only — race, class and level base stats are not included.
				{stats.known_items} of {stats.equipped_count} equipped items are in the item database.
			</Text>

			<AutoGrid min="10rem">
				<Metric label="AC" value={stats.ac} icon="lock" tone="info" />
				<Metric label="HP" value={stats.hp} icon="heart" tone="ok" />
				<Metric label="Mana" value={stats.mana} icon="star" tone="info" />
				<Metric label="Endurance" value={stats.endurance} icon="live" />
				<Metric label="Haste" value={stats.haste} unit="%" icon="clock" sub="highest worn" />
				<Metric label="Weight" value={stats.weight} icon="archive" />
			</AutoGrid>

			<AutoGrid min="18rem">
				<Card padding="none">
					<DataTable
						columns={statColumns}
						rows={attributes}
						rowKey={(row) => row.label}
						empty="No attributes from gear."
					/>
				</Card>
				<Card>
					<Stack gap="var(--sp-2)">
						<Heading level={3} size="sm">Resists</Heading>
						<Cluster gap="var(--sp-2)">
							{#each resists as resist (resist.label)}
								<Badge tone={resist.value > 0 ? 'info' : 'neutral'} mono>
									{resist.label}
									{signed(resist.value)}
								</Badge>
							{/each}
						</Cluster>
					</Stack>
				</Card>
				<Card>
					<Stack gap="var(--sp-3)">
						<Heading level={3} size="sm">Weapons</Heading>
						{@render weaponCard('Primary', stats.primary)}
						{@render weaponCard('Secondary', stats.secondary)}
					</Stack>
				</Card>
			</AutoGrid>

			<Card>
				<Stack gap="var(--sp-2)">
					<Cluster justify="space-between">
						<Heading level={3} size="sm">Usable by</Heading>
						<Badge tone={stats.min_classes_needed === null ? 'danger' : 'neutral'}>
							{coverLabel(stats.min_classes_needed)}
						</Badge>
					</Cluster>
					{#if stats.usable_by.length}
						<Cluster gap="var(--sp-2)">
							{#each stats.usable_by as klass (klass)}
								<Badge tone="ok" size="sm">{klass}</Badge>
							{/each}
						</Cluster>
					{:else}
						<Text tone="muted" variant="caption">
							No single class can wear every equipped item — normal for a three-class loadout.
						</Text>
					{/if}
				</Stack>
			</Card>

			{#if restricted.length}
				<Card padding="none">
					<DataTable
						columns={classColumns}
						rows={restricted}
						rowKey={(row) => `${row.location}:${row.name}`}
						empty="No class-restricted items."
						stickyHeader
					/>
				</Card>
			{/if}
		</Stack>
	{/if}
{/snippet}

{#snippet eventsPanel()}
	{#if events.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading events…</Text>
			</Cluster>
		</Card>
	{:else if events.isError}
		<EmptyState
			title="No event stream"
			description={events.error.message}
			icon="alert-circle"
			tone="warn"
			actionLabel="Retry"
			onAction={() => events.refetch()}
		/>
	{:else if eventRows.length === 0}
		<EmptyState
			title="No events yet"
			description="Turn logging on in game with /log on — the daemon ships new lines as they are written."
			icon="clock"
		/>
	{:else}
		<Stack gap="var(--sp-2)">
			<Card padding="none">
				<DataTable
					columns={eventColumns}
					rows={eventRows}
					rowKey={(event) => event.id}
					cellSnippets={{ at: eventWhen, kind: eventKind }}
					empty="No events yet."
					stickyHeader
				/>
			</Card>
			<Cluster justify="space-between">
				<Text variant="caption" tone="faint">{eventRows.length} events</Text>
				{#if events.hasNextPage}
					<Button
						variant="ghost"
						size="sm"
						loading={events.isFetchingNextPage}
						onclick={() => events.fetchNextPage()}
					>
						Load older
					</Button>
				{/if}
			</Cluster>
		</Stack>
	{/if}
{/snippet}

{#snippet eventWhen(event: LogEvent)}
	<Timestamp value={event.at} mode="datetime" />
{/snippet}

{#snippet eventKind(event: LogEvent)}
	<Badge tone={eventTone(event)} size="sm">{event.kind}</Badge>
{/snippet}

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
