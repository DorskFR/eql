import type { FightView } from './api';

type Json = unknown;

const isRecord = (value: Json): value is Record<string, Json> =>
	typeof value === 'object' && value !== null && !Array.isArray(value);

const num = (value: Json): number => (typeof value === 'number' && Number.isFinite(value) ? value : 0);

const str = (value: Json): string | null => {
	if (typeof value !== 'string') return null;
	const trimmed = value.trim();
	return trimmed ? trimmed : null;
};

const strings = (value: Json): string[] =>
	Array.isArray(value) ? value.map(str).filter((entry): entry is string => entry !== null) : [];

const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

export interface AbilityRow {
	key: string;
	name: string;
	total: number;
	hits: number;
	crits: number;
	biggest: number;
	average: number | null;
	category: string | null;
	proc: boolean;
	share: number;
}

export interface CastRow {
	key: string;
	name: string;
	casts: number;
	resists: number;
}

export interface FightRow {
	key: string;
	id: number;
	at: string;
	zone: string | null;
	span: number;
	active_secs: number;
	dmg_out: number;
	dmg_in: number;
	heal_out: number;
	kills: number;
	deaths: number;
	dps: number | null;
	taken_per_sec: number | null;
	stance: string | null;
	invocation: string | null;
	enemies: string[];
	allies: string[];
	abilities: AbilityRow[];
	casts: CastRow[];
}

export interface FightTotals {
	fights: number;
	dmg_out: number;
	dmg_in: number;
	heal_out: number;
	kills: number;
	deaths: number;
	active_secs: number;
	dps: number | null;
}

export interface FightsProjection {
	usable: boolean;
	fights: FightRow[];
	totals: FightTotals;
}

function abilities(raw: Json, fightKey: string): AbilityRow[] {
	if (!isRecord(raw)) return [];
	const rows = Object.entries(raw).map(([name, entryRaw]) => {
		const entry = isRecord(entryRaw) ? entryRaw : {};
		const total = num(entry.total);
		const hits = num(entry.hits);
		return {
			key: `${fightKey}:${name}`,
			name,
			total,
			hits,
			crits: num(entry.crits),
			biggest: num(entry.biggest),
			average: hits > 0 ? total / hits : null,
			category: str(entry.category),
			proc: entry.proc === true,
			share: 0
		};
	});
	const damage = sum(rows.map((row) => row.total));
	for (const row of rows) row.share = damage > 0 ? row.total / damage : 0;
	rows.sort((a, b) => b.total - a.total || a.name.localeCompare(b.name));
	return rows;
}

function casts(castsRaw: Json, resistsRaw: Json, fightKey: string): CastRow[] {
	const resists = isRecord(resistsRaw) ? resistsRaw : {};
	const names = new Set<string>([
		...(isRecord(castsRaw) ? Object.keys(castsRaw) : []),
		...Object.keys(resists)
	]);
	const rows = [...names].map((name) => ({
		key: `${fightKey}:cast:${name}`,
		name,
		casts: isRecord(castsRaw) ? num(castsRaw[name]) : 0,
		resists: num(resists[name])
	}));
	rows.sort((a, b) => b.casts - a.casts || a.name.localeCompare(b.name));
	return rows;
}

export function projectFight(view: FightView): FightRow {
	const fight = isRecord(view.fight) ? view.fight : {};
	const key = String(view.id);
	const active = num(fight.active_secs);
	const dmgOut = num(fight.dmg_out_you);
	const dmgIn = num(fight.dmg_in_you);
	return {
		key,
		id: view.id,
		at: view.started_at,
		zone: str(fight.zone),
		span: num(fight.span),
		active_secs: active,
		dmg_out: dmgOut,
		dmg_in: dmgIn,
		heal_out: num(fight.heal_out),
		kills: num(fight.kills),
		deaths: num(fight.deaths),
		dps: active > 0 ? dmgOut / active : null,
		taken_per_sec: active > 0 ? dmgIn / active : null,
		stance: str(fight.stance),
		invocation: str(fight.invocation),
		enemies: strings(fight.enemies),
		allies: strings(fight.allies),
		abilities: abilities(fight.abilities_dmg, key),
		casts: casts(fight.spell_casts, fight.spell_resists, key)
	};
}

export function projectFights(views: FightView[] | undefined): FightsProjection {
	const fights = (views ?? []).map(projectFight);
	const active = sum(fights.map((fight) => fight.active_secs));
	const dmgOut = sum(fights.map((fight) => fight.dmg_out));
	return {
		usable: fights.length > 0,
		fights,
		totals: {
			fights: fights.length,
			dmg_out: dmgOut,
			dmg_in: sum(fights.map((fight) => fight.dmg_in)),
			heal_out: sum(fights.map((fight) => fight.heal_out)),
			kills: sum(fights.map((fight) => fight.kills)),
			deaths: sum(fights.map((fight) => fight.deaths)),
			active_secs: active,
			dps: active > 0 ? dmgOut / active : null
		}
	};
}

export function clock(seconds: number): string {
	const whole = Math.max(0, Math.round(seconds));
	const minutes = Math.floor(whole / 60);
	const rest = whole % 60;
	if (minutes < 60) return `${minutes}m ${String(rest).padStart(2, '0')}s`;
	return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`;
}

const CATEGORY_TONES: Record<string, 'neutral' | 'info' | 'ok' | 'warn' | 'danger'> = {
	melee: 'warn',
	skill: 'info',
	spell: 'info',
	poison: 'ok',
	ds: 'neutral'
};

export const categoryTone = (category: string | null) =>
	(category && CATEGORY_TONES[category]) || 'neutral';

export const enemyList = (enemies: string[]): string =>
	enemies.length ? enemies.join(', ') : 'nothing named';
