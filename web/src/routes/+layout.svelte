<script lang="ts">
	import '@dorsk/tsumikit/styles/app.css';
	import { QueryClient, QueryClientProvider } from '@tanstack/svelte-query';
	import { Cluster, Container, Heading, Link, Stack, ThemePicker } from '@dorsk/tsumikit';

	let { children } = $props();

	const queryClient = new QueryClient({
		defaultOptions: {
			queries: { retry: 1, staleTime: 5_000, refetchOnWindowFocus: false }
		}
	});
</script>

<QueryClientProvider client={queryClient}>
	<Container pad>
		<Stack gap="var(--sp-5)">
			<Cluster justify="space-between">
				<Cluster gap="var(--sp-4)">
					<Link href="/">
						<Heading level={1} size="lg">eql</Heading>
					</Link>
					<Link href="/layouts">Layouts</Link>
					<Link href="/devices">Devices</Link>
				</Cluster>
				<ThemePicker />
			</Cluster>
			{@render children()}
		</Stack>
	</Container>
</QueryClientProvider>
