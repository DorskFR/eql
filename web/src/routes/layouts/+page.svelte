<script lang="ts">
	import { goto } from '$app/navigation';
	import {
		Badge,
		Button,
		Card,
		Cluster,
		DataTable,
		EmptyState,
		Field,
		Heading,
		Input,
		Modal,
		Spinner,
		Stack,
		Text,
		Timestamp,
		type Column
	} from '@dorsk/tsumikit';
	import { endpoints, type LayoutSummary } from '$lib/api';
	import { tokenStore } from '$lib/layout';
	import { useLayouts } from '$lib/queries';

	const layouts = useLayouts();

	let creating = $state(false);
	let newName = $state('dorskui');
	let token = $state(tokenStore.load());
	let busy = $state(false);
	let error = $state('');

	const columns: Column<LayoutSummary>[] = [
		{ key: 'name', label: 'Layout', sortable: true },
		{ key: 'windows', label: 'Windows', align: 'right', sortable: true },
		{ key: 'screen_w', label: 'Screen', sortable: true },
		{ key: 'updated_at', label: 'Updated', sortable: true }
	];

	async function cloneDefault() {
		error = '';
		busy = true;
		try {
			tokenStore.save(token);
			await endpoints.cloneDefault(token, newName.trim());
			creating = false;
			await goto(`/layouts/${encodeURIComponent(newName.trim())}`);
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
		} finally {
			busy = false;
		}
	}
</script>

<Stack gap="var(--sp-4)">
	<Cluster justify="space-between">
		<Heading level={2}>Layouts</Heading>
		<Cluster gap="var(--sp-2)">
			{#if layouts.isFetching}<Spinner label="Refreshing" />{/if}
			<Button variant="primary" onclick={() => (creating = true)}>Clone default</Button>
		</Cluster>
	</Cluster>

	{#if layouts.isPending}
		<Card>
			<Cluster gap="var(--sp-2)">
				<Spinner />
				<Text tone="muted">Loading layouts…</Text>
			</Cluster>
		</Card>
	{:else if layouts.isError}
		<EmptyState
			title="Could not load layouts"
			description={layouts.error.message}
			icon="alert-circle"
			tone="danger"
			actionLabel="Retry"
			onAction={() => layouts.refetch()}
		/>
	{:else if layouts.data.length === 0}
		<EmptyState
			title="No layouts yet"
			description="Start from the dorskui template and drag the windows where you want them."
			icon="grid"
			actionLabel="Clone default"
			onAction={() => (creating = true)}
		/>
	{:else}
		<Card padding="none">
			<DataTable
				{columns}
				rows={layouts.data}
				rowKey={(row) => row.name}
				onrowclick={(row) => goto(`/layouts/${encodeURIComponent(row.name)}`)}
				cellSnippets={{ screen_w: screen, updated_at: updated }}
			/>
		</Card>
	{/if}
</Stack>

{#snippet screen(row: LayoutSummary)}
	<Badge tone="neutral" mono>{row.screen_w}×{row.screen_h}</Badge>
{/snippet}

{#snippet updated(row: LayoutSummary)}
	<Timestamp value={row.updated_at} mode="relative" />
{/snippet}

{#if creating}
	<Modal title="Clone the default layout" onclose={() => (creating = false)}>
		{#snippet body()}
			<Stack gap="var(--sp-3)">
				<Field label="Layout name" for="new-layout-name">
					<Input id="new-layout-name" bind:value={newName} mono />
				</Field>
				<Field
					label="Machine token"
					for="new-layout-token"
					hint="Stored in this browser only; a stopgap until eqls has real auth."
					error={error || undefined}
				>
					<Input id="new-layout-token" type="password" bind:value={token} mono />
				</Field>
			</Stack>
		{/snippet}
		{#snippet footer()}
			<Cluster justify="flex-end" gap="var(--sp-2)">
				<Button variant="ghost" onclick={() => (creating = false)}>Cancel</Button>
				<Button
					variant="primary"
					disabled={busy || !newName.trim() || !token}
					onclick={cloneDefault}
				>
					{busy ? 'Creating…' : 'Create'}
				</Button>
			</Cluster>
		{/snippet}
	</Modal>
{/if}
