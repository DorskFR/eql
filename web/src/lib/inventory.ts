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
