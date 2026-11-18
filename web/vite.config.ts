import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

declare const process: { env: Record<string, string | undefined> };

export default defineConfig({
	plugins: [
		sveltekit({
			preprocess: vitePreprocess(),
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	server: {
		host: true,
		port: 5273,
		proxy: process.env.EQL_PROXY
			? { '/api': { target: process.env.EQL_PROXY, changeOrigin: true, secure: true } }
			: undefined
	}
});
