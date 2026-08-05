import type { HarvestDoc } from './api';

export type Json = unknown;

const isRecord = (value: Json): value is Record<string, Json> =>
	typeof value === 'object' && value !== null && !Array.isArray(value);

const num = (value: Json): number | null =>
	typeof value === 'number' && Number.isFinite(value) ? value : null;

const str = (value: Json): string | null => (typeof value === 'string' ? value : null);

const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

export interface ZoneRow {
	key: string;
	zone: string;
	kills: number;
	group_kills: number;
	coin_copper: number;
	loots: number;
	mobs: number;
	last_seen: number | null;
}

export interface AtlasProjection {
	usable: boolean;
	zones: ZoneRow[];
	kills: number;
	group_kills: number;
	loots: number;
	coin_copper: number;
	top_drops: DropRow[];
}

export interface DropRow {
	key: string;
	item: string;
	zone: string;
	mob: string;
	count: number;
	sold_copper: number;
}

export function coin(copper: number): string {
	if (!copper) return '0c';
	const parts: string[] = [];
	const units: [number, string][] = [
		[1000, 'p'],
		[100, 'g'],
		[10, 's'],
		[1, 'c']
	];
	let left = Math.floor(copper);
	for (const [size, label] of units) {
		const amount = Math.floor(left / size);
		if (amount) parts.push(`${amount}${label}`);
		left -= amount * size;
	}
	return parts.join(' ');
}

export function projectAtlas(doc: Json): AtlasProjection {
	const empty: AtlasProjection = {
		usable: false,
		zones: [],
		kills: 0,
		group_kills: 0,
		loots: 0,
		coin_copper: 0,
		top_drops: []
	};
	if (!isRecord(doc)) return empty;
	const zonesRaw = doc.zones;
	if (!isRecord(zonesRaw)) return empty;

	const zones: ZoneRow[] = [];
	const drops: DropRow[] = [];
	for (const [key, zoneRaw] of Object.entries(zonesRaw)) {
		if (!isRecord(zoneRaw)) continue;
		const zone = str(zoneRaw.long) ?? key;
		const mobsRaw = isRecord(zoneRaw.mobs) ? zoneRaw.mobs : {};
		const mobs = Object.entries(mobsRaw).filter(([, mob]) => isRecord(mob)) as [
			string,
			Record<string, Json>
		][];

		let loots = 0;
		for (const [mobKey, mob] of mobs) {
			const mobName = str(mob.name) ?? mobKey;
			const dropsRaw = isRecord(mob.drops) ? mob.drops : {};
			for (const [item, dropRaw] of Object.entries(dropsRaw)) {
				if (!isRecord(dropRaw)) continue;
				const count = num(dropRaw.count) ?? 0;
				loots += count;
				drops.push({
					key: `${key}:${mobKey}:${item}`,
					item,
					zone,
					mob: mobName,
					count,
					sold_copper: num(dropRaw.sold_copper) ?? 0
				});
			}
		}

		zones.push({
			key,
			zone,
			kills: sum(mobs.map(([, mob]) => num(mob.kills) ?? 0)),
			group_kills: sum(mobs.map(([, mob]) => num(mob.kills_group) ?? 0)),
			coin_copper: sum(mobs.map(([, mob]) => num(mob.coin_copper) ?? 0)),
			loots,
			mobs: mobs.length,
			last_seen: mobs.length
				? Math.max(...mobs.map(([, mob]) => num(mob.last_seen) ?? 0)) || null
				: null
		});
	}

	zones.sort((a, b) => b.kills - a.kills || a.zone.localeCompare(b.zone));
	drops.sort((a, b) => b.count - a.count || a.item.localeCompare(b.item));

	const totals = isRecord(doc.totals) ? doc.totals : {};
	return {
		usable: zones.length > 0,
		zones,
		kills: num(totals.kills) ?? sum(zones.map((zone) => zone.kills)),
		group_kills: num(totals.kills_group) ?? sum(zones.map((zone) => zone.group_kills)),
		loots: num(totals.loots) ?? sum(zones.map((zone) => zone.loots)),
		coin_copper: num(totals.coin_copper) ?? sum(zones.map((zone) => zone.coin_copper)),
		top_drops: drops.slice(0, 25)
	};
}

export interface QuestRow {
	key: string;
	quest: string;
	have: number;
	need: number | null;
	ratio: number | null;
	tracked: boolean;
	confirmed: boolean;
	added: number | null;
}

export interface QuestProjection {
	usable: boolean;
	quests: QuestRow[];
	tracked: string | null;
	confirmed: number;
}

// Required counts live in the shipped quest DB, not the per-character file,
// so `need` stays null unless a future schema carries it.
export function projectQuest(doc: Json): QuestProjection {
	const empty: QuestProjection = { usable: false, quests: [], tracked: null, confirmed: 0 };
	if (!isRecord(doc)) return empty;
	const questsRaw = doc.quests;
	if (!isRecord(questsRaw)) return empty;

	const order = Array.isArray(doc.order) ? doc.order.map(String) : [];
	const confirmedRaw = isRecord(doc.confirmed) ? doc.confirmed : {};
	const tracked = str(doc.current);

	const rows: QuestRow[] = Object.entries(questsRaw).map(([key, entryRaw]) => {
		const entry = isRecord(entryRaw) ? entryRaw : {};
		const haveRaw = isRecord(entry.have) ? entry.have : {};
		const have = sum(Object.values(haveRaw).map((value) => num(value) ?? 0));
		const needRaw = isRecord(entry.need) ? entry.need : null;
		const need = needRaw
			? sum(Object.values(needRaw).map((value) => num(value) ?? 0))
			: (num(entry.need) ?? null);
		return {
			key,
			quest: str(entry.name) ?? `Quest ${key}`,
			have,
			need,
			ratio: need && need > 0 ? Math.min(1, have / need) : null,
			tracked: tracked === key,
			confirmed: key in confirmedRaw,
			added: num(entry.added)
		};
	});

	const rank = (row: QuestRow) => {
		const index = order.indexOf(row.key);
		return index === -1 ? order.length : index;
	};
	rows.sort((a, b) => rank(a) - rank(b) || a.key.localeCompare(b.key));

	return {
		usable: rows.length > 0,
		quests: rows,
		tracked,
		confirmed: Object.keys(confirmedRaw).length
	};
}

export interface ShareRow {
	key: string;
	label: string;
	value: number;
	share: number;
}

export interface CombatTotals {
	hits: number;
	misses: number;
	crits: number;
	kills: number;
	deaths: number;
	biggest: number;
	combat_secs: number;
	damage: number;
	accuracy: number | null;
	dps: number | null;
	crit_rate: number | null;
	kill_death: number | null;
}

export interface BuildRow extends CombatTotals {
	key: string;
	build: string;
	sources: ShareRow[];
	stances: ShareRow[];
	invocations: ShareRow[];
}

export interface AlltimeProjection {
	usable: boolean;
	builds: BuildRow[];
	sources: ShareRow[];
	stances: ShareRow[];
	invocations: ShareRow[];
	totals: CombatTotals;
}

// The meter writes classes as WAR-CLR because a slash cannot go in a filename.
export const buildLabel = (key: string): string => key.split('-').join(' / ');

const numbers = (raw: Json): Record<string, number> => {
	if (!isRecord(raw)) return {};
	const out: Record<string, number> = {};
	for (const [key, value] of Object.entries(raw)) {
		const parsed = num(value);
		if (parsed !== null && parsed > 0) out[key] = parsed;
	}
	return out;
};

function shares(prefix: string, map: Record<string, number>): ShareRow[] {
	const total = sum(Object.values(map));
	return Object.entries(map)
		.map(([label, value]) => ({
			key: `${prefix}:${label}`,
			label,
			value,
			share: total > 0 ? value / total : 0
		}))
		.sort((a, b) => b.value - a.value || a.label.localeCompare(b.label));
}

function merge(maps: Record<string, number>[]): Record<string, number> {
	const out: Record<string, number> = {};
	for (const map of maps) {
		for (const [key, value] of Object.entries(map)) out[key] = (out[key] ?? 0) + value;
	}
	return out;
}

function totalsOf(
	counts: Omit<CombatTotals, 'accuracy' | 'dps' | 'crit_rate' | 'kill_death'>
): CombatTotals {
	const swings = counts.hits + counts.misses;
	return {
		...counts,
		accuracy: swings > 0 ? counts.hits / swings : null,
		dps: counts.combat_secs > 0 && counts.damage > 0 ? counts.damage / counts.combat_secs : null,
		crit_rate: counts.hits > 0 ? counts.crits / counts.hits : null,
		kill_death: counts.deaths > 0 ? counts.kills / counts.deaths : null
	};
}

function buildRow(key: string, build: string, raw: Record<string, Json>): BuildRow {
	const sourceDmg = numbers(raw.source_dmg);
	return {
		...totalsOf({
			hits: num(raw.hits) ?? 0,
			misses: num(raw.misses) ?? 0,
			crits: num(raw.crits) ?? 0,
			kills: num(raw.kills) ?? 0,
			deaths: num(raw.deaths) ?? 0,
			biggest: num(raw.biggest) ?? 0,
			combat_secs: num(raw.combat_secs) ?? 0,
			damage: sum(Object.values(sourceDmg))
		}),
		key,
		build,
		sources: shares(`${key}:src`, sourceDmg),
		stances: shares(`${key}:stance`, numbers(raw.stance_secs)),
		invocations: shares(`${key}:inv`, numbers(raw.invocation_secs))
	};
}

const ALLTIME_KEYS = ['hits', 'misses', 'crits', 'kills', 'deaths', 'biggest', 'source_dmg'];

export function projectAlltime(doc: Json): AlltimeProjection {
	const zero = totalsOf({
		hits: 0,
		misses: 0,
		crits: 0,
		kills: 0,
		deaths: 0,
		biggest: 0,
		combat_secs: 0,
		damage: 0
	});
	const empty: AlltimeProjection = {
		usable: false,
		builds: [],
		sources: [],
		stances: [],
		invocations: [],
		totals: zero
	};
	if (!isRecord(doc)) return empty;

	const builds: BuildRow[] = [];
	if (isRecord(doc.builds)) {
		for (const [key, raw] of Object.entries(doc.builds)) {
			if (isRecord(raw)) builds.push(buildRow(key, buildLabel(key), raw));
		}
	} else if (ALLTIME_KEYS.some((key) => key in doc)) {
		const key = str(doc.build);
		builds.push(buildRow('current', key ? buildLabel(key) : 'Current build', doc));
	}
	if (!builds.length) return empty;

	builds.sort((a, b) => b.damage - a.damage || a.build.localeCompare(b.build));

	const totals = totalsOf({
		hits: sum(builds.map((row) => row.hits)),
		misses: sum(builds.map((row) => row.misses)),
		crits: sum(builds.map((row) => row.crits)),
		kills: sum(builds.map((row) => row.kills)),
		deaths: sum(builds.map((row) => row.deaths)),
		biggest: Math.max(...builds.map((row) => row.biggest)),
		combat_secs: sum(builds.map((row) => row.combat_secs)),
		damage: sum(builds.map((row) => row.damage))
	});

	const raws = isRecord(doc.builds) ? Object.values(doc.builds) : [doc];
	const pick = (field: string) => merge(raws.map((raw) => numbers(isRecord(raw) ? raw[field] : null)));

	return {
		usable: true,
		builds,
		sources: shares('src', pick('source_dmg')),
		stances: shares('stance', pick('stance_secs')),
		invocations: shares('inv', pick('invocation_secs')),
		totals
	};
}

export const rawJson = (doc: HarvestDoc | undefined) =>
	doc ? JSON.stringify(doc.doc, null, 2) : '';

export const duration = (seconds: number): string => {
	if (!seconds || seconds < 0) return '0m';
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.round((seconds % 3600) / 60);
	return hours ? `${hours}h ${minutes}m` : `${minutes}m`;
};

export const percent = (ratio: number | null): string =>
	ratio === null ? '—' : `${(ratio * 100).toFixed(1)}%`;
