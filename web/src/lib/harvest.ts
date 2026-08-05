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

export interface BuildRow {
	key: string;
	build: string;
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
}

export interface AlltimeProjection {
	usable: boolean;
	builds: BuildRow[];
	sources: { key: string; source: string; damage: number; share: number }[];
}

function buildRow(key: string, build: string, raw: Record<string, Json>): BuildRow {
	const hits = num(raw.hits) ?? 0;
	const misses = num(raw.misses) ?? 0;
	const combat = num(raw.combat_secs) ?? 0;
	const sourceDmg = isRecord(raw.source_dmg) ? raw.source_dmg : {};
	const damage = sum(Object.values(sourceDmg).map((value) => num(value) ?? 0));
	const swings = hits + misses;
	return {
		key,
		build,
		hits,
		misses,
		crits: num(raw.crits) ?? 0,
		kills: num(raw.kills) ?? 0,
		deaths: num(raw.deaths) ?? 0,
		biggest: num(raw.biggest) ?? 0,
		combat_secs: combat,
		damage,
		accuracy: swings > 0 ? hits / swings : null,
		dps: combat > 0 && damage > 0 ? damage / combat : null
	};
}

export function projectAlltime(doc: Json): AlltimeProjection {
	const empty: AlltimeProjection = { usable: false, builds: [], sources: [] };
	if (!isRecord(doc)) return empty;

	const builds: BuildRow[] = [];
	if (isRecord(doc.builds)) {
		for (const [key, raw] of Object.entries(doc.builds)) {
			if (isRecord(raw)) builds.push(buildRow(key, key, raw));
		}
	} else if ('hits' in doc || 'kills' in doc || 'source_dmg' in doc) {
		builds.push(buildRow('current', str(doc.build) ?? 'Current build', doc));
	}
	if (!builds.length) return empty;

	builds.sort((a, b) => b.damage - a.damage || a.build.localeCompare(b.build));

	const totals = new Map<string, number>();
	const withSources = isRecord(doc.builds) ? Object.values(doc.builds) : [doc];
	for (const raw of withSources) {
		if (!isRecord(raw) || !isRecord(raw.source_dmg)) continue;
		for (const [source, value] of Object.entries(raw.source_dmg)) {
			totals.set(source, (totals.get(source) ?? 0) + (num(value) ?? 0));
		}
	}
	const grand = sum([...totals.values()]);
	const sources = [...totals.entries()]
		.map(([source, damage]) => ({
			key: source,
			source,
			damage,
			share: grand > 0 ? damage / grand : 0
		}))
		.sort((a, b) => b.damage - a.damage);

	return { usable: true, builds, sources };
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
