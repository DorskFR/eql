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
}

export interface CharacterSummary {
	name: string;
	server: string;
	last_snapshot_at: string | null;
	snapshot_count: number;
}

export interface InventoryView {
	character: string;
	server: string;
	captured_at: string;
	entries: InventoryEntry[];
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

export const endpoints = {
	characters: () => get<CharacterSummary[]>('/api/v1/characters'),
	inventory: (server: string, name: string) =>
		get<InventoryView>(
			`/api/v1/characters/${encodeURIComponent(server)}/${encodeURIComponent(name)}/inventory`
		),
	itemSearch: (query: string) =>
		get<ItemRecord[]>(`/api/v1/items?q=${encodeURIComponent(query)}`),
	item: (key: string) => get<ItemRecord>(`/api/v1/items/${encodeURIComponent(key)}`)
};
