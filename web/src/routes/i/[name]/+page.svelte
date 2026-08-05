<script lang="ts">
	import { page } from '$app/state';
	import {
		AutoGrid,
		Badge,
		Button,
		Card,
		Cluster,
		EmptyState,
		Heading,
		Metric,
		Spinner,
		Stack,
		Text,
		Timestamp
	} from '@dorsk/tsumikit';
	import { ATTRIBUTES, damageDelay, flags, iconUrl, pairs, ratio, RESISTS } from '$lib/items';
	import { useItem } from '$lib/queries';

	const key = $derived(page.params.name ?? '');
	const item = useItem(() => key);

	const stats = $derived(item.data?.stats);
	const icon = $derived(iconUrl(stats));
	let brokenIcon = $state('');
	const attributes = $derived(stats ? pairs(stats, ATTRIBUTES) : []);
	const resists = $derived(stats ? pairs(stats, RESISTS) : []);
</script>

<Stack gap="var(--sp-4)">
	<Cluster justify="space-between">
		<Cluster gap="var(--sp-3)">
			<Button href="/" variant="ghost" size="sm">Back</Button>
			{#if icon && brokenIcon !== icon}
				<img
					class="item-icon"
					src={icon}
					alt=""
					width="40"
					height="40"
					onerror={() => (brokenIcon = icon)}
				/>
			{/if}
			<Heading level={2}>{item.data?.name ?? key}</Heading>
		</Cluster>
		{#if item.data}
			<Badge tone="neutral">
				scraped <Timestamp value={item.data.scraped_at} mode="relative" details={false} />
			</Badge>
		{/if}
	</Cluster>

	{#if item.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading item…</Text>
			</Cluster>
		</Card>
	{:else if item.isError || !stats}
		<EmptyState
			title="Item not in the database"
			description="Nothing scraped from eqlwiki matches this name yet."
			icon="search"
			tone="warn"
		/>
	{:else}
		<Cluster gap="var(--sp-2)">
			{#each flags(stats) as flag (flag)}
				<Badge tone="warn" uppercase size="sm">{flag}</Badge>
			{/each}
			{#each stats.slots as slot (slot)}
				<Badge tone="info" size="sm">{slot}</Badge>
			{/each}
			{#if stats.item_type}
				<Badge tone="info" size="sm">{stats.item_type}</Badge>
			{/if}
			{#if stats.required_level}
				<Badge tone="neutral" size="sm">Required level {stats.required_level}</Badge>
			{/if}
		</Cluster>

		<AutoGrid min="10rem">
			{#if stats.ac}
				<Metric label="AC" value={stats.ac} icon="lock" tone="info" />
			{/if}
			{#if stats.hp}
				<Metric label="HP" value={stats.hp} icon="heart" tone="ok" />
			{/if}
			{#if stats.mana}
				<Metric label="Mana" value={stats.mana} icon="star" tone="info" />
			{/if}
			{#if stats.endurance}
				<Metric label="Endurance" value={stats.endurance} icon="live" />
			{/if}
			{#if stats.damage || stats.delay}
				<Metric label="Dmg / Delay" value={damageDelay(stats)} sub={`ratio ${ratio(stats)}`} />
			{/if}
			{#if stats.haste}
				<Metric label="Haste" value={stats.haste} unit="%" icon="clock" />
			{/if}
			{#if stats.weight !== null}
				<Metric label="Weight" value={stats.weight} sub={stats.size ?? undefined} />
			{/if}
			{#if stats.capacity}
				<Metric
					label="Capacity"
					value={stats.capacity}
					sub={stats.size_capacity ?? undefined}
					icon="archive"
				/>
			{/if}
			{#if stats.weight_reduction}
				<Metric label="Weight reduction" value={stats.weight_reduction} unit="%" />
			{/if}
			{#if stats.hp_regen}
				<Metric label="HP regen" value={stats.hp_regen} tone="ok" />
			{/if}
			{#if stats.mana_regen}
				<Metric label="Mana regen" value={stats.mana_regen} tone="info" />
			{/if}
		</AutoGrid>

		{#if attributes.length}
			<Card>
				<Stack gap="var(--sp-2)">
					<Heading level={3} size="sm">Attributes</Heading>
					<Cluster gap="var(--sp-2)">
						{#each attributes as attribute (attribute.label)}
							<Badge tone="ok" mono>{attribute.label} +{attribute.value}</Badge>
						{/each}
					</Cluster>
				</Stack>
			</Card>
		{/if}

		{#if resists.length}
			<Card>
				<Stack gap="var(--sp-2)">
					<Heading level={3} size="sm">Resists</Heading>
					<Cluster gap="var(--sp-2)">
						{#each resists as resist (resist.label)}
							<Badge tone="info" mono>{resist.label} +{resist.value}</Badge>
						{/each}
					</Cluster>
				</Stack>
			</Card>
		{/if}

		{#if stats.effects.length}
			<Card>
				<Stack gap="var(--sp-2)">
					<Heading level={3} size="sm">Effects</Heading>
					{#each stats.effects as effect (effect.name)}
						<Cluster gap="var(--sp-2)">
							<Badge tone="info" uppercase size="sm">{effect.kind}</Badge>
							<Text weight="medium">{effect.name}</Text>
							{#if effect.restriction}
								<Text tone="muted" variant="caption">{effect.restriction}</Text>
							{/if}
							{#if effect.casting_time}
								<Text tone="muted" variant="caption">cast {effect.casting_time}</Text>
							{/if}
							{#if effect.level}
								<Text tone="muted" variant="caption">level {effect.level}</Text>
							{/if}
							{#if effect.cooldown_seconds}
								<Text tone="muted" variant="caption">cooldown {effect.cooldown_seconds}s</Text>
							{/if}
						</Cluster>
					{/each}
				</Stack>
			</Card>
		{/if}

		<Card>
			<Stack gap="var(--sp-2)">
				<Heading level={3} size="sm">Restrictions</Heading>
				<Cluster gap="var(--sp-2)">
					<Text tone="muted" variant="caption">Classes</Text>
					{#each stats.classes.length ? stats.classes : ['—'] as klass (klass)}
						<Badge size="sm">{klass}</Badge>
					{/each}
				</Cluster>
				<Cluster gap="var(--sp-2)">
					<Text tone="muted" variant="caption">Races</Text>
					{#each stats.races.length ? stats.races : ['—'] as race (race)}
						<Badge size="sm">{race}</Badge>
					{/each}
				</Cluster>
				{#if stats.deity}
					<Cluster gap="var(--sp-2)">
						<Text tone="muted" variant="caption">Deity</Text>
						<Badge size="sm">{stats.deity}</Badge>
					</Cluster>
				{/if}
			</Stack>
		</Card>
	{/if}
</Stack>

<style>
	.item-icon {
		width: 2.5rem;
		height: 2.5rem;
		image-rendering: pixelated;
	}
</style>
