<script lang="ts">
	import { page } from '$app/state';
	import { Timestamp } from '@dorsk/tsumikit';
	import type { ItemStats } from '$lib/api';
	import { ATTRIBUTES, flags, iconUrl, ratio, RESISTS } from '$lib/items';
	import { useItem } from '$lib/queries';

	const key = $derived(page.params.name ?? '');
	const item = useItem(() => key);

	const stats = $derived(item.data?.stats);
	const icon = $derived(iconUrl(stats));
	let brokenIcon = $state('');

	const title = $derived(
		item.data ? `${item.data.name}${item.data.upgrade ? ` +${item.data.upgrade}` : ''}` : key
	);

	const signed = (value: number) => (value > 0 ? `+${value}` : String(value));

	const statLine = (s: ItemStats, keys: readonly (readonly [keyof ItemStats, string])[]) =>
		keys
			.map(([k, label]) => ({ label, value: s[k] as number | null }))
			.filter((pair) => pair.value)
			.map((pair) => `${pair.label}: ${signed(pair.value as number)}`)
			.join('  ');

	const vitalsLine = (s: ItemStats) =>
		statLine(s, [
			['hp', 'HP'],
			['mana', 'MANA'],
			['endurance', 'END'],
			['hp_regen', 'HP REGEN'],
			['mana_regen', 'MANA REGEN']
		] as const);

	const resistLine = (s: ItemStats) =>
		statLine(s, RESISTS.map(([k, label]) => [k, `SV ${label.toUpperCase()}`] as const));
</script>

<div class="eq-window eq-inspect">
	<a class="eq-back" href="/">« Back</a>

	{#if item.isPending}
		<div class="eq-panel eq-faint">Consulting the item database…</div>
	{:else if item.isError || !stats}
		<div class="eq-panel">
			<div class="eq-title">{decodeURIComponent(key)}</div>
			<div class="eq-faint">Item not in the database — nothing scraped from eqlwiki matches this name yet.</div>
		</div>
	{:else}
		<div class="eq-panel">
			<div class="eq-head">
				{#if icon && brokenIcon !== icon}
					<img
						class="eq-icon"
						src={icon}
						alt=""
						width="40"
						height="40"
						onerror={() => (brokenIcon = icon)}
					/>
				{/if}
				<span class="eq-title">{title}</span>
			</div>

			{#if flags(stats).length}
				<div class="eq-line eq-flags">{flags(stats).map((f) => f.toUpperCase()).join('  ')}</div>
			{/if}
			{#if stats.slots.length}
				<div class="eq-line">Slot: {stats.slots.join(' ')}</div>
			{/if}
			{#if stats.item_type}
				<div class="eq-line">Skill: {stats.item_type}{stats.delay ? `  Atk Delay: ${stats.delay}` : ''}</div>
			{/if}
			{#if stats.damage}
				<div class="eq-line">DMG: {stats.damage}{ratio(stats) ? `  (ratio ${ratio(stats)})` : ''}</div>
			{/if}
			{#if stats.backstab}
				<div class="eq-line">Backstab DMG: {stats.backstab}</div>
			{/if}
			{#if stats.range}
				<div class="eq-line">Range: {stats.range}</div>
			{/if}
			{#if stats.ac}
				<div class="eq-line">AC: {stats.ac}</div>
			{/if}
			{#if statLine(stats, ATTRIBUTES)}
				<div class="eq-line">{statLine(stats, ATTRIBUTES)}</div>
			{/if}
			{#if vitalsLine(stats)}
				<div class="eq-line">{vitalsLine(stats)}</div>
			{/if}
			{#if resistLine(stats)}
				<div class="eq-line">{resistLine(stats)}</div>
			{/if}
			{#if stats.haste}
				<div class="eq-line">Haste: +{stats.haste}%</div>
			{/if}
			{#if stats.weight_reduction}
				<div class="eq-line">Weight Reduction: {stats.weight_reduction}%</div>
			{/if}
			{#if stats.capacity}
				<div class="eq-line">
					Capacity: {stats.capacity}{stats.size_capacity ? `  Size Capacity: ${stats.size_capacity}` : ''}
				</div>
			{/if}
			{#if stats.charges}
				<div class="eq-line">Charges: {stats.charges}</div>
			{/if}
			{#if stats.weight !== null || stats.size}
				<div class="eq-line">
					{stats.weight !== null ? `WT: ${stats.weight}` : ''}{stats.size ? `  Size: ${stats.size}` : ''}
				</div>
			{/if}
			<div class="eq-line">Class: {stats.classes.length ? stats.classes.join(' ') : 'ALL'}</div>
			<div class="eq-line">Race: {stats.races.length ? stats.races.join(' ') : 'ALL'}</div>
			{#if stats.deity}
				<div class="eq-line">Deity: {stats.deity}</div>
			{/if}

			{#each stats.effects as effect (effect.name)}
				<div class="eq-line eq-effect">
					{effect.kind === 'worn' ? 'Worn Effect' : 'Effect'}: {effect.name}
					{#if effect.casting_time}(Casting Time: {effect.casting_time}){/if}
					{#if effect.level}at Level {effect.level}{/if}
					{#if effect.cooldown_seconds}— cooldown {effect.cooldown_seconds}s{/if}
					{#if effect.restriction}<span class="eq-faint"> {effect.restriction}</span>{/if}
				</div>
			{/each}
			{#if stats.focus_effect}
				<div class="eq-line eq-effect">Focus Effect: {stats.focus_effect}</div>
			{/if}
			{#if stats.required_level}
				<div class="eq-line">Required level of {stats.required_level}.</div>
			{/if}

			<div class="eq-foot">
				{#if item.data?.upgrade}
					<span>merge tier +{item.data.upgrade} applied</span>
				{/if}
				<span>
					scraped <Timestamp value={item.data?.scraped_at ?? ''} mode="relative" details={false} />
				</span>
			</div>
		</div>
	{/if}
</div>

<style>
	.eq-window {
		--eq-gold: #96825a;
		--eq-gold-bright: #c9b37a;
		--eq-text: #d8cfae;
		color: var(--eq-text);
		font-family: Georgia, 'Times New Roman', serif;
		background:
			radial-gradient(ellipse 80% 50% at 20% 10%, rgba(90, 90, 100, 0.25), transparent 60%),
			radial-gradient(ellipse 60% 40% at 80% 80%, rgba(70, 70, 85, 0.2), transparent 60%),
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
		border-radius: 4px;
		padding: var(--sp-3, 0.75rem);
		display: flex;
		flex-direction: column;
		gap: var(--sp-3, 0.75rem);
	}

	.eq-inspect {
		max-width: 34rem;
		margin: 0 auto;
	}

	.eq-back {
		color: var(--eq-gold-bright);
		font-size: 0.8rem;
		text-decoration: none;
		align-self: flex-start;
	}

	.eq-back:hover {
		text-decoration: underline;
	}

	.eq-panel {
		background: #131315;
		border: 1px solid var(--eq-gold);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.8);
		padding: 0.7rem 0.85rem;
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		font-size: 0.85rem;
	}

	.eq-head {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin-bottom: 0.3rem;
	}

	.eq-title {
		color: var(--eq-gold-bright);
		font-size: 1.05rem;
		text-shadow: 0 1px 2px #000;
	}

	.eq-icon {
		width: 40px;
		height: 40px;
		image-rendering: pixelated;
	}

	.eq-line {
		line-height: 1.35;
	}

	.eq-flags {
		letter-spacing: 0.04em;
	}

	.eq-effect {
		color: var(--eq-gold-bright);
	}

	.eq-faint {
		color: #8a8470;
		font-size: 0.75rem;
	}

	.eq-foot {
		margin-top: 0.5rem;
		padding-top: 0.4rem;
		border-top: 1px solid rgba(150, 130, 90, 0.35);
		display: flex;
		justify-content: space-between;
		gap: 0.5rem;
		color: #8a8470;
		font-size: 0.72rem;
	}
</style>
