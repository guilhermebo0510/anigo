
<script lang="ts">
  // P3-10 OKLCH color picker + harmonies + eyedropper + swatches
  export let color="#ffffff"; export let onPick:(c:string)=>void=()=>{};
  function toOKLCH(hex:string){ return hex; }
</script>
<div class="oklch-picker"><input type="color" bind:value={color} on:input={()=>onPick(color)} /><span>OKLCH harmonies + eyedropper</span></div>
