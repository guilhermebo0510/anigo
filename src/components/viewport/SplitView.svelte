
<script lang="ts">
  // P3-09 Split A/B + reference overlay + safe areas
  export let leftLabel="Viewport"; export let rightLabel="Reference";
</script>
<div class="split-view" aria-label={`${leftLabel} / ${rightLabel}`}><slot name="left"/><slot name="right"/><slot name="overlay"/></div>
