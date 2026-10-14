# Referências de anime (calibração do look)

Coloque aqui os quadros de anime usados para calibrar o shading (`docs/native/SHADING_ANIME_PLANO.md`).

* **As imagens NÃO vão para o git** (direitos autorais): o `.gitignore` desta pasta ignora `png/jpg/webp/...`. Só este README e os
  manifestos `*.manifest.json` são versionados.
* Um quadro por cena, ou um recorte do personagem; quanto mais limpo o fundo, melhor a medição (senão use uma máscara `*.mask.png`,
  branco = personagem).
* Nomeie `titulo_cena01.png`, e crie `titulo_cena01.manifest.json` ao lado (veja `exemplo.manifest.json`).

Medir e comparar:

```text
python scripts/look_calibration/analyze_frame.py referencias/anime/titulo_cena01.png --out referencias/anime/_medidas/
python scripts/look_calibration/capture_app.py referencias/anime/_medidas/render.png --render 1920x1080
python scripts/look_calibration/compare.py referencias/anime/titulo_cena01.png referencias/anime/_medidas/render.png --out referencias/anime/_medidas/
```
