# Calibração do look por referência

Ferramentas (só pesquisa/QA, em Python) para **medir o look** de um quadro de anime e do render do ANIGO e compará-los número a número.
É a base do plano de `docs/native/SHADING_ANIME_PLANO.md` (fase 0: calibrar antes de construir).

```text
python scripts/look_calibration/analyze_frame.py referencias/anime/cena01.png --out saida/      # mede um quadro
python scripts/look_calibration/capture_app.py saida/render.png --render 1920x1080              # render real do ANIGO (app aberto com --bridge)
python scripts/look_calibration/compare.py referencias/anime/cena01.png saida/render.png --out saida/
python -m unittest scripts/look_calibration/test_look_calibration.py                            # testes com esferas cel sintéticas
```

Dependências: `numpy`, `scipy`, `scikit-image`, `scikit-learn`, `Pillow` (`pip install numpy scipy scikit-image scikit-learn pillow`).

## O que é medido

| Métrica | Como | Para que serve |
|---|---|---|
| Paleta (8 cores) | k-means em Lab sobre o personagem | base_color / shade_color |
| Faixas de tom + "posterização" | picos do histograma de luminância (só no interior: a linha não conta) | `toon_steps`, degraus |
| Dureza da sombra (px) | área da zona entre faixas ÷ comprimento da borda | `shadow_smoothness` |
| Linha (px e relativo à altura) + cobertura | esqueleto dos pixels escuros junto da silhueta | `outline_width`, opacidade |
| Realces | top-hat (claros e pequenos) | `specular_size`, `spec_intensity` |
| Rim | anel interno vs. miolo | `rim_intensity`, `rim_spread` |
| Sombra: ΔL, Δmatiz, razão de croma | faixa clara → faixa escura | `shade_color`, `hue_shift` |

`compare.py` imprime a tabela, uma dica de ajuste por linha e uma nota 0–100. A nota é um **guia**; o olho decide.

## Limites conhecidos (honestidade)

* Fundo complexo: passe `--mask` (branco = personagem); sem máscara a cor de fundo é estimada nas bordas.
* Faixas de sombra muito finas (boneco de teste) somem depois de excluir a linha do contorno; em personagens reais a sombra é larga.
* Uma imagem com várias luzes coloridas pode somar mais de duas faixas "reais"; leia a paleta junto.
* Mede **um** quadro. Comparar vários quadros da mesma cena (planos diferentes) é o passo seguinte do plano.
