export interface InventoryEntry {
	location: string;
	name: string;
	id: number;
	count: number;
	slots: number;
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
		)
};
