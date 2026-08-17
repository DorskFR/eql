import type { InventoryEntry } from './api';

const CONTAINER_PREFIXES = ['General', 'Bank', 'SharedBank'];

export function isContainerLocation(location: string): boolean {
	return CONTAINER_PREFIXES.some((prefix) => location.startsWith(prefix));
}

export function isEquipped(entry: InventoryEntry): boolean {
	return !entry.location.includes('-Slot') && !isContainerLocation(entry.location);
}

export function isGeneral(entry: InventoryEntry): boolean {
	return entry.location.startsWith('General');
}

export function isBank(entry: InventoryEntry): boolean {
	return entry.location.startsWith('Bank') || entry.location.startsWith('SharedBank');
}

export interface InventoryGroups<T extends InventoryEntry = InventoryEntry> {
	equipped: T[];
	general: T[];
	bank: T[];
}

export function groupEntries<T extends InventoryEntry>(entries: T[]): InventoryGroups<T> {
	return {
		equipped: entries.filter(isEquipped),
		general: entries.filter(isGeneral),
		bank: entries.filter(isBank)
	};
}

export function filled<T extends InventoryEntry>(entries: T[]): T[] {
	return entries.filter((entry) => entry.name !== 'Empty');
}

export const SLOT_ROWS: string[][] = [
	['Focus', 'Ear', 'Head', 'Face', 'Ear'],
	['Neck', 'Shoulders', 'Arms', 'Back', 'Wrist', 'Wrist'],
	['Range', 'Hands', 'Primary', 'Secondary', 'Fingers', 'Fingers'],
	['Chest', 'Legs', 'Feet', 'Waist', 'Extra', 'Ammo']
];

/** EQL's two custom sockets arrive as two `Any Slot` rows; the dump lists the
 *  first (Focus) at the top and the second (Extra) just before Ammo. */
const ANY_SLOT_LABELS = ['Focus', 'Extra'];

export interface PaperdollSlot<T extends InventoryEntry> {
	key: string;
	label: string;
	entry: T | null;
}

const baseLocation = (location: string) => location.replace(/ ?\d+$/, '');

export function paperdoll<T extends InventoryEntry>(equipped: T[]): PaperdollSlot<T>[][] {
	const pools = new Map<string, T[]>();
	const anySlots: T[] = [];
	for (const entry of equipped) {
		if (entry.location === 'Any Slot') {
			anySlots.push(entry);
			continue;
		}
		const base = baseLocation(entry.location);
		const pool = pools.get(base) ?? [];
		pool.push(entry);
		pools.set(base, pool);
	}
	return SLOT_ROWS.map((row, rowIndex) =>
		row.map((label, colIndex) => {
			const anyIndex = ANY_SLOT_LABELS.indexOf(label);
			const entry = (anyIndex >= 0 ? anySlots[anyIndex] : pools.get(label)?.shift()) ?? null;
			return {
				key: `${rowIndex}-${colIndex}`,
				label,
				entry: entry && entry.name !== 'Empty' ? entry : null
			};
		})
	);
}

export interface Bag<T extends InventoryEntry> {
	key: string;
	label: string;
	container: T | null;
	contents: T[];
}

const BAG_PATTERN = /^(General|SharedBank|Bank) ?(\d+)(-Slot\d+)?$/;

export function bags<T extends InventoryEntry>(entries: T[]): Bag<T>[] {
	const byNumber = new Map<string, Bag<T>>();
	for (const entry of entries) {
		const match = BAG_PATTERN.exec(entry.location);
		if (!match) continue;
		const id = `${match[1]}${match[2]}`;
		let bag = byNumber.get(id);
		if (!bag) {
			bag = { key: id, label: `${match[1] === 'General' ? 'Slot' : match[1]} ${match[2]}`, container: null, contents: [] };
			byNumber.set(id, bag);
		}
		if (match[3]) {
			if (entry.name !== 'Empty') bag.contents.push(entry);
		} else if (entry.name !== 'Empty') {
			bag.container = entry;
		}
	}
	return [...byNumber.values()].filter((bag) => bag.container || bag.contents.length);
}
