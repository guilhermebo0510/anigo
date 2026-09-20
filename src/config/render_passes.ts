
// P3-11 Isolated passes + EXR/16-bit + queue
export type RenderPassId = "beauty"|"line"|"shadow"|"depth"|"normal"|"mask";
export const PASSES: RenderPassId[] = ["beauty","line","shadow","depth","normal","mask"];
export type RenderJob = { id:string, pass:RenderPassId, resolution:[number,number], format:"png8"|"png16"|"exr" };
