import { describe, expect, it } from 'vitest';
import type { ItemStats } from './api';
import { iconUrl } from './items';

const stats = (icon: number | null) => ({ icon }) as ItemStats;

describe('iconUrl', () => {
	it('points at the icon endpoint', () => {
		expect(iconUrl(stats(550))).toBe('/api/v1/icons/550.png');
	});

	it('has no url without an item', () => {
		expect(iconUrl(undefined)).toBeNull();
	});

	it('has no url for an item the wiki gave no icon', () => {
		expect(iconUrl(stats(null))).toBeNull();
		expect(iconUrl(stats(0))).toBeNull();
	});
});
