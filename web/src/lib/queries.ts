import { createInfiniteQuery, createQuery } from '@tanstack/svelte-query';
import { endpoints, type LogEvent } from './api';

const REFETCH_MS = 30_000;
const EVENT_PAGE = 100;

export const qk = {
	characters: ['characters'] as const,
	inventory: (server: string, name: string) => ['inventory', server, name] as const,
	stats: (server: string, name: string) => ['stats', server, name] as const,
	events: (server: string, name: string) => ['events', server, name] as const,
	item: (key: string) => ['item', key] as const,
	layouts: ['layouts'] as const,
	layout: (name: string) => ['layout', name] as const
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

export const useStats = (server: () => string, name: () => string) =>
	createQuery(() => ({
		queryKey: qk.stats(server(), name()),
		queryFn: () => endpoints.stats(server(), name()),
		refetchInterval: REFETCH_MS
	}));

export const useEvents = (server: () => string, name: () => string) =>
	createInfiniteQuery(() => ({
		queryKey: qk.events(server(), name()),
		queryFn: ({ pageParam }: { pageParam: string | undefined }) =>
			endpoints.events(server(), name(), EVENT_PAGE, pageParam),
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (last: LogEvent[]) =>
			last.length < EVENT_PAGE ? undefined : last[last.length - 1].at,
		refetchInterval: REFETCH_MS
	}));

export const useItem = (key: () => string) =>
	createQuery(() => ({
		queryKey: qk.item(key()),
		queryFn: () => endpoints.item(key()),
		retry: false
	}));

export const useLayouts = () =>
	createQuery(() => ({
		queryKey: qk.layouts,
		queryFn: endpoints.layouts
	}));

export const useLayout = (name: () => string) =>
	createQuery(() => ({
		queryKey: qk.layout(name()),
		queryFn: () => endpoints.layout(name()),
		retry: false
	}));
