/// <reference types="svelte" />
/// <reference types="vite/client" />

declare module "*.wgsl?raw" {
  const content: string;
  export default content;
}
declare module "*.wgsl" {
  const content: string;
  export default content;
}
declare module "*.glsl?raw" {
  const content: string;
  export default content;
}
declare module "*.glsl" {
  const content: string;
  export default content;
}

// O `lib.dom` do TypeScript do projeto (5.7) declara as *interfaces* WebGPU
// (GPUTextureView, GPUCanvasContext...) mas não os enums de *valores* nem a
// `depthResolveAttachment` (revisão antiga da spec). O runtime (browser)
// fornece tudo; esta declaração ambiente mínima cobre os nomes que o renderer
// usa, evitando "Cannot find name" sem trocar a tipagem do projeto.
// (`@webgpu/types` completo entra quando o projeto quiser tipagem estrita.)
declare const GPUBufferUsage: { [key: string]: number };
declare const GPUTextureUsage: { [key: string]: number };
declare const GPUShaderStage: { [key: string]: number };
