# Fase 2 — Interoperabilidade e Shading Anime Supremo (plano de execução)

Milestone GitHub #2 — 8 issues. Este arquivo acompanha o progresso.

## Progresso

- [x] #25 — parser glTF 2.0 + VRM 1.0 (TS, 16 testes) + espelho Rust anigo-vrm + comandos Tauri (commits 286a679, 99f0eeb)
- [x] #18 — material anime VRoid/MToon (176 B de MaterialUniform, slots main/shade/2nd shade/emission/matcap/outline width, composição no cel_shading + inverted_hull, paridade headless↔viewport, frame congelado intacto)
- [ ] #17 Face SDF · #43 Eye · #53 DoF · #42 CSM · #26 Texturas · #59 relatório

## Escopo por issue

| # | Issue | Entrega principal | Verificação |
|---|-------|-------------------|-------------|
| 25 | [Assets] Parser/Exportador glTF 2.0 + VRM 1.0 | `src/services/vrm/*` (parser completo, extensões VRM 1.0, exportador determinístico), espelho Rust em `anigo-vrm`, comandos Tauri | tests node (roundtrip, fixtures VRM, determinismo) |
| 18 | [Shading] Material Anime MToon | slots de textura (main/shade/2nd shade/emission/matcap/outline width), composição no `cel_shading.wgsl`, bloco `material_mtoon` no contrato | check:wgsl, fixtures, paridade de reference frame |
| 17 | [Shading] Face Shadow SDF | integração do SDF facial no passe de cel (projeção angular em espaço local da cabeça), paridade headless↔viewport | check:wgsl (bloco compartilhado), reference frame, UI |
| 43 | [Shading] Motor de Olhos/Íris | `anime_eye.wgsl` (parallax + highlights desacoplados), look-at solver com micro-sacadas (TS + Rust) | tests do solver, check:wgsl |
| 53 | [Camera] Cinematográfica | presets de lente (24–135mm), CoC/DoF (`postprocess_dof.wgsl`), target tracking com damping | tests do math, check:wgsl, UI |
| 42 | [Renderer] CSM | `shadow_pass.wgsl`, filtragem toon (`step`) com slope-scaled bias, cascades 2–3, integração com hue-shift | tests de cascades, check:wgsl |
| 26 | [Assets] Texturas + Mipmaps | `texture_manager` (cache FNV-1a, decode assíncrono, mip chain), `mip_blit.wgsl` | tests (mip count, downsample, cache) |
| 59 | feat: VRM interop + shading contracts (úmbrella) | fechamento com relatório | todos os gates |

## Gates (rodar após cada etapa)

- `npm test` (baseline 278 passing)
- `npm run build`
- `npm run check:wgsl`
- `npm run fixtures:check`
- `node scripts/check_rust_syntax.mjs`
- `git diff --check`

Notas de ambiente: sem `cargo`/`rustc` neste sandbox (conforme #59) — o lado Rust é
validado por syntax-check (tree-sitter) e paridade manual com o contrato; `cargo test`
fica documentado como follow-up de CI.
