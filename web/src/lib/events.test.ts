import { describe, expect, it } from 'vitest';
import type { LogEvent, LogEventPayload } from './api';
import { collapseLocations, describeEvent, eventTone } from './events';

const event = (kind: string, payload: LogEventPayload = {}): LogEvent => ({
	id: 1,
	at: '2026-08-05T19:29:24Z',
	kind,
	payload
});

describe('describeEvent', () => {
	it('reads a /who row as level, classes and race', () => {
		expect(
			describeEvent(
				event('who', { level: 15, classes: ['WAR', 'DRU', 'NEC'], race: 'Dark Elf' })
			)
		).toBe('Level 15 WAR/DRU/NEC Dark Elf');
	});

	it('leaves out a race the row did not carry', () => {
		expect(describeEvent(event('who', { level: 16, classes: ['WAR'] }))).toBe('Level 16 WAR');
		expect(eventTone(event('who'))).toBe('info');
	});

	it('still describes the kinds that came before it', () => {
		expect(describeEvent(event('level', { level: 5 }))).toBe('Reached level 5');
		expect(describeEvent(event('death'))).toBe('Died');
		expect(describeEvent(event('nonsense'))).toBe('nonsense');
	});
});

describe('collapseLocations', () => {
	const row = (id: number, kind: string): LogEvent => ({ ...event(kind), id });

	it('keeps the newest row of each run of location rows', () => {
		const rows = [
			row(9, 'location'),
			row(8, 'location'),
			row(7, 'loot'),
			row(6, 'location'),
			row(5, 'zone'),
			row(4, 'location'),
			row(3, 'location'),
			row(2, 'location')
		];
		expect(collapseLocations(rows).map((r) => r.id)).toEqual([9, 7, 6, 5, 4]);
	});

	it('leaves rows without locations alone', () => {
		const rows = [row(2, 'loot'), row(1, 'level')];
		expect(collapseLocations(rows)).toEqual(rows);
		expect(collapseLocations([])).toEqual([]);
	});
});
