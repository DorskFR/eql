import { describe, expect, it } from 'vitest';
import type { InventoryEntry } from './api';
import { paperdoll } from './inventory';

const entry = (location: string, name: string): InventoryEntry => ({
	location,
	name,
	id: 0,
	count: 1,
	slots: 0
});

const flat = (equipped: InventoryEntry[]) =>
	paperdoll(equipped)
		.flat()
		.filter((slot) => slot.entry)
		.map((slot) => [slot.label, slot.entry?.name]);

describe('paperdoll', () => {
	it('maps the two Any Slot rows to Focus then Extra in dump order', () => {
		const rows = flat([
			entry('Any Slot', 'Shield of Midnight'),
			entry('Head', 'Bronze Helm'),
			entry('Any Slot', 'Shield of the Dawn'),
			entry('Ammo', 'Throwing Knife')
		]);
		expect(rows).toEqual([
			['Focus', 'Shield of Midnight'],
			['Head', 'Bronze Helm'],
			['Extra', 'Shield of the Dawn'],
			['Ammo', 'Throwing Knife']
		]);
	});

	it('leaves empty Any Slot rows as empty cells', () => {
		const rows = paperdoll([entry('Any Slot', 'Empty'), entry('Any Slot', 'Empty')]).flat();
		expect(rows.every((slot) => slot.entry === null)).toBe(true);
		expect(rows.map((slot) => slot.label)).toContain('Focus');
		expect(rows.map((slot) => slot.label)).toContain('Extra');
	});

	it('splits paired slots left to right in dump order', () => {
		const rows = flat([entry('Ear', 'Left Hoop'), entry('Ear', 'Right Hoop')]);
		expect(rows).toEqual([
			['Ear', 'Left Hoop'],
			['Ear', 'Right Hoop']
		]);
	});
});
