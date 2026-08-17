<script lang="ts">
	import { page } from '$app/state';
	import { SvelteSet } from 'svelte/reactivity';
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
		Icon,
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
	import type { InventoryEntry, ItemClasses, ItemStats, LogEvent, WeaponSummary } from '$lib/api';
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
		type ShareRow,
		type ZoneRow
	} from '$lib/harvest';
	import { describeEvent, eventTone } from '$lib/events';
	import {
		type AbilityRow,
		categoryTone,
		clock,
		enemyList,
		type FightRow,
		projectFights
	} from '$lib/fights';
	import { bags, filled, groupEntries, paperdoll } from '$lib/inventory';
	import {
		ATTRIBUTES,
		attributeTotals,
		damageDelay,
		equippedTotals,
		gearPairs,
		iconUrl,
		itemPath,
		RESISTS,
		type StatRow,
		weaponDamageDelay,
		weaponRatio
	} from '$lib/items';
	import {
		useBis,
		useCharacter,
		useEvents,
		useFights,
		useHarvest,
		useInventory,
		useStats
	} from '$lib/queries';

	const server = $derived(page.params.server ?? '');
	const name = $derived(page.params.name ?? '');
	const picked = $derived(page.url.searchParams.get('loadout') ?? '');

	const inventory = useInventory(
		() => server,
		() => name,
		() => picked
	);

	const gear = useStats(
		() => server,
		() => name,
		() => picked
	);

	const events = useEvents(
		() => server,
		() => name
	);

	const character = useCharacter(
		() => server,
		() => name
	);

	const who = $derived(character.data?.identity_at ? character.data : null);
	const loadouts = $derived(character.data?.loadouts ?? []);
	/** The profile on screen belongs to the loadout its snapshot was taken in,
	 *  which is not always the one the character is wearing right now. */
	const shown = $derived(loadouts.find((entry) => entry.key === inventory.data?.loadout) ?? null);
	const classes = $derived((shown?.classes ?? who?.classes ?? []).join('/'));
	const shownLevel = $derived(shown?.level ?? who?.level);
	const identityLine = $derived(
		who
			? [`Level ${shownLevel} ${classes}`.trim(), who.race, server].filter(Boolean).join(' · ')
			: ''
	);
	const loadoutHref = (key: string) => `?loadout=${encodeURIComponent(key)}`;

	const eventRows = $derived(events.data?.pages.flat() ?? []);
	const loggedLevel = $derived(
		eventRows.reduce(
			(max, event) => (event.kind === 'level' ? Math.max(max, event.payload.level ?? 0) : max),
			0
		)
	);
	const level = $derived(who?.level ?? loggedLevel);

	const fightPages = useFights(
		() => server,
		() => name
	);

	const fights = $derived(projectFights(fightPages.data?.pages.flat()));
	const openFights = new SvelteSet<string>();

	function toggleFight(key: string) {
		if (!openFights.delete(key)) openFights.add(key);
	}

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

	const count = (value: number) => value.toLocaleString();
	const rate = (value: number | null) => (value === null ? '—' : value.toFixed(1));

	const buildColumns: Column<BuildRow>[] = [
		{ key: 'build', label: 'Build', sortable: true, width: '11rem' },
		{
			key: 'damage',
			label: 'Damage',
			align: 'right',
			sortable: true,
			get: (row) => count(row.damage)
		},
		{ key: 'dps', label: 'DPS', align: 'right', sortable: true, get: (row) => rate(row.dps) },
		{ key: 'kills', label: 'Kills', align: 'right', sortable: true },
		{ key: 'deaths', label: 'Deaths', align: 'right', sortable: true },
		{
			key: 'kill_death',
			label: 'K/D',
			align: 'right',
			sortable: true,
			get: (row) => rate(row.kill_death)
		},
		{ key: 'hits', label: 'Hits', align: 'right', sortable: true, get: (row) => count(row.hits) },
		{
			key: 'accuracy',
			label: 'Accuracy',
			align: 'right',
			sortable: true,
			get: (row) => percent(row.accuracy)
		},
		{
			key: 'crit_rate',
			label: 'Crit rate',
			align: 'right',
			sortable: true,
			get: (row) => percent(row.crit_rate)
		},
		{ key: 'biggest', label: 'Biggest hit', align: 'right', sortable: true },
		{
			key: 'combat_secs',
			label: 'In combat',
			align: 'right',
			sortable: true,
			get: (row) => duration(row.combat_secs)
		}
	];

	const abilityColumns: Column<AbilityRow>[] = [
		{ key: 'name', label: 'Ability', sortable: true },
		{ key: 'category', label: 'Type' },
		{
			key: 'total',
			label: 'Damage',
			align: 'right',
			sortable: true,
			get: (row) => count(row.total)
		},
		{ key: 'share', label: 'Share of fight' },
		{ key: 'hits', label: 'Hits', align: 'right', sortable: true },
		{ key: 'crits', label: 'Crits', align: 'right', sortable: true },
		{
			key: 'average',
			label: 'Average',
			align: 'right',
			sortable: true,
			get: (row) => rate(row.average)
		},
		{ key: 'biggest', label: 'Biggest', align: 'right', sortable: true }
	];

	const eventColumns: Column<LogEvent>[] = [
		{ key: 'at', label: 'When' },
		{ key: 'kind', label: 'Event' },
		{ key: 'detail', label: 'Detail', get: describeEvent }
	];

	const stats = $derived(gear.data?.stats);
	const base = $derived(gear.data?.base);
	const attributes = $derived(stats ? gearPairs(stats, ATTRIBUTES) : []);
	const totalAttributes = $derived(base && stats ? attributeTotals(base, stats) : []);
	const resists = $derived(stats ? gearPairs(stats, RESISTS) : []);
	const restricted = $derived(
		(stats?.item_classes.filter((item) => item.classes.length) ?? []).map((item, index) => ({
			...item,
			key: String(index)
		}))
	);

	const signed = (value: number) => (value > 0 ? `+${value}` : String(value));

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

	const keyedEntries = $derived(
		(inventory.data?.entries ?? []).map((entry, index) => ({ ...entry, key: String(index) }))
	);
	const groups = $derived(groupEntries(keyedEntries));
	const general = $derived(filled(groups.general));
	const bank = $derived(filled(groups.bank));
	const totals = $derived(equippedTotals(groups.equipped));
	const slotRows = $derived(paperdoll(groups.equipped));
	const generalBags = $derived(bags(keyedEntries.filter((e) => e.location.startsWith('General'))));
	const bankBags = $derived(
		bags(keyedEntries.filter((e) => e.location.startsWith('Bank') || e.location.startsWith('SharedBank')))
	);

	const shortName = (value: string) => value.replace(/ \+\d+$/, '');

	function tooltipLines(entry: InventoryEntry): string[] {
		const s = entry.item?.stats;
		if (!s) return [];
		const lines: string[] = [];
		const pair = (label: string, value: number | null | undefined) => {
			if (value) lines.push(`${label} ${signed(value)}`);
		};
		if (s.damage || s.delay) lines.push(`DMG ${s.damage ?? '?'} / DLY ${s.delay ?? '?'}`);
		pair('AC', s.ac);
		pair('HP', s.hp);
		pair('MANA', s.mana);
		for (const [key, label] of ATTRIBUTES) pair(label, s[key as keyof ItemStats] as number | null);
		for (const [key, label] of RESISTS)
			pair(`SV ${label.toUpperCase()}`, s[key as keyof ItemStats] as number | null);
		if (s.weight) lines.push(`WT ${s.weight}`);
		if (s.classes.length) lines.push(s.classes.join(' '));
		return lines;
	}

	const rowKey = (entry: InventoryEntry & { key: string }) => entry.key;

	const brokenIcons = new SvelteSet<string>();

	const tabs: TabItem[] = $derived([
		{ id: 'stats', label: 'Stats', icon: 'star' },
		{ id: 'bis', label: 'Best in Slot', icon: 'search' },
		{ id: 'general', label: `General (${general.length})`, icon: 'archive' },
		{ id: 'bank', label: `Bank (${bank.length})`, icon: 'lock' },
		{ id: 'events', label: 'Events', icon: 'clock' },
		{ id: 'fights', label: 'Fights', icon: 'list' },
		{ id: 'atlas', label: 'Atlas', icon: 'grid' },
		{ id: 'quests', label: 'Quests', icon: 'bookmark' },
		{ id: 'alltime', label: 'Lifetime', icon: 'live' }
	]);

	let tab = $state('general');

	const bis = useBis(
		() => server,
		() => name,
		() => picked,
		() => tab === 'bis'
	);

	const equippedBases = $derived(
		new Set(
			filled(groups.equipped).map((entry) =>
				(entry.item?.name ?? shortName(entry.name)).toLowerCase()
			)
		)
	);

	function candidateLine(stats: ItemStats): string {
		const parts: string[] = [];
		if (stats.damage || stats.delay) {
			const r = stats.damage && stats.delay ? ` (${(stats.damage / stats.delay).toFixed(2)})` : '';
			parts.push(`${stats.damage ?? '?'}/${stats.delay ?? '?'}${r}`);
		}
		const pair = (label: string, value: number | null) => {
			if (value) parts.push(`${label} ${signed(value)}`);
		};
		pair('AC', stats.ac);
		pair('HP', stats.hp);
		pair('MANA', stats.mana);
		pair('HASTE', stats.haste);
		for (const [key, label] of ATTRIBUTES) pair(label, stats[key] as number | null);
		return parts.join(' · ');
	}

	const candidateClasses = (stats: ItemStats) => stats.classes.join(' ');
</script>

<div class="eq">
	<Cluster justify="space-between">
		<Cluster gap="var(--sp-3)">
			<Button href="/" variant="ghost" size="sm">Back</Button>
			<Heading level={2}>{name}</Heading>
			{#if who}
				<Badge tone="neutral">Level {shownLevel} {classes}</Badge>
			{/if}
			<Badge tone="info">{server}</Badge>
		</Cluster>
		{#if inventory.data}
			<Badge tone="neutral">
				last synced <Timestamp value={inventory.data.captured_at} mode="relative" details={false} />
			</Badge>
		{/if}
	</Cluster>

	{#if loadouts.length > 1}
		<Cluster gap="var(--sp-2)">
			<Text variant="caption">Loadout</Text>
			{#each loadouts as entry (entry.key)}
				<Button
					href={loadoutHref(entry.key)}
					size="sm"
					variant={entry.key === shown?.key ? 'primary' : 'ghost'}
				>
					{entry.classes.join('/')}
					{entry.snapshot_count === 0 ? ' · no dump' : ''}
				</Button>
			{/each}
		</Cluster>
	{/if}

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
		<div class="eq-window">
			<div class="eq-sheet">
				<aside class="eq-col">
					<div class="eq-panel eq-identity">
						<div class="eq-name">{name}</div>
						<div class="eq-sub">
							{#if who}
								{identityLine}
							{:else if level}
								Level {level} <span class="eq-faint">(from logs)</span> · {server}
							{:else}
								{server}
							{/if}
						</div>
						{#if !who}
							<div class="eq-faint">
								Race and class unknown — press the EQLD social in game, or type /who
							</div>
						{/if}
					</div>

					<div class="eq-panel">
						<div class="eq-panel-title">Vitals · from gear</div>
						<div class="eq-vitals">
							<div class="eq-row"><span>HP</span><b class="eq-green">+{stats?.hp ?? totals.hp}</b></div>
							<div class="eq-row"><span>Mana</span><b class="eq-blue">+{stats?.mana ?? totals.mana}</b></div>
							<div class="eq-row"><span>End</span><b class="eq-tan">+{stats?.endurance ?? 0}</b></div>
							<div class="eq-row"><span>AC</span><b class="eq-green">+{stats?.ac ?? totals.ac}</b></div>
							{#if stats?.haste}
								<div class="eq-row"><span>Haste</span><b class="eq-green">{stats.haste}%</b></div>
							{/if}
						</div>
					</div>

					<div class="eq-panel">
						{#if totalAttributes.length}
							<div class="eq-panel-title">Attributes</div>
							<div class="eq-vitals">
								{#each totalAttributes as attr (attr.label)}
									<div class="eq-row">
										<span>{attr.label}</span>
										<span>
											{#if attr.gear}<span class="eq-gearpart eq-green">{signed(attr.gear)}</span>{/if}
											<b>{attr.total}</b>
										</span>
									</div>
								{/each}
							</div>
							<div class="eq-faint">
								Race and class base plus gear — points allocated at character creation are not in
								the dumps.
							</div>
						{:else}
							<div class="eq-panel-title">Attributes · from gear</div>
							<div class="eq-vitals">
								{#each attributes as attr (attr.label)}
									<div class="eq-row">
										<span>{attr.label}</span>
										<b class={attr.value > 0 ? 'eq-green' : 'eq-dim'}>{signed(attr.value)}</b>
									</div>
								{/each}
							</div>
						{/if}
					</div>

					<div class="eq-panel">
						<div class="eq-panel-title">Resists · from gear</div>
						<div class="eq-vitals">
							{#each resists as resist (resist.label)}
								<div class="eq-row">
									<span>SV {resist.label.toUpperCase()}</span>
									<b class={resist.value > 0 ? 'eq-green' : 'eq-dim'}>{signed(resist.value)}</b>
								</div>
							{/each}
						</div>
					</div>
				</aside>

				<section class="eq-col eq-main">
					<div class="eq-panel">
						<div class="eq-panel-title">
							Equipment
							<span class="eq-faint">{totals.known} of {totals.known + totals.unknown} items known</span>
						</div>
						<div class="eq-grid">
							{#each slotRows as row, rowIndex (rowIndex)}
								<div class="eq-grid-row">
									{#each row as slot (slot.key)}
										{@render slotCell(slot.label, slot.entry)}
									{/each}
								</div>
							{/each}
						</div>
						<div class="eq-weight">
							<span>EQUIPPED WEIGHT</span>
							<b class="eq-red">{totals.weight}</b>
						</div>
					</div>
				</section>
			</div>

			<Tabs {tabs} bind:value={tab} label="Inventory containers">
				{#snippet panel(id)}
					<div class="eq-tabpanel">
						{#if id === 'stats'}
							{@render statsPanel()}
						{:else if id === 'bis'}
							{@render bisPanel()}
						{:else if id === 'events'}
							{@render eventsPanel()}
						{:else if id === 'fights'}
							{@render fightsPanel()}
						{:else if id === 'atlas'}
							{@render atlasPanel()}
						{:else if id === 'quests'}
							{@render questsPanel()}
						{:else if id === 'alltime'}
							{@render alltimePanel()}
						{:else if id === 'general'}
							{@render bagPanel(generalBags, 'General inventory is empty.')}
						{:else}
							{@render bagPanel(bankBags, 'Bank is empty.')}
						{/if}
					</div>
				{/snippet}
			</Tabs>
		</div>
	{/if}
</div>

{#snippet slotCell(label: string, entry: (InventoryEntry & { key: string }) | null)}
	{#if entry}
		{@const icon = iconUrl(entry.item?.stats)}
		<a class="eq-slot eq-filled" href={itemPath(entry.name)}>
			{#if icon && !brokenIcons.has(icon)}
				<img
					class="eq-icon"
					src={icon}
					alt={shortName(entry.name)}
					width="40"
					height="40"
					loading="lazy"
					onerror={() => brokenIcons.add(icon)}
				/>
			{:else}
				<span class="eq-slot-name">{shortName(entry.name)}</span>
			{/if}
			{#if entry.upgrade}
				<span class="eq-upgrade">+{entry.upgrade}</span>
			{/if}
			<span class="eq-tooltip">
				<b>{entry.name}</b>
				{#each tooltipLines(entry) as line (line)}
					<span>{line}</span>
				{:else}
					<span class="eq-faint">not in the item database</span>
				{/each}
			</span>
		</a>
	{:else}
		<div class="eq-slot"><span class="eq-slot-label">{label.toUpperCase()}</span></div>
	{/if}
{/snippet}

{#snippet bisPanel()}
	{#if bis.isPending}
		<div class="eq-panel eq-empty">Searching the item database…</div>
	{:else if bis.isError}
		<div class="eq-panel eq-empty">Best-in-slot lookup failed: {bis.error.message}</div>
	{:else}
		<div class="eq-faint eq-bis-note">
			Top candidates from the eqlwiki item database for this loadout ({(inventory.data?.classes ?? []).join(
				'/'
			) || 'any class'}{level ? `, level ${level}` : ''}). Base values — merging adds up to +10% per
			tier on top.
		</div>
		<div class="eq-bags">
			{#each bis.data ?? [] as slot (slot.slot)}
				<div class="eq-panel eq-bag">
					<div class="eq-panel-title">{slot.slot}</div>
					{#if slot.candidates.length === 0}
						<div class="eq-faint">No usable items known for this slot.</div>
					{:else}
						<div class="eq-bis-list">
							{#each slot.candidates as candidate (candidate.id)}
								{@const icon = iconUrl(candidate.stats)}
								{@const owned = equippedBases.has(candidate.name.toLowerCase())}
								<a class="eq-bis-row" class:eq-bis-owned={owned} href={itemPath(candidate.name)}>
									{#if icon && !brokenIcons.has(icon)}
										<img
											class="eq-icon eq-bis-icon"
											src={icon}
											alt=""
											width="40"
											height="40"
											loading="lazy"
											onerror={() => brokenIcons.add(icon)}
										/>
									{:else}
										<span class="eq-bis-icon"></span>
									{/if}
									<span class="eq-bis-body">
										<span class="eq-bis-name">
											{candidate.name}
											{#if owned}<span class="eq-bis-tag">equipped</span>{/if}
										</span>
										<span class="eq-bis-stats">{candidateLine(candidate.stats)}</span>
										{#if candidate.stats.classes.length}
											<span class="eq-faint">{candidateClasses(candidate.stats)}</span>
										{/if}
									</span>
								</a>
							{/each}
						</div>
					{/if}
				</div>
			{/each}
		</div>
	{/if}
{/snippet}

{#snippet bagPanel(list: ReturnType<typeof bags<InventoryEntry & { key: string }>>, empty: string)}
	{#if list.length === 0}
		<div class="eq-panel eq-empty">{empty}</div>
	{:else}
		<div class="eq-bags">
			{#each list as bag (bag.key)}
				<div class="eq-panel eq-bag">
					<div class="eq-panel-title">
						{bag.label}
						{#if bag.container}
							· <a class="eq-baglink" href={itemPath(bag.container.item?.name ?? bag.container.name)}
								>{bag.container.name}</a>
						{/if}
					</div>
					{#if bag.contents.length}
						<div class="eq-bag-grid">
							{#each bag.contents as entry (entry.key)}
								{@render slotCell('', entry)}
							{/each}
						</div>
					{:else if bag.container}
						<div class="eq-faint">{bag.container.slots ? 'empty bag' : 'not a container'}</div>
					{/if}
				</div>
			{/each}
		</div>
	{/if}
{/snippet}

{#snippet weaponCard(label: string, weapon: WeaponSummary | null)}
	<Stack gap="var(--sp-1)">
		<Cluster gap="var(--sp-2)">
			<Text variant="caption" tone="muted">{label}</Text>
			{#if weapon}
				<Link href={itemPath(shortName(weapon.name))}>{weapon.name}</Link>
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
				Gear values include merge (+N) tier bonuses. HP, mana and endurance are from gear only —
				level-based vitals are not derivable from the dumps.
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
				<Card>
					<Stack gap="var(--sp-3)">
						<Heading level={3} size="sm">Weapons</Heading>
						{@render weaponCard('Primary', stats.primary)}
						{@render weaponCard('Secondary', stats.secondary)}
					</Stack>
				</Card>
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
			</AutoGrid>

			{#if restricted.length}
				<Card padding="none">
					<DataTable
						columns={classColumns}
						rows={restricted}
						rowKey={(row) => row.key}
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

{#snippet abilityType(row: AbilityRow)}
	<Cluster gap="var(--sp-1)">
		{#if row.category}
			<Badge tone={categoryTone(row.category)} size="sm">{row.category}</Badge>
		{:else}
			<Text tone="faint" variant="caption">—</Text>
		{/if}
		{#if row.proc}
			<Badge tone="neutral" size="sm">proc</Badge>
		{/if}
	</Cluster>
{/snippet}

{#snippet abilityShare(row: AbilityRow)}
	<Cluster gap="var(--sp-2)" wrap={false}>
		<Progress value={row.share * 100} max={100} size="sm" tone="accent" />
		<Text variant="caption" tone="muted">{percent(row.share)}</Text>
	</Cluster>
{/snippet}

{#snippet fightDetail(row: FightRow)}
	<Stack gap="var(--sp-3)">
		<Cluster gap="var(--sp-2)">
			{#if row.stance}
				<Badge tone="info" size="sm">stance · {row.stance}</Badge>
			{/if}
			{#if row.invocation}
				<Badge tone="info" size="sm">invocation · {row.invocation}</Badge>
			{/if}
			{#if !row.stance && !row.invocation}
				<Text variant="caption" tone="faint">No stance or invocation recorded.</Text>
			{/if}
		</Cluster>

		<AutoGrid min="9rem">
			<Metric label="Damage out" value={count(row.dmg_out)} icon="live" tone="info" sub="{rate(row.dps)} dps" />
			<Metric
				label="Damage taken"
				value={count(row.dmg_in)}
				icon="heart"
				tone="danger"
				sub="{rate(row.taken_per_sec)} per second"
			/>
			<Metric label="Healing done" value={count(row.heal_out)} icon="check-circle" tone="ok" />
			<Metric label="Kills" value={row.kills} icon="star" tone="ok" sub="{row.deaths} deaths" />
			<Metric
				label="Duration"
				value={clock(row.span)}
				icon="clock"
				sub="{clock(row.active_secs)} active"
			/>
		</AutoGrid>

		<Stack gap="var(--sp-1)">
			<Text variant="caption" tone="faint">Fought</Text>
			<Text variant="caption">{enemyList(row.enemies)}</Text>
			{#if row.allies.length}
				<Text variant="caption" tone="faint">Alongside {row.allies.join(', ')}</Text>
			{/if}
		</Stack>

		{#if row.abilities.length}
			<Card padding="none">
				<DataTable
					columns={abilityColumns}
					rows={row.abilities}
					rowKey={(ability) => ability.key}
					cellSnippets={{ category: abilityType, share: abilityShare }}
					empty="No damage broken down."
				/>
			</Card>
		{:else}
			<Text variant="caption" tone="faint">
				No damage of yours landed in this fight, so there is nothing to break down.
			</Text>
		{/if}

		{#if row.casts.length}
			<Stack gap="var(--sp-2)">
				<Text variant="caption" tone="faint">Spells cast</Text>
				<Cluster gap="var(--sp-2)">
					{#each row.casts as cast (cast.key)}
						<Badge size="sm" tone={cast.resists ? 'warn' : 'neutral'}>
							{cast.name} ×{cast.casts}{cast.resists ? ` · ${cast.resists} resisted` : ''}
						</Badge>
					{/each}
				</Cluster>
			</Stack>
		{/if}
	</Stack>
{/snippet}

{#snippet fightCard(row: FightRow)}
	{@const open = openFights.has(row.key)}
	<Card padding="none">
		<button
			type="button"
			class="fight-head"
			aria-expanded={open}
			onclick={() => toggleFight(row.key)}
		>
			<span class="fight-when">
				<Timestamp value={row.at} mode="datetime" />
				<span class="fight-zone">{row.zone ?? 'zone unknown'}</span>
			</span>
			<span class="fight-foes">{enemyList(row.enemies)}</span>
			<span class="fight-figures">
				<span class="fight-fig"><b>{count(row.dmg_out)}</b> out</span>
				<span class="fight-fig"><b>{count(row.dmg_in)}</b> in</span>
				<span class="fight-fig"><b>{rate(row.dps)}</b> dps</span>
				<span class="fight-fig"><b>{row.kills}</b> kills</span>
				{#if row.deaths}
					<span class="fight-fig fight-bad"><b>{row.deaths}</b> deaths</span>
				{/if}
				<span class="fight-fig">{clock(row.span)}</span>
			</span>
			<Icon name={open ? "chevron-up" : "chevron-down"} />
		</button>
		{#if open}
			<div class="fight-body">{@render fightDetail(row)}</div>
		{/if}
	</Card>
{/snippet}

{#snippet fightsPanel()}
	{#if fightPages.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading fights…</Text>
			</Cluster>
		</Card>
	{:else if fightPages.isError}
		<EmptyState
			title="No fight history"
			description={fightPages.error.message}
			icon="alert-circle"
			tone="warn"
			actionLabel="Retry"
			onAction={() => fightPages.refetch()}
		/>
	{:else if !fights.usable}
		<EmptyState
			title="No fights recorded yet"
			description="Fights are cut from the log by the EQL Log Reader combat tracker — switch on [tools.log_reader] in eqld and they appear as you play."
			icon="list"
		/>
	{:else}
		<Stack gap="var(--sp-3)">
			<Text variant="caption" tone="faint">
				One encounter per row, newest first. A fight starts on the first damage and ends after 45
				seconds with none; rates use active seconds only, so idle time between pulls is not counted
				against you.
			</Text>

			<AutoGrid min="10rem">
				<Metric label="Fights" value={fights.totals.fights} icon="list" />
				<Metric label="Damage out" value={count(fights.totals.dmg_out)} icon="live" tone="info" />
				<Metric label="DPS" value={rate(fights.totals.dps)} icon="star" tone="ok" sub="while active" />
				<Metric
					label="Damage taken"
					value={count(fights.totals.dmg_in)}
					icon="heart"
					tone="danger"
				/>
				<Metric label="Healing done" value={count(fights.totals.heal_out)} icon="check-circle" />
				<Metric
					label="Kills"
					value={fights.totals.kills}
					icon="check-circle"
					tone="ok"
					sub="{fights.totals.deaths} deaths"
				/>
			</AutoGrid>

			<Stack gap="var(--sp-2)">
				{#each fights.fights as row (row.key)}
					{@render fightCard(row)}
				{/each}
			</Stack>

			<Cluster justify="space-between">
				<Text variant="caption" tone="faint">{fights.fights.length} fights</Text>
				{#if fightPages.hasNextPage}
					<Button
						variant="ghost"
						size="sm"
						loading={fightPages.isFetchingNextPage}
						onclick={() => fightPages.fetchNextPage()}
					>
						Load older
					</Button>
				{/if}
			</Cluster>
		</Stack>
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

{#snippet shareBars(rows: ShareRow[], format: (value: number) => string)}
	<Stack gap="var(--sp-2)">
		{#each rows as row (row.key)}
			<Stack gap="var(--sp-1)">
				<Cluster justify="space-between">
					<Text variant="caption">{row.label}</Text>
					<Text variant="caption" tone="muted">{format(row.value)} · {percent(row.share)}</Text>
				</Cluster>
				<Progress value={row.share * 100} max={100} size="sm" tone="accent" />
			</Stack>
		{/each}
	</Stack>
{/snippet}

{#snippet shareCard(title: string, rows: ShareRow[], format: (value: number) => string)}
	<Card>
		<Stack gap="var(--sp-3)">
			<Heading level={3} size="sm">{title}</Heading>
			{@render shareBars(rows, format)}
		</Stack>
	</Card>
{/snippet}

{#snippet buildCard(row: BuildRow)}
	<Card>
		<Stack gap="var(--sp-3)">
			<Cluster justify="space-between">
				<Heading level={4} size="sm">{row.build}</Heading>
				<Badge tone="info" size="sm" mono>{rate(row.dps)} dps</Badge>
			</Cluster>
			<Cluster gap="var(--sp-3)">
				<Text variant="caption" tone="muted">{count(row.damage)} damage</Text>
				<Text variant="caption" tone="muted">{row.kills} kills · {row.deaths} deaths</Text>
				<Text variant="caption" tone="muted">{duration(row.combat_secs)} in combat</Text>
			</Cluster>
			{#if row.sources.length}
				{@render shareBars(row.sources, count)}
			{/if}
			{#if row.stances.length}
				<Stack gap="var(--sp-2)">
					<Text variant="caption" tone="faint">Stances</Text>
					{@render shareBars(row.stances, duration)}
				</Stack>
			{/if}
			{#if row.invocations.length}
				<Stack gap="var(--sp-2)">
					<Text variant="caption" tone="faint">Invocations</Text>
					{@render shareBars(row.invocations, duration)}
				</Stack>
			{/if}
		</Stack>
	</Card>
{/snippet}

{#snippet alltimePanel()}
	{#if !alltimeDoc.data}
		{@render harvestState(alltimeDoc, 'lifetime combat data')}
	{:else if !alltime.usable}
		{@render rawFallback('Lifetime', rawJson(alltimeDoc.data))}
	{:else}
		<Stack gap="var(--sp-3)">
			<Cluster justify="space-between">
				<Text variant="caption" tone="faint">
					Counted by the EQL Log Reader DPS meter across {alltime.builds.length}
					{alltime.builds.length === 1 ? 'build' : 'builds'} while it was running — time played without
					the meter is not included.
				</Text>
				<Badge tone="neutral">
					harvested <Timestamp value={alltimeDoc.data.captured_at} mode="relative" details={false} />
				</Badge>
			</Cluster>

			<AutoGrid min="10rem">
				<Metric label="Damage" value={count(alltime.totals.damage)} icon="live" tone="info" />
				<Metric
					label="DPS"
					value={rate(alltime.totals.dps)}
					icon="star"
					tone="ok"
					sub="while in combat"
				/>
				<Metric label="Kills" value={alltime.totals.kills} icon="check-circle" tone="ok" />
				<Metric label="Deaths" value={alltime.totals.deaths} icon="x-circle" tone="danger" />
				<Metric
					label="Accuracy"
					value={percent(alltime.totals.accuracy)}
					icon="eye"
					sub="{count(alltime.totals.hits)} of {count(
						alltime.totals.hits + alltime.totals.misses
					)} swings"
				/>
				<Metric
					label="In combat"
					value={duration(alltime.totals.combat_secs)}
					icon="clock"
					sub="biggest hit {alltime.totals.biggest}"
				/>
			</AutoGrid>

			<Card padding="none">
				<DataTable
					columns={buildColumns}
					rows={alltime.builds}
					rowKey={(row) => row.key}
					empty="No builds recorded."
					stickyHeader
				/>
			</Card>

			<AutoGrid min="18rem">
				{#if alltime.sources.length}
					{@render shareCard('Damage sources', alltime.sources, count)}
				{/if}
				{#if alltime.stances.length}
					{@render shareCard('Stance time', alltime.stances, duration)}
				{/if}
				{#if alltime.invocations.length}
					{@render shareCard('Invocation time', alltime.invocations, duration)}
				{/if}
			</AutoGrid>

			{#if alltime.builds.length > 1}
				<Stack gap="var(--sp-2)">
					<Heading level={3} size="sm">Build by build</Heading>
					<AutoGrid min="18rem">
						{#each alltime.builds as row (row.key)}
							{@render buildCard(row)}
						{/each}
					</AutoGrid>
				</Stack>
			{/if}
		</Stack>
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

<style>
	.eq {
		display: flex;
		flex-direction: column;
		gap: var(--sp-4, 1rem);
	}

	.eq-window {
		border-radius: 4px;
		--eq-gold: #96825a;
		--eq-gold-bright: #c9b37a;
		--eq-text: #d8cfae;
		--eq-green: #4ade4a;
		--eq-blue: #7db4f0;
		--eq-red: #e05252;
		--eq-panel: #1a1a1d;
		color: var(--eq-text);
		font-family: Georgia, 'Times New Roman', serif;
		background:
			radial-gradient(ellipse 80% 50% at 20% 10%, rgba(90, 90, 100, 0.25), transparent 60%),
			radial-gradient(ellipse 60% 40% at 80% 80%, rgba(70, 70, 85, 0.2), transparent 60%),
			radial-gradient(ellipse 40% 30% at 60% 30%, rgba(110, 110, 120, 0.12), transparent 70%),
			repeating-linear-gradient(
				115deg,
				#232326 0px,
				#1c1c1f 3px,
				#232327 7px,
				#18181b 11px,
				#212124 16px
			);
		border: 2px solid #3a3a3e;
		border-top-color: #55555a;
		border-left-color: #4a4a4f;
		outline: 1px solid #0c0c0e;
		box-shadow:
			inset 0 0 0 1px #0c0c0e,
			0 4px 18px rgba(0, 0, 0, 0.6);
		padding: var(--sp-3, 0.75rem);
		display: flex;
		flex-direction: column;
		gap: var(--sp-3, 0.75rem);
	}

	.eq-sheet {
		display: grid;
		grid-template-columns: minmax(13rem, 16rem) 1fr;
		gap: var(--sp-3, 0.75rem);
		align-items: start;
	}

	@media (max-width: 44rem) {
		.eq-sheet {
			grid-template-columns: 1fr;
		}
	}

	.eq-col {
		display: flex;
		flex-direction: column;
		gap: var(--sp-3, 0.75rem);
		min-width: 0;
	}

	.eq-panel {
		background: linear-gradient(160deg, #1d1d20, #151517 70%);
		border: 1px solid #060607;
		border-bottom-color: #48484d;
		border-right-color: #3c3c41;
		box-shadow:
			inset 0 1px 4px rgba(0, 0, 0, 0.7),
			inset 0 0 0 1px rgba(120, 110, 80, 0.12);
		border-radius: 2px;
		padding: 0.6rem 0.7rem;
	}

	.eq-panel-title {
		font-size: 0.72rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--eq-gold-bright);
		border-bottom: 1px solid rgba(150, 130, 90, 0.35);
		padding-bottom: 0.3rem;
		margin-bottom: 0.5rem;
		display: flex;
		justify-content: space-between;
		gap: 0.5rem;
		flex-wrap: wrap;
	}

	.eq-identity .eq-name {
		font-size: 1.3rem;
		color: var(--eq-gold-bright);
		text-shadow: 0 1px 2px #000;
	}

	.eq-sub {
		font-size: 0.8rem;
		color: var(--eq-text);
	}

	.eq-faint {
		color: #8a8470;
		font-size: 0.72rem;
		text-transform: none;
		letter-spacing: normal;
	}

	.eq-vitals {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		font-size: 0.85rem;
	}

	.eq-row {
		display: flex;
		justify-content: space-between;
	}

	.eq-green {
		color: var(--eq-green);
	}
	.eq-blue {
		color: var(--eq-blue);
	}
	.eq-red {
		color: var(--eq-red);
	}
	.eq-tan {
		color: var(--eq-text);
	}
	.eq-dim {
		color: #6f6a5a;
	}

	.eq-gearpart {
		font-size: 0.7rem;
		margin-right: 0.35rem;
	}

	.eq-grid {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}

	.eq-grid-row {
		display: flex;
		gap: 0.4rem;
		flex-wrap: wrap;
	}

	.eq-slot {
		position: relative;
		width: 3.5rem;
		height: 3.5rem;
		flex: 0 0 auto;
		display: flex;
		align-items: center;
		justify-content: center;
		text-align: center;
		padding: 0.25rem;
		background: linear-gradient(150deg, #212124, #101012 80%);
		border: 2px solid #050506;
		border-bottom-color: #4c4c52;
		border-right-color: #3e3e44;
		box-shadow: inset 0 2px 6px rgba(0, 0, 0, 0.8);
		border-radius: 2px;
		text-decoration: none;
	}

	.eq-slot-label {
		color: #55524a;
		font-size: 0.5rem;
		letter-spacing: 0.04em;
		overflow-wrap: anywhere;
	}

	.eq-filled {
		cursor: pointer;
	}

	.eq-filled:hover {
		outline: 1px solid var(--eq-gold-bright);
	}

	.eq-slot-name {
		color: var(--eq-text);
		font-size: 0.55rem;
		line-height: 1.1;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 4;
		line-clamp: 4;
		-webkit-box-orient: vertical;
	}

	.eq-icon {
		width: 40px;
		height: 40px;
		image-rendering: pixelated;
	}

	.eq-upgrade {
		position: absolute;
		top: 2px;
		right: 3px;
		color: var(--eq-gold-bright);
		font-size: 0.62rem;
		text-shadow: 0 1px 1px #000;
	}

	.eq-tooltip {
		display: none;
		position: absolute;
		z-index: 30;
		bottom: calc(100% + 4px);
		left: 50%;
		transform: translateX(-50%);
		min-width: 11rem;
		max-width: 16rem;
		flex-direction: column;
		gap: 0.1rem;
		padding: 0.5rem 0.6rem;
		font-size: 0.72rem;
		text-align: left;
		color: var(--eq-text);
		background: #131315;
		border: 1px solid var(--eq-gold);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.8);
	}

	.eq-tooltip b {
		color: var(--eq-gold-bright);
	}

	.eq-filled:hover .eq-tooltip {
		display: flex;
	}

	.eq-weight {
		margin-top: 0.6rem;
		display: flex;
		justify-content: space-between;
		font-size: 0.75rem;
		letter-spacing: 0.08em;
		border-top: 1px solid rgba(150, 130, 90, 0.35);
		padding-top: 0.4rem;
	}

	.eq-bags {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
		gap: 0.6rem;
	}

	.eq-bag-grid {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}

	.eq-baglink {
		color: var(--eq-gold-bright);
		text-decoration: none;
	}

	.eq-baglink:hover {
		text-decoration: underline;
	}

	.eq-bis-note {
		margin-bottom: 0.6rem;
	}

	.eq-bis-list {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
	}

	.eq-bis-row {
		display: flex;
		gap: 0.5rem;
		align-items: center;
		padding: 0.25rem 0.3rem;
		border: 1px solid transparent;
		border-radius: 2px;
		text-decoration: none;
		color: var(--eq-text);
	}

	.eq-bis-row:hover {
		border-color: var(--eq-gold-bright);
		background: rgba(150, 130, 90, 0.08);
	}

	.eq-bis-owned {
		background: rgba(74, 222, 74, 0.06);
	}

	.eq-bis-icon {
		width: 28px;
		height: 28px;
		flex: 0 0 auto;
	}

	.eq-bis-body {
		display: flex;
		flex-direction: column;
		gap: 0.05rem;
		min-width: 0;
	}

	.eq-bis-name {
		color: var(--eq-gold-bright);
		font-size: 0.8rem;
	}

	.eq-bis-tag {
		color: var(--eq-green);
		font-size: 0.62rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		margin-left: 0.4rem;
	}

	.eq-bis-stats {
		font-size: 0.72rem;
	}

	.eq-empty {
		color: #8a8470;
		font-size: 0.85rem;
	}

	.eq-tabpanel {
		padding-top: var(--sp-2, 0.5rem);
	}

	.fight-head {
		width: 100%;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--sp-2, 0.5rem) var(--sp-3, 0.75rem);
		padding: var(--sp-3, 0.75rem);
		background: none;
		border: 0;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	.fight-head:hover,
	.fight-head:focus-visible {
		background: var(--surface-2, rgba(255, 255, 255, 0.04));
	}

	.fight-when {
		display: flex;
		flex-direction: column;
		gap: 0.1rem;
		min-width: 0;
	}

	.fight-zone {
		font-size: 0.75rem;
		color: var(--text-muted, #8a8470);
	}

	.fight-foes {
		flex: 1 1 12rem;
		min-width: 0;
		font-size: 0.8rem;
		color: var(--text-muted, #8a8470);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.fight-figures {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem 0.75rem;
		font-size: 0.8rem;
		color: var(--text-muted, #8a8470);
	}

	.fight-fig b {
		color: var(--text, inherit);
		font-variant-numeric: tabular-nums;
	}

	.fight-bad b {
		color: var(--danger, #e05252);
	}

	.fight-body {
		padding: 0 var(--sp-3, 0.75rem) var(--sp-3, 0.75rem);
	}
</style>
