<script lang="ts">
  let { size = 18 }: { size?: number } = $props();
</script>

<svg xmlns="http://www.w3.org/2000/svg" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
  <circle cx="12" cy="12" r="10"/>
  <path d="M12 2a10 10 0 0 1 10 10 10 10 0 0 1-10 10V2z" fill="currentColor" fill-opacity="0.3"/>
</svg>
