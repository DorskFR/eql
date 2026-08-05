import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

declare const process: { env: Record<string, string | undefined> };

export default defineConfig({
	plugins: [sveltekit()],
	server: {
		host: true,
		port: 5273,
		proxy: process.env.EQL_PROXY
			? { '/api': { target: process.env.EQL_PROXY, changeOrigin: true, secure: true } }
			: undefined
	}
});
