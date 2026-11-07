#!/usr/bin/env node
/**
 * Captures the README screenshots from a local eqls seeded with fake data, in a light and a dark theme.
 *
 * Usage: scripts/readme-demo.sh up, then
 *   node scripts/readme-screenshots.mjs [--url http://127.0.0.1:5411]
 * Needs the `playwright` package (~1.63) with Chromium; point NODE_PATH at any install of it.
 */
import { mkdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { parseArgs } from 'node:util';

const { chromium } = createRequire(import.meta.url)('playwright');

const { values } = parseArgs({
	options: {
		url: { type: 'string', default: 'http://127.0.0.1:5411' },
		out: { type: 'string', default: 'docs/assets' },
	},
});

const VIEWPORT = { width: 1280, height: 1100 };
const SHOTS = [
	{ path: '/c/erudin/Ashvale', theme: 'light', file: 'profile-light.png', below: '.eq-window' },
	{ path: '/c/erudin/Ashvale', theme: 'dark', file: 'profile-dark.png', below: '.eq-window' },
	{ path: '/layouts/demo', theme: 'light', file: 'designer-light.png', height: 808 },
	{ path: '/layouts/demo', theme: 'dark', file: 'designer-dark.png', height: 808 },
];

await mkdir(values.out, { recursive: true });
const browser = await chromium.launch();
try {
	for (const shot of SHOTS) {
		const page = await browser.newPage({
			viewport: VIEWPORT,
			deviceScaleFactor: 1,
			colorScheme: shot.theme,
			reducedMotion: 'reduce',
		});
		await page.addInitScript((theme) => localStorage.setItem('tsumikit-theme', theme), shot.theme);
		await page.goto(`${values.url}${shot.path}`, { waitUntil: 'networkidle' });
		await page.evaluate(() => document.fonts.ready);
		await page.mouse.move(0, 0);
		const bottom =
			shot.height ??
			(await page.evaluate((selector) => {
				const box = document.querySelector(selector)?.getBoundingClientRect();
				return box ? Math.ceil(box.bottom) + 24 : 1100;
			}, shot.below));
		await page.screenshot({
			path: `${values.out}/${shot.file}`,
			clip: { x: 0, y: 0, width: VIEWPORT.width, height: Math.min(bottom, VIEWPORT.height) },
		});
		await page.close();
		console.log(`${values.out}/${shot.file}`);
	}
} finally {
	await browser.close();
}
