
// P3-07 per-object materials + library (was 1 global material)
export type MaterialEntry = { id:string, name:string, material:any, preview?:string };
export const MATERIAL_LIBRARY: MaterialEntry[] = [
  { id:"skin", name:"Pele", material:{ base_color:"#faeae0", shade:"#e6c8c8" } },
  { id:"hair", name:"Cabelo", material:{ base_color:"#2a1a0f", shade:"#1a0f0a" } },
  { id:"eye", name:"Olho", material:{ base_color:"#6b8cff", shade:"#3b5bff" } },
];
