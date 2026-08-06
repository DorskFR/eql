export interface ItemEffect {
	kind: string;
	name: string;
	restriction: string | null;
	casting_time: string | null;
	level: number | null;
	cooldown_seconds: number | null;
}

export interface ItemStats {
	name: string;
	slots: string[];
	classes: string[];
	races: string[];
	deity: string | null;
	item_type: string | null;
	ac: number | null;
	hp: number | null;
	mana: number | null;
	endurance: number | null;
	hp_regen: number | null;
	mana_regen: number | null;
	str: number | null;
	sta: number | null;
	agi: number | null;
	dex: number | null;
	wis: number | null;
	int: number | null;
	cha: number | null;
	sv_fire: number | null;
	sv_cold: number | null;
	sv_magic: number | null;
	sv_disease: number | null;
	sv_poison: number | null;
	damage: number | null;
	delay: number | null;
	backstab: number | null;
	range: number | null;
	haste: number | null;
	weight: number | null;
	size: string | null;
	capacity: number | null;
	size_capacity: string | null;
	weight_reduction: number | null;
	charges: number | null;
	required_level: number | null;
	magic: boolean;
	lore: boolean;
	no_drop: boolean;
	no_trade: boolean;
	temporary: boolean;
	expendable: boolean;
	quest_item: boolean;
	icon: number | null;
	effects: ItemEffect[];
	focus_effect: string | null;
	unparsed: string[];
}

export interface ItemRecord {
	id: number;
	game_id: number | null;
	name: string;
	stats: ItemStats;
	scraped_at: string;
}

export interface InventoryEntry {
	location: string;
	name: string;
	id: number;
	count: number;
	slots: number;
	item?: ItemRecord;
	upgrade?: number;
}

export interface CharacterSummary {
	name: string;
	server: string;
	last_snapshot_at: string | null;
	snapshot_count: number;
}

export interface CharacterView {
	name: string;
	server: string;
	level: number | null;
	race: string | null;
	classes: string[];
	identity_at: string | null;
}

export interface InventoryView {
	character: string;
	server: string;
	captured_at: string;
	entries: InventoryEntry[];
}

export interface WeaponSummary {
	name: string;
	item_type: string | null;
	damage: number | null;
	delay: number | null;
	ratio: number | null;
}

export interface ItemClasses {
	location: string;
	name: string;
	classes: string[];
}

export interface GearStats {
	ac: number;
	hp: number;
	mana: number;
	endurance: number;
	hp_regen: number;
	mana_regen: number;
	str: number;
	sta: number;
	agi: number;
	dex: number;
	wis: number;
	int: number;
	cha: number;
	sv_fire: number;
	sv_cold: number;
	sv_magic: number;
	sv_disease: number;
	sv_poison: number;
	haste: number;
	weight: number;
	equipped_count: number;
	known_items: number;
	unknown_items: number;
	primary: WeaponSummary | null;
	secondary: WeaponSummary | null;
	usable_by: string[];
	no_single_class_can_use_all: boolean;
	min_classes_needed: number | null;
	item_classes: ItemClasses[];
}

export interface StatsView {
	character: string;
	server: string;
	captured_at: string;
	stats: GearStats;
	equipped: InventoryEntry[];
}

export type LogEventKind =
	| 'loot'
	| 'level'
	| 'zone'
	| 'death'
	| 'location'
	| 'skill'
	| 'who';

export interface LogEventPayload {
	item?: string;
	level?: number;
	zone?: string;
	killer?: string;
	skill?: string;
	value?: number;
	y?: number;
	x?: number;
	z?: number;
	classes?: string[];
	race?: string;
}

export interface LogEvent {
	id: number;
	at: string;
	kind: LogEventKind | string;
	payload: LogEventPayload;
}

export interface FightView {
	id: number;
	started_at: string;
	start_wall: number;
	fight: unknown;
}

export type HarvestKind = 'atlas' | 'quest' | 'alltime';

export interface HarvestDoc {
	character: string;
	server: string;
	kind: HarvestKind | string;
	captured_at: string;
	doc: unknown;
}

export type Rect = [number, number, number, number];

export interface LayoutSummary {
	name: string;
	screen_w: number;
	screen_h: number;
	windows: number;
	updated_at: string;
}

export interface LayoutView {
	name: string;
	screen_w: number;
	screen_h: number;
	layout: Record<string, Rect>;
	problems: string[];
	updated_at: string;
}

export interface LayoutBody {
	screen_w: number;
	screen_h: number;
	layout: Record<string, Rect>;
}

export class ApiError extends Error {
	status: number;
	constructor(status: number, message: string) {
		super(message);
		this.status = status;
		this.name = 'ApiError';
	}
}

function envelopeError(body: string): string | null {
	try {
		return (JSON.parse(body) as { error?: string }).error ?? null;
	} catch {
		return null;
	}
}

async function get<T>(path: string): Promise<T> {
	const response = await fetch(path);
	if (!response.ok) {
		const body = await response.text();
		throw new ApiError(response.status, envelopeError(body) || body || response.statusText);
	}
	return (await response.json()) as T;
}

async function authed<T>(
	method: string,
	path: string,
	token: string,
	body?: unknown
): Promise<T> {
	const response = await fetch(path, {
		method,
		headers: {
			authorization: `Bearer ${token}`,
			...(body === undefined ? {} : { 'content-type': 'application/json' })
		},
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	if (!response.ok) {
		const text = await response.text();
		throw new ApiError(response.status, envelopeError(text) || text || response.statusText);
	}
	return response.status === 204 ? (null as T) : ((await response.json()) as T);
}

export const endpoints = {
	characters: () => get<CharacterSummary[]>('/api/v1/characters'),
	character: (server: string, name: string) =>
		get<CharacterView>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}`
		),
	inventory: (server: string, name: string) =>
		get<InventoryView>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/inventory`
		),
	stats: (server: string, name: string) =>
		get<StatsView>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/stats`
		),
	events: (server: string, name: string, limit: number, before?: string) => {
		const query = new URLSearchParams({ limit: String(limit) });
		if (before) query.set('before', before);
		return get<LogEvent[]>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/events?${query}`
		);
	},
	fights: (server: string, name: string, limit: number, before?: string) => {
		const query = new URLSearchParams({ limit: String(limit) });
		if (before) query.set('before', before);
		return get<FightView[]>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/fights?${query}`
		);
	},
	harvest: (server: string, name: string, kind: HarvestKind) =>
		get<HarvestDoc>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/harvest/${kind}`
		),
	itemSearch: (query: string) =>
		get<ItemRecord[]>(`/api/v1/items?q=${encodeURIComponent(query)}`),
	item: (key: string) => get<ItemRecord>(`/api/v1/items/${encodeURIComponent(key)}`),
	layouts: () => get<LayoutSummary[]>('/api/v1/layouts'),
	layout: (name: string) => get<LayoutView>(`/api/v1/layouts/${encodeURIComponent(name)}`),
	layoutWindows: () => get<string[]>('/api/v1/layout-windows'),
	saveLayout: (token: string, name: string, body: LayoutBody) =>
		authed<LayoutView>('PUT', `/api/v1/layouts/${encodeURIComponent(name)}`, token, body),
	cloneDefault: (token: string, name: string) =>
		authed<LayoutView>(
			'POST',
			`/api/v1/layouts/${encodeURIComponent(name)}/clone-default`,
			token
		),
	deleteLayout: (token: string, name: string) =>
		authed<null>('DELETE', `/api/v1/layouts/${encodeURIComponent(name)}`, token)
};

export const bundleUrl = (name: string, skin: string) => {
	const query = skin ? `?skin=${encodeURIComponent(skin)}` : '';
	return `/api/v1/layouts/${encodeURIComponent(name)}/bundle${query}`;
};
