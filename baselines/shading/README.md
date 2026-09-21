# P3-13 Golden images — run anigo_compare_baseline via MCP to generate baselines for 12 presets (4 light x 3 bands)

## P0 "Consolidar o renderer" — paridade sem GPU

O bloco P0 do renderer unifica headless e viewport sobre
`contracts/fixtures/render_contract_v1.json`: os mesmos shaders (hash FNV-1a de 64
bits conferido dos dois lados), o mesmo layout de vértice/uniform, o mesmo grafo
de passes, o mesmo MSAA/formato e o mesmo toon ramp (bytes congelados).

Isso é verificado sem GPU em `tests/contracts/render_contract.test.ts` e em
`cargo test -p anigo-renderer render_contract`. O que **não** dá para verificar
sem adapter é o pixel final: um golden render precisa de GPU e entra aqui com
`anigo_compare_baseline` (mesmo caminho de código do viewport, `render_scene` do
headless), comparando com tolerância perceptual.
