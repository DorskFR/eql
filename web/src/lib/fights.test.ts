import { describe, expect, it } from 'vitest';
import fixture from '../../../fixtures/fights/eql_fights_Dorsk_erudin.json';
import type { FightView } from './api';
import { clock, enemyList, projectFight, projectFights } from './fights';

const views: FightView[] = [...fixture]
	.sort((a, b) => b.start_wall - a.start_wall)
	.map((fight, index) => ({
		id: fixture.length - index,
		started_at: new Date(fight.start_wall * 1000).toISOString(),
		start_wall: fight.start_wall,
		fight
	}));

const viewAt = (start_wall: number): FightView => {
	const found = views.find((view) => view.start_wall === start_wall);
	if (!found) throw new Error(`no fixture fight at ${start_wall}`);
	return found;
};

describe('projectFight', () => {
	it('reads the whole fight the tracker emitted', () => {
		const najena = viewAt(1785931338);
		const row = projectFight(najena);

		expect(row.zone).toBe('Najena 4 (Refined)');
		expect(row.span).toBe(158);
		expect(row.active_secs).toBe(158);
		expect(row.dmg_out).toBe(7654);
		expect(row.dmg_in).toBe(3142);
		expect(row.heal_out).toBe(2166);
		expect(row.kills).toBe(5);
		expect(row.deaths).toBe(1);
		expect(row.dps).toBeCloseTo(48.4, 1);
		expect(row.taken_per_sec).toBeCloseTo(19.9, 1);
		expect(row.stance).toBe('Mage Hunter Stance');
		expect(row.invocation).toBe('Spellblade');
		expect(row.enemies).toContain('a greater skeleton');
		expect(row.allies).toEqual([]);
	});

	it('ranks abilities by damage and shares them out', () => {
		const row = projectFight(viewAt(1785931338));
		expect(row.abilities.map((ability) => ability.name)).toEqual([
			'Melee',
			'Lifedraw',
			'Kick',
			'Reaving Strike',
			'Poison Storm'
		]);

		const melee = row.abilities[0];
		expect(melee.total).toBe(4322);
		expect(melee.hits).toBe(116);
		expect(melee.crits).toBe(10);
		expect(melee.biggest).toBe(185);
		expect(melee.category).toBe('melee');
		expect(melee.proc).toBe(false);
		expect(melee.average).toBeCloseTo(37.3, 1);
		expect(melee.share).toBeCloseTo(0.58, 2);
		expect(row.abilities.reduce((total, a) => total + a.share, 0)).toBeCloseTo(1, 6);
	});

	it('pairs casts with the resists of the same spell', () => {
		const row = projectFight(viewAt(1785960884));
		const frost = row.casts.find((cast) => cast.name === 'Column of Frost');
		expect(frost).toEqual({
			key: `${row.key}:cast:Column of Frost`,
			name: 'Column of Frost',
			casts: 7,
			resists: 3
		});
	});

	it('survives a fight with no abilities, no zone and no rates', () => {
		const row = projectFight({
			id: 7,
			started_at: '2026-08-05T12:18:50Z',
			start_wall: 1785932330,
			fight: { start_wall: 1785932330, span: 1, active_secs: 0, dmg_in_you: 5 }
		});
		expect(row.zone).toBeNull();
		expect(row.stance).toBeNull();
		expect(row.dps).toBeNull();
		expect(row.taken_per_sec).toBeNull();
		expect(row.abilities).toEqual([]);
		expect(row.casts).toEqual([]);
		expect(row.enemies).toEqual([]);
		expect(row.dmg_out).toBe(0);
	});

	it('treats a fight that is not an object as an empty one', () => {
		for (const fight of [null, 42, 'nope', []]) {
			const row = projectFight({ id: 1, started_at: 'x', start_wall: 0, fight });
			expect(row.dmg_out).toBe(0);
			expect(row.abilities).toEqual([]);
		}
	});
});

describe('projectFights', () => {
	it('totals the whole page and keeps the order it was given', () => {
		const projection = projectFights(views);
		expect(projection.usable).toBe(true);
		expect(projection.fights).toHaveLength(13);
		expect(projection.fights[0].zone).toBe('The Greater Faydark');

		expect(projection.totals.fights).toBe(13);
		expect(projection.totals.kills).toBe(65);
		expect(projection.totals.deaths).toBe(2);
		expect(projection.totals.dmg_out).toBe(28_049);
		expect(projection.totals.dmg_in).toBe(10_010);
		expect(projection.totals.dps).toBeCloseTo(
			projection.totals.dmg_out / projection.totals.active_secs,
			6
		);
	});

	it('is unusable with nothing to show', () => {
		for (const empty of [undefined, []]) {
			const projection = projectFights(empty);
			expect(projection.usable).toBe(false);
			expect(projection.totals.dps).toBeNull();
			expect(projection.totals.kills).toBe(0);
		}
	});
});

describe('clock', () => {
	it('reads as minutes and seconds until it needs hours', () => {
		expect(clock(0)).toBe('0m 00s');
		expect(clock(-5)).toBe('0m 00s');
		expect(clock(58.6)).toBe('0m 59s');
		expect(clock(158)).toBe('2m 38s');
		expect(clock(3661)).toBe('1h 01m');
	});
});

describe('enemyList', () => {
	it('says so when a fight named nothing', () => {
		expect(enemyList([])).toBe('nothing named');
		expect(enemyList(['orc centurion', 'orc slaver'])).toBe('orc centurion, orc slaver');
	});
});
