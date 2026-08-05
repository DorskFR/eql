import { describe, expect, it } from 'vitest';
import fixture from '../../../fixtures/harvest/eql_alltime_Dorsk_erudin__WAR-CLR.json';
import { buildLabel, projectAlltime } from './harvest';

const live = {
	hits: 84,
	misses: 9,
	crits: 1,
	kills: 5,
	deaths: 0,
	biggest: 51,
	combat_secs: 332.821355342865,
	stance_secs: { Balanced: 332.821355342865 },
	invocation_secs: {
		'Arcane Mastery': 31.231687307357788,
		Inviolable: 105.81969451904297,
		Recover: 153.81518983840942,
		Divine: 41.95478367805481
	},
	source_dmg: { spell: 1594, melee: 80, ds: 384 }
};

describe('buildLabel', () => {
	it('restores the slashes the meter strips for the filename', () => {
		expect(buildLabel('WAR-CLR')).toBe('WAR / CLR');
		expect(buildLabel('SHD-SHM-MNK')).toBe('SHD / SHM / MNK');
	});
});

describe('projectAlltime', () => {
	it('rejects docs that are not alltime', () => {
		for (const doc of [undefined, null, 42, 'x', [], {}, { zones: {} }, { builds: {} }]) {
			expect(projectAlltime(doc).usable).toBe(false);
		}
	});

	it('reports zeroed totals when unusable', () => {
		const empty = projectAlltime(null);
		expect(empty.totals.damage).toBe(0);
		expect(empty.totals.accuracy).toBeNull();
		expect(empty.totals.dps).toBeNull();
		expect(empty.builds).toEqual([]);
	});

	it('projects a flat single-build doc', () => {
		const out = projectAlltime(live);
		expect(out.usable).toBe(true);
		expect(out.builds).toHaveLength(1);

		const [row] = out.builds;
		expect(row.key).toBe('current');
		expect(row.build).toBe('Current build');
		expect(row.damage).toBe(2058);
		expect(row.accuracy).toBeCloseTo(84 / 93);
		expect(row.crit_rate).toBeCloseTo(1 / 84);
		expect(row.dps).toBeCloseTo(2058 / 332.821355342865);
		expect(row.kill_death).toBeNull();

		expect(out.sources.map((s) => s.label)).toEqual(['spell', 'ds', 'melee']);
		expect(out.sources[0].share).toBeCloseTo(1594 / 2058);
		expect(out.sources.reduce((total, s) => total + s.share, 0)).toBeCloseTo(1);

		expect(out.stances.map((s) => s.label)).toEqual(['Balanced']);
		expect(out.stances[0].share).toBe(1);
		expect(out.invocations.map((s) => s.label)).toEqual([
			'Recover',
			'Inviolable',
			'Divine',
			'Arcane Mastery'
		]);
	});

	it('labels a flat doc that names its own build', () => {
		expect(projectAlltime({ ...live, build: 'WAR-CLR' }).builds[0].build).toBe('WAR / CLR');
	});

	it('projects the repo fixture', () => {
		const out = projectAlltime(fixture);
		expect(out.usable).toBe(true);
		expect(out.totals.damage).toBe(4120334 + 881204 + 210556 + 91002);
		expect(out.totals.kill_death).toBeCloseTo(1349 / 21);
		expect(out.stances.map((s) => s.label)).toEqual(['Precision', 'Ferocity']);
	});

	it('merges and ranks several builds', () => {
		const out = projectAlltime({
			builds: {
				'WAR-CLR': {
					hits: 10,
					misses: 10,
					crits: 2,
					kills: 4,
					deaths: 2,
					biggest: 90,
					combat_secs: 100,
					stance_secs: { Precision: 100 },
					source_dmg: { melee: 100 }
				},
				'SHD-SHM-MNK': {
					hits: 30,
					misses: 10,
					crits: 3,
					kills: 6,
					deaths: 2,
					biggest: 200,
					combat_secs: 100,
					stance_secs: { Precision: 40, Ferocity: 60 },
					invocation_secs: { Recover: 25 },
					source_dmg: { melee: 200, spell: 300 }
				}
			}
		});

		expect(out.builds.map((b) => b.build)).toEqual(['SHD / SHM / MNK', 'WAR / CLR']);
		expect(out.builds.map((b) => b.key)).toEqual(['SHD-SHM-MNK', 'WAR-CLR']);
		expect(out.builds[0].damage).toBe(500);
		expect(out.builds[0].dps).toBeCloseTo(5);
		expect(out.builds[1].dps).toBeCloseTo(1);

		expect(out.totals.damage).toBe(600);
		expect(out.totals.hits).toBe(40);
		expect(out.totals.biggest).toBe(200);
		expect(out.totals.combat_secs).toBe(200);
		expect(out.totals.accuracy).toBeCloseTo(40 / 60);
		expect(out.totals.dps).toBeCloseTo(3);
		expect(out.totals.kill_death).toBeCloseTo(10 / 4);

		expect(out.sources).toEqual([
			{ key: 'src:melee', label: 'melee', value: 300, share: 0.5 },
			{ key: 'src:spell', label: 'spell', value: 300, share: 0.5 }
		]);
		expect(out.stances.map((s) => [s.label, s.value])).toEqual([
			['Precision', 140],
			['Ferocity', 60]
		]);
		expect(out.invocations).toHaveLength(1);
	});

	it('keeps per-build breakdowns keyed apart', () => {
		const out = projectAlltime({
			builds: {
				A: { hits: 1, source_dmg: { melee: 10 } },
				B: { hits: 1, source_dmg: { melee: 10 } }
			}
		});
		const keys = out.builds.flatMap((b) => b.sources.map((s) => s.key));
		expect(new Set(keys).size).toBe(keys.length);
	});

	it('drops zero and non-numeric breakdown entries', () => {
		const out = projectAlltime({
			hits: 1,
			source_dmg: { melee: 10, spell: 0, bogus: 'x' },
			stance_secs: { Precision: 0 }
		});
		expect(out.sources.map((s) => s.label)).toEqual(['melee']);
		expect(out.stances).toEqual([]);
	});

	it('accepts a doc that has only counters and no damage yet', () => {
		const out = projectAlltime({ hits: 0, misses: 0, kills: 0 });
		expect(out.usable).toBe(true);
		expect(out.totals.dps).toBeNull();
		expect(out.totals.accuracy).toBeNull();
	});
});
