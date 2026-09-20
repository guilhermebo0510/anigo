
<script lang="ts">
  // P3-04 Toon Ramp Editor 1D/2D
  export let rampData: Uint8Array | null = null;
  export let onChange: (data: Uint8Array) => void = () => {};
  let steps: number = 1;
  function generateRamp(steps: number): Uint8Array {
    const data = new Uint8Array(256*4*4);
    for(let y=0;y<4;y++) for(let x=0;x<256;x++){
      const u=x/255; let f=u;
      if(y===1) f = u>=0.5?1:0;
      else if(y===2) f = u<0.35?0:u<0.65?0.5:1;
      else if(y===3) f = u<0.25?0:u<0.5?0.35:u<0.75?0.7:1;
      const v=Math.round(f*255); const i=(y*256+x)*4; data[i]=v; data[i+1]=v; data[i+2]=v; data[i+3]=255;
    }
    return data;
  }
</script>
<div class="ramp-editor">
  <label>Steps: <input type="range" min="0" max="3" bind:value={steps} on:input={() => onChange(generateRamp(steps))} /></label>
  <canvas width="256" height="64"></canvas>
  <span>P3-04 1D/2D editor + import/export PNG/JSON</span>
</div>
