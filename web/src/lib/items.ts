import type { GearStats, InventoryEntry, ItemStats, WeaponSummary } from './api';

export function itemPath(name: string): string {
	return `/i/${encodeURIComponent(name)}`;
}

export function damageDelay(stats: ItemStats | undefined): string {
	if (!stats?.damage && !stats?.delay) return '';
	return `${stats.damage ?? '?'}/${stats.delay ?? '?'}`;
}

export function ratio(stats: ItemStats | undefined): string {
	if (!stats?.damage || !stats?.delay) return '';
	return (stats.damage / stats.delay).toFixed(2);
}

export interface EquippedTotals {
	ac: number;
	hp: number;
	mana: number;
	weight: number;
	known: number;
	unknown: number;
}

export function equippedTotals(entries: InventoryEntry[]): EquippedTotals {
	const totals: EquippedTotals = { ac: 0, hp: 0, mana: 0, weight: 0, known: 0, unknown: 0 };
	for (const entry of entries) {
		if (entry.name === 'Empty') continue;
		const stats = entry.item?.stats;
		if (!stats) {
			totals.unknown += 1;
			continue;
		}
		totals.known += 1;
		totals.ac += stats.ac ?? 0;
		totals.hp += stats.hp ?? 0;
		totals.mana += stats.mana ?? 0;
		totals.weight += stats.weight ?? 0;
	}
	totals.weight = Math.round(totals.weight * 10) / 10;
	return totals;
}

export const RESISTS = [
	['sv_fire', 'Fire'],
	['sv_cold', 'Cold'],
	['sv_magic', 'Magic'],
	['sv_disease', 'Disease'],
	['sv_poison', 'Poison']
] as const;

export const ATTRIBUTES = [
	['str', 'STR'],
	['sta', 'STA'],
	['agi', 'AGI'],
	['dex', 'DEX'],
	['wis', 'WIS'],
	['int', 'INT'],
	['cha', 'CHA']
] as const;

export function pairs(
	stats: ItemStats,
	keys: readonly (readonly [keyof ItemStats, string])[]
): { label: string; value: number }[] {
	return keys
		.map(([key, label]) => ({ label, value: stats[key] as number | null }))
		.filter((pair): pair is { label: string; value: number } => Boolean(pair.value));
}

export interface StatRow {
	label: string;
	value: number;
}

export function gearPairs(
	stats: GearStats,
	keys: readonly (readonly [keyof GearStats, string])[]
): StatRow[] {
	return keys.map(([key, label]) => ({ label, value: stats[key] as number }));
}

export function weaponRatio(weapon: WeaponSummary): string {
	return weapon.ratio === null ? '—' : weapon.ratio.toFixed(2);
}

export function weaponDamageDelay(weapon: WeaponSummary): string {
	return `${weapon.damage ?? '?'}/${weapon.delay ?? '?'}`;
}

export function flags(stats: ItemStats): string[] {
	const named: [boolean, string][] = [
		[stats.magic, 'Magic'],
		[stats.lore, 'Lore'],
		[stats.no_drop, 'No Drop'],
		[stats.no_trade, 'No Trade'],
		[stats.quest_item, 'Quest'],
		[stats.temporary, 'Temporary'],
		[stats.expendable, 'Expendable']
	];
	return named.filter(([on]) => on).map(([, label]) => label);
}
