import { createQuery } from '@tanstack/svelte-query';
import { endpoints } from './api';

const REFETCH_MS = 30_000;

export const qk = {
	characters: ['characters'] as const,
	inventory: (server: string, name: string) => ['inventory', server, name] as const,
	item: (key: string) => ['item', key] as const
};

export const useCharacters = () =>
	createQuery(() => ({
		queryKey: qk.characters,
		queryFn: endpoints.characters,
		refetchInterval: REFETCH_MS
	}));

export const useInventory = (server: () => string, name: () => string) =>
	createQuery(() => ({
		queryKey: qk.inventory(server(), name()),
		queryFn: () => endpoints.inventory(server(), name()),
		refetchInterval: REFETCH_MS
	}));

export const useItem = (key: () => string) =>
	createQuery(() => ({
		queryKey: qk.item(key()),
		queryFn: () => endpoints.item(key()),
		retry: false
	}));
