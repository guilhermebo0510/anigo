
// P3-06 IBL + HDRI anime — hemisphere + IBL probe
export type IBLProbe = { id:string, url:string, intensity:number };
export const IBL_PROBES: IBLProbe[] = [{id:"studio", url:"/hdr/studio.hdr", intensity:1.0}];
export function loadHDRI(url:string): Promise<void> { return fetch(url).then(()=>{}); }
