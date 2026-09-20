
// P3-08 Post: bloom, DoF, chromatic aberration, grain, LUT
export type PostSettings = { bloom:number, dof:number, aberration:number, grain:number, lut:string };
export const DEFAULT_POST: PostSettings = { bloom:0.2, dof:0, aberration:0.01, grain:0.02, lut:"neutral" };
