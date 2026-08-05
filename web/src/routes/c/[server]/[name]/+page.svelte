<script lang="ts">
	import { page } from '$app/state';
	import {
		AutoGrid,
		Badge,
		Button,
		Card,
		Cluster,
		CodeBlock,
		DataTable,
		EmptyState,
		Heading,
		Link,
		Metric,
		Progress,
		Spinner,
		Stack,
		Tabs,
		Text,
		Timestamp,
		type Column,
		type TabItem
	} from '@dorsk/tsumikit';
	import type { InventoryEntry, ItemClasses, LogEvent, WeaponSummary } from '$lib/api';
	import {
		type BuildRow,
		coin,
		type DropRow,
		duration,
		percent,
		projectAlltime,
		projectAtlas,
		projectQuest,
		type QuestRow,
		rawJson,
		type ZoneRow
	} from '$lib/harvest';
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
	import { useEvents, useHarvest, useInventory, useStats } from '$lib/queries';

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

	const atlasDoc = useHarvest(
		() => server,
		() => name,
		'atlas'
	);
	const questDoc = useHarvest(
		() => server,
		() => name,
		'quest'
	);
	const alltimeDoc = useHarvest(
		() => server,
		() => name,
		'alltime'
	);

	const atlas = $derived(projectAtlas(atlasDoc.data?.doc));
	const quests = $derived(projectQuest(questDoc.data?.doc));
	const alltime = $derived(projectAlltime(alltimeDoc.data?.doc));

	const zoneColumns: Column<ZoneRow>[] = [
		{ key: 'zone', label: 'Zone', sortable: true },
		{ key: 'kills', label: 'Kills', align: 'right', sortable: true },
		{ key: 'group_kills', label: 'Group kills', align: 'right', sortable: true },
		{ key: 'loots', label: 'Loots', align: 'right', sortable: true },
		{
			key: 'coin_copper',
			label: 'Coin',
			align: 'right',
			sortable: true,
			get: (row) => coin(row.coin_copper)
		},
		{ key: 'mobs', label: 'Mobs seen', align: 'right', sortable: true }
	];

	const dropColumns: Column<DropRow>[] = [
		{ key: 'item', label: 'Item', sortable: true },
		{ key: 'mob', label: 'From', sortable: true },
		{ key: 'zone', label: 'Zone', sortable: true },
		{ key: 'count', label: 'Count', align: 'right', sortable: true },
		{
			key: 'sold_copper',
			label: 'Sold for',
			align: 'right',
			sortable: true,
			get: (row) => coin(row.sold_copper)
		}
	];

	const questColumns: Column<QuestRow>[] = [
		{ key: 'quest', label: 'Quest', sortable: true },
		{ key: 'have', label: 'Have', align: 'right', sortable: true },
		{
			key: 'need',
			label: 'Need',
			align: 'right',
			sortable: true,
			get: (row) => (row.need === null ? '—' : String(row.need))
		},
		{ key: 'ratio', label: 'Progress' },
		{ key: 'flags', label: '' }
	];

	const buildColumns: Column<BuildRow>[] = [
		{ key: 'build', label: 'Build', sortable: true },
		{
			key: 'dps',
			label: 'DPS',
			align: 'right',
			sortable: true,
			get: (row) => (row.dps === null ? '—' : row.dps.toFixed(1))
		},
		{ key: 'damage', label: 'Damage', align: 'right', sortable: true },
		{ key: 'kills', label: 'Kills', align: 'right', sortable: true },
		{ key: 'deaths', label: 'Deaths', align: 'right', sortable: true },
		{ key: 'biggest', label: 'Biggest hit', align: 'right', sortable: true },
		{
			key: 'accuracy',
			label: 'Accuracy',
			align: 'right',
			sortable: true,
			get: (row) => percent(row.accuracy)
		},
		{
			key: 'combat_secs',
			label: 'In combat',
			align: 'right',
			sortable: true,
			get: (row) => duration(row.combat_secs)
		}
	];

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
		{ id: 'events', label: 'Events', icon: 'clock' },
		{ id: 'atlas', label: 'Atlas', icon: 'grid' },
		{ id: 'quests', label: 'Quests', icon: 'bookmark' }
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
				{:else if id === 'atlas'}
					{@render atlasPanel()}
				{:else if id === 'quests'}
					{@render questsPanel()}
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

			{@render alltimeCard()}

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

{#snippet harvestState(
	query: { isPending: boolean; isError: boolean; error: Error | null; refetch: () => void },
	what: string
)}
	{#if query.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading {what}…</Text>
			</Cluster>
		</Card>
	{:else}
		<EmptyState
			title="No {what} yet"
			description="Run the EQL Log Reader companion app and switch on [harvest] in eqld to see this."
			icon="archive"
			actionLabel="Retry"
			onAction={() => query.refetch()}
		/>
	{/if}
{/snippet}

{#snippet rawFallback(label: string, json: string)}
	<Card>
		<Stack gap="var(--sp-2)">
			<Text variant="caption" tone="muted">
				{label} — the harvested file did not match a shape this page knows, so it is shown raw.
			</Text>
			<CodeBlock code={json} lang="json" wrap copy />
		</Stack>
	</Card>
{/snippet}

{#snippet alltimeCard()}
	{#if alltimeDoc.data && alltime.usable}
		<Card>
			<Stack gap="var(--sp-2)">
				<Cluster justify="space-between">
					<Heading level={3} size="sm">Per-build lifetime</Heading>
					<Badge tone="neutral">
						harvested <Timestamp
							value={alltimeDoc.data.captured_at}
							mode="relative"
							details={false}
						/>
					</Badge>
				</Cluster>
				<DataTable
					columns={buildColumns}
					rows={alltime.builds}
					rowKey={(row) => row.key}
					empty="No lifetime combat stats."
				/>
				{#if alltime.sources.length}
					<Cluster gap="var(--sp-2)">
						{#each alltime.sources as source (source.key)}
							<Badge tone="info" size="sm" mono>
								{source.source}
								{percent(source.share)}
							</Badge>
						{/each}
					</Cluster>
				{/if}
			</Stack>
		</Card>
	{:else if alltimeDoc.data}
		{@render rawFallback('Per-build lifetime', rawJson(alltimeDoc.data))}
	{/if}
{/snippet}

{#snippet atlasPanel()}
	{#if !atlasDoc.data}
		{@render harvestState(atlasDoc, 'atlas data')}
	{:else if !atlas.usable}
		{@render rawFallback('Atlas', rawJson(atlasDoc.data))}
	{:else}
		<Stack gap="var(--sp-3)">
			<Cluster justify="space-between">
				<Text variant="caption" tone="faint">
					Observed by the EQL Log Reader Atlas across {atlas.zones.length} zones.
				</Text>
				<Badge tone="neutral">
					harvested <Timestamp value={atlasDoc.data.captured_at} mode="relative" details={false} />
				</Badge>
			</Cluster>

			<AutoGrid min="10rem">
				<Metric label="Kills" value={atlas.kills} icon="star" tone="ok" />
				<Metric label="Group kills" value={atlas.group_kills} icon="users" />
				<Metric label="Loots" value={atlas.loots} icon="archive" tone="info" />
				<Metric label="Coin" value={coin(atlas.coin_copper)} icon="tag" />
			</AutoGrid>

			<Card padding="none">
				<DataTable
					columns={zoneColumns}
					rows={atlas.zones}
					rowKey={(row) => row.key}
					empty="No zones recorded."
					stickyHeader
				/>
			</Card>

			{#if atlas.top_drops.length}
				<Stack gap="var(--sp-2)">
					<Heading level={3} size="sm">Top drops</Heading>
					<Card padding="none">
						<DataTable
							columns={dropColumns}
							rows={atlas.top_drops}
							rowKey={(row) => row.key}
							cellSnippets={{ item: dropItem }}
							empty="No drops recorded."
							stickyHeader
						/>
					</Card>
				</Stack>
			{/if}
		</Stack>
	{/if}
{/snippet}

{#snippet dropItem(row: DropRow)}
	<Link href={itemPath(row.item)}>{row.item}</Link>
{/snippet}

{#snippet questProgress(row: QuestRow)}
	{#if row.ratio === null}
		<Text tone="faint" variant="caption">{row.have} collected</Text>
	{:else}
		<Progress value={row.ratio * 100} max={100} size="sm" tone={row.ratio >= 1 ? 'success' : 'accent'} />
	{/if}
{/snippet}

{#snippet questFlags(row: QuestRow)}
	<Cluster gap="var(--sp-2)">
		{#if row.tracked}
			<Badge tone="info" size="sm">tracked</Badge>
		{/if}
		{#if row.confirmed}
			<Badge tone="ok" size="sm">completed</Badge>
		{/if}
	</Cluster>
{/snippet}

{#snippet questsPanel()}
	{#if !questDoc.data}
		{@render harvestState(questDoc, 'quest data')}
	{:else if !quests.usable}
		{@render rawFallback('Quests', rawJson(questDoc.data))}
	{:else}
		<Stack gap="var(--sp-3)">
			<Cluster justify="space-between">
				<Text variant="caption" tone="faint">
					Required counts live in the companion app's quest database, so only collected items are
					known here.
				</Text>
				<Badge tone="neutral">
					harvested <Timestamp value={questDoc.data.captured_at} mode="relative" details={false} />
				</Badge>
			</Cluster>

			<Cluster gap="var(--sp-3)">
				<Metric label="On the list" value={quests.quests.length} icon="list" />
				<Metric label="Confirmed complete" value={quests.confirmed} icon="check-circle" tone="ok" />
			</Cluster>

			<Card padding="none">
				<DataTable
					columns={questColumns}
					rows={quests.quests}
					rowKey={(row) => row.key}
					cellSnippets={{ ratio: questProgress, flags: questFlags }}
					empty="No quests tracked."
					stickyHeader
				/>
			</Card>
		</Stack>
	{/if}
{/snippet}
