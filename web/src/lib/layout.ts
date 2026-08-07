import type { Rect } from './api';

export const GRID = 10;

/** Mirrors `eql_core::layout::Rect::overlaps` — strict, touching edges are fine. */
export function overlaps(a: Rect, b: Rect): boolean {
	return a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
}

/** Mirrors `eql_core::layout::Rect::within`. */
export function within(rect: Rect, width: number, height: number): boolean {
	return rect[0] >= 0 && rect[1] >= 0 && rect[0] + rect[2] <= width && rect[1] + rect[3] <= height;
}

export function validate(
	layout: Record<string, Rect>,
	width: number,
	height: number,
	hidden: string[] = []
): string[] {
	const entries = Object.entries(layout)
		.filter(([name]) => !hidden.includes(name))
		.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
	const problems: string[] = [];
	for (const [name, rect] of entries) {
		if (!within(rect, width, height)) problems.push(`${name} offscreen`);
	}
	for (let i = 0; i < entries.length; i++) {
		for (let j = i + 1; j < entries.length; j++) {
			if (overlaps(entries[i][1], entries[j][1])) {
				problems.push(`${entries[i][0]} overlaps ${entries[j][0]}`);
			}
		}
	}
	return problems;
}

export const snap = (value: number) => Math.round(value / GRID) * GRID;

export function clamp(rect: Rect, width: number, height: number): Rect {
	const w = Math.max(GRID, Math.min(rect[2], width));
	const h = Math.max(GRID, Math.min(rect[3], height));
	return [
		Math.max(0, Math.min(rect[0], width - w)),
		Math.max(0, Math.min(rect[1], height - h)),
		w,
		h
	];
}

const TOKEN_KEY = 'eql.machine-token';

/** Stopgap until eqls grows real user auth: the machine token is pasted once
 *  and kept in localStorage so writes from the editor can be authorised. */
export const tokenStore = {
	load: () => (typeof localStorage === 'undefined' ? '' : (localStorage.getItem(TOKEN_KEY) ?? '')),
	save: (token: string) => {
		if (typeof localStorage === 'undefined') return;
		if (token) localStorage.setItem(TOKEN_KEY, token);
		else localStorage.removeItem(TOKEN_KEY);
	}
};
