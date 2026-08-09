import { describe, expect, it } from 'vitest';
import { validate } from './layout';
import type { Rect } from './api';

const layout: Record<string, Rect> = {
	PlayerWindow: [0, 0, 100, 100],
	TargetWindow: [50, 50, 100, 100]
};

describe('validate', () => {
	it('reports an overlap between two visible windows', () => {
		expect(validate(layout, 1600, 900)).toEqual(['PlayerWindow overlaps TargetWindow']);
	});

	it('ignores windows the skin hides', () => {
		expect(validate(layout, 1600, 900, ['TargetWindow'])).toEqual([]);
	});

	it('still reports a visible window pushed offscreen', () => {
		expect(validate({ MainChat: [1500, 800, 400, 300] }, 1600, 900)).toEqual([
			'MainChat offscreen'
		]);
	});
});
