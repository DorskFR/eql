<script lang="ts">
	import { page } from '$app/state';
	import ItemDetail from '$lib/ItemDetail.svelte';

	const key = $derived(page.params.name ?? '');
</script>

<div class="eq-window eq-inspect">
	<a class="eq-back" href="/">« Back</a>
	<ItemDetail {key} />
</div>

<style>
	.eq-window {
		--eq-gold-bright: #c9b37a;
		font-family: Georgia, 'Times New Roman', serif;
		background:
			radial-gradient(ellipse 80% 50% at 20% 10%, rgba(90, 90, 100, 0.25), transparent 60%),
			radial-gradient(ellipse 60% 40% at 80% 80%, rgba(70, 70, 85, 0.2), transparent 60%),
			repeating-linear-gradient(
				115deg,
				#232326 0px,
				#1c1c1f 3px,
				#232327 7px,
				#18181b 11px,
				#212124 16px
			);
		border: 2px solid #3a3a3e;
		border-top-color: #55555a;
		border-left-color: #4a4a4f;
		outline: 1px solid #0c0c0e;
		box-shadow:
			inset 0 0 0 1px #0c0c0e,
			0 4px 18px rgba(0, 0, 0, 0.6);
		border-radius: 4px;
		padding: var(--sp-3, 0.75rem);
		display: flex;
		flex-direction: column;
		gap: var(--sp-3, 0.75rem);
	}

	.eq-inspect {
		max-width: 34rem;
		margin: 0 auto;
	}

	.eq-back {
		color: var(--eq-gold-bright);
		font-size: 0.8rem;
		text-decoration: none;
		align-self: flex-start;
	}

	.eq-back:hover {
		text-decoration: underline;
	}
</style>
