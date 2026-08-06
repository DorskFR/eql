import type { LogEvent } from './api';

type Tone = 'neutral' | 'info' | 'ok' | 'warn' | 'danger';

const TONES: Record<string, Tone> = {
	loot: 'ok',
	level: 'info',
	zone: 'neutral',
	death: 'danger',
	location: 'neutral',
	skill: 'info',
	who: 'info'
};

export function eventTone(event: LogEvent): Tone {
	return TONES[event.kind] ?? 'neutral';
}

const coord = (value: number | undefined) => (value === undefined ? '?' : value.toFixed(2));

export function describeEvent(event: LogEvent): string {
	const { payload } = event;
	switch (event.kind) {
		case 'loot':
			return `Looted ${payload.item ?? 'something'}`;
		case 'level':
			return `Reached level ${payload.level ?? '?'}`;
		case 'zone':
			return `Entered ${payload.zone ?? 'a new zone'}`;
		case 'death':
			return payload.killer ? `Slain by ${payload.killer}` : 'Died';
		case 'location':
			return `At ${coord(payload.y)}, ${coord(payload.x)}, ${coord(payload.z)}`;
		case 'skill':
			return `${payload.skill ?? 'A skill'} improved to ${payload.value ?? '?'}`;
		case 'who':
			return `Level ${payload.level ?? '?'} ${(payload.classes ?? []).join('/')}${
				payload.race ? ` ${payload.race}` : ''
			}`.trim();
		default:
			return event.kind;
	}
}
