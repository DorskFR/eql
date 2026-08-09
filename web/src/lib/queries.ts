import { createInfiniteQuery, createQuery } from '@tanstack/svelte-query';
import { endpoints, type FightView, type HarvestKind, type LogEvent } from './api';

const REFETCH_MS = 30_000;
const EVENT_PAGE = 100;
const FIGHT_PAGE = 50;

export const qk = {
	characters: ['characters'] as const,
	character: (server: string, name: string) => ['character', server, name] as const,
	inventory: (server: string, name: string, loadout: string) =>
		['inventory', server, name, loadout] as const,
	stats: (server: string, name: string, loadout: string) =>
		['stats', server, name, loadout] as const,
	events: (server: string, name: string) => ['events', server, name] as const,
	fights: (server: string, name: string) => ['fights', server, name] as const,
	harvest: (server: string, name: string, kind: HarvestKind) =>
		['harvest', server, name, kind] as const,
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

export const useCharacter = (server: () => string, name: () => string) =>
	createQuery(() => ({
		queryKey: qk.character(server(), name()),
		queryFn: () => endpoints.character(server(), name()),
		refetchInterval: REFETCH_MS,
		retry: false
	}));

export const useInventory = (
	server: () => string,
	name: () => string,
	loadout: () => string
) =>
	createQuery(() => ({
		queryKey: qk.inventory(server(), name(), loadout()),
		queryFn: () => endpoints.inventory(server(), name(), loadout()),
		refetchInterval: REFETCH_MS
	}));

export const useStats = (server: () => string, name: () => string, loadout: () => string) =>
	createQuery(() => ({
		queryKey: qk.stats(server(), name(), loadout()),
		queryFn: () => endpoints.stats(server(), name(), loadout()),
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

export const useFights = (server: () => string, name: () => string) =>
	createInfiniteQuery(() => ({
		queryKey: qk.fights(server(), name()),
		queryFn: ({ pageParam }: { pageParam: string | undefined }) =>
			endpoints.fights(server(), name(), FIGHT_PAGE, pageParam),
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (last: FightView[]) =>
			last.length < FIGHT_PAGE ? undefined : last[last.length - 1].started_at,
		refetchInterval: REFETCH_MS
	}));

export const useHarvest = (server: () => string, name: () => string, kind: HarvestKind) =>
	createQuery(() => ({
		queryKey: qk.harvest(server(), name(), kind),
		queryFn: () => endpoints.harvest(server(), name(), kind),
		refetchInterval: REFETCH_MS,
		retry: false
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
