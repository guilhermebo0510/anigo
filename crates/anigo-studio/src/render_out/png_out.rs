//! Escrita de PNG/JPEG da saída de render: alfa reto, 8/16 bits, `pHYs` (DPI), marcação de espaço de cor
//! (`sRGB` ou `cICP` Display P3) e metadados `anigo:*` em `iTXt` (UTF-8). A escrita é atômica (arquivo `.tmp` + rename).
//!
//! Os bytes são função só dos pixels e dos metadados: nada de data/hora, pid ou caminho entra no arquivo.

use png::{BitDepth, ColorType, Compression, Encoder, FilterType, PixelDimensions, SrgbRenderingIntent, Unit};
use std::io::Write;
use std::path::Path;

/// Como o espaço de cor é sinalizado no PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngColor {
    /// Chunk `sRGB` (intenção perceptual) + `gAMA`/`cHRM` de compatibilidade.
    Srgb,
    /// Chunk `cICP`: primárias 12 (Display P3), transferência 13 (a curva do sRGB), matriz 0, faixa total.
    DisplayP3,
    /// Dados (profundidade, ID, normais…): sem marcação de cor.
    Data,
}

#[derive(Debug, Clone)]
pub struct PngSpec<'a> {
    pub w: u32,
    pub h: u32,
    pub color_type: ColorType,
    pub depth: BitDepth,
    pub data: &'a [u8],
    pub color: PngColor,
    pub dpi: Option<u32>,
    /// (palavra-chave ASCII, valor UTF-8). Palavra-chave: 1–79 bytes, Latin-1 imprimível.
    pub texts: Vec<(String, String)>,
}

/// Codifica o PNG em memória.
pub fn encode_png(spec: &PngSpec<'_>) -> Result<Vec<u8>, String> {
    let channels = match spec.color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => return Err("PNG indexado não é suportado".into()),
    };
    let bytes = if spec.depth == BitDepth::Sixteen { 2 } else { 1 };
    let expected = spec.w as usize * spec.h as usize * channels * bytes;
    if spec.data.len() != expected {
        return Err(format!("dados de imagem com {} bytes; {}×{} exige {expected}", spec.data.len(), spec.w, spec.h));
    }
    let mut out = Vec::with_capacity(expected / 2 + 1024);
    {
        let mut enc = Encoder::new(&mut out, spec.w, spec.h);
        enc.set_color(spec.color_type);
        enc.set_depth(spec.depth);
        enc.set_compression(Compression::Default);
        enc.set_filter(FilterType::Sub);
        if let Some(dpi) = spec.dpi {
            let ppm = (dpi as f64 / 0.0254).round() as u32;
            enc.set_pixel_dims(Some(PixelDimensions { xppu: ppm, yppu: ppm, unit: Unit::Meter }));
        }
        match spec.color {
            PngColor::Srgb => enc.set_source_srgb(SrgbRenderingIntent::Perceptual),
            PngColor::DisplayP3 | PngColor::Data => {}
        }
        for (k, v) in &spec.texts {
            if k.is_empty() || k.len() > 79 || !k.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                return Err(format!("palavra-chave PNG inválida: '{k}'"));
            }
            enc.add_itxt_chunk(k.clone(), v.clone()).map_err(|e| e.to_string())?;
        }
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        if spec.color == PngColor::DisplayP3 {
            // cICP precisa vir antes de PLTE/IDAT: logo depois do cabeçalho
            writer.write_chunk(png::chunk::ChunkType(*b"cICP"), &[12, 13, 0, 1]).map_err(|e| e.to_string())?;
        }
        writer.write_image_data(spec.data).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// JPEG RGB 8 bits (sem alfa) com `quality` 1..=100.
pub fn encode_jpeg(w: u32, h: u32, rgb: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100));
    image::ImageEncoder::write_image(enc, rgb, w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Grava `bytes` em `path` de forma atômica (cria a pasta; escreve `.tmp` e renomeia).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp-anigo");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// O que dá para ler de volta de um PNG nosso (testes e histórico).
#[derive(Debug, Clone, Default)]
pub struct PngInfo {
    pub w: u32,
    pub h: u32,
    pub color_type: Option<ColorType>,
    pub depth: Option<BitDepth>,
    pub dpi: Option<u32>,
    pub srgb: bool,
    pub cicp: Option<[u8; 4]>,
    pub texts: Vec<(String, String)>,
}

/// Lê cabeçalho e metadados (sem decodificar os pixels além do necessário) de bytes PNG.
pub fn read_info(bytes: &[u8]) -> Result<PngInfo, String> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = dec.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let mut out = PngInfo {
        w: info.width,
        h: info.height,
        color_type: Some(info.color_type),
        depth: Some(info.bit_depth),
        srgb: info.srgb.is_some(),
        ..Default::default()
    };
    if let Some(d) = info.pixel_dims {
        if d.unit == Unit::Meter {
            out.dpi = Some((d.xppu as f64 * 0.0254).round() as u32);
        }
    }
    for t in &info.utf8_text {
        if let Ok(text) = t.get_text() {
            out.texts.push((t.keyword.clone(), text));
        }
    }
    // cICP não é exposto pela API do decoder 0.17: varre os chunks até IDAT
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        if kind == b"IDAT" {
            break;
        }
        if kind == b"cICP" && len == 4 && pos + 12 <= bytes.len() {
            out.cicp = Some([bytes[pos + 8], bytes[pos + 9], bytes[pos + 10], bytes[pos + 11]]);
        }
        pos += 12 + len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(data: &'a [u8], color: PngColor) -> PngSpec<'a> {
        PngSpec {
            w: 2,
            h: 2,
            color_type: ColorType::Rgba,
            depth: BitDepth::Eight,
            data,
            color,
            dpi: Some(300),
            texts: vec![("anigo:project".into(), "Projeção — ação".into()), ("Software".into(), "ANIGO".into())],
        }
    }

    #[test]
    fn png_carries_alpha_dpi_color_and_utf8_metadata() {
        let data = [255u8, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 40];
        let bytes = encode_png(&spec(&data, PngColor::Srgb)).unwrap();
        let info = read_info(&bytes).unwrap();
        assert_eq!((info.w, info.h), (2, 2));
        assert_eq!(info.color_type, Some(ColorType::Rgba));
        assert_eq!(info.dpi, Some(300));
        assert!(info.srgb && info.cicp.is_none());
        assert!(info.texts.contains(&("anigo:project".to_string(), "Projeção — ação".to_string())), "{:?}", info.texts);
        // decodifica de volta: pixels idênticos (alfa reto preservado)
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(img.as_raw().as_slice(), &data);
    }

    #[test]
    fn display_p3_is_signalled_with_cicp_before_the_pixels() {
        let data = [0u8; 16];
        let bytes = encode_png(&spec(&data, PngColor::DisplayP3)).unwrap();
        let info = read_info(&bytes).unwrap();
        assert_eq!(info.cicp, Some([12, 13, 0, 1]));
        assert!(!info.srgb);
        let data_png = encode_png(&spec(&data, PngColor::Data)).unwrap();
        let i = read_info(&data_png).unwrap();
        assert!(!i.srgb && i.cicp.is_none());
    }

    #[test]
    fn sixteen_bit_gray_round_trips() {
        let vals: [u16; 4] = [0, 1000, 40000, 65535];
        let mut raw = Vec::new();
        for v in vals {
            raw.extend_from_slice(&v.to_be_bytes());
        }
        let s = PngSpec { w: 2, h: 2, color_type: ColorType::Grayscale, depth: BitDepth::Sixteen, data: &raw, color: PngColor::Data, dpi: None, texts: vec![] };
        let bytes = encode_png(&s).unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_luma16();
        assert_eq!(img.as_raw().as_slice(), &vals);
    }

    #[test]
    fn encoding_is_deterministic_and_validates_input() {
        let data = [7u8; 16];
        let a = encode_png(&spec(&data, PngColor::Srgb)).unwrap();
        let b = encode_png(&spec(&data, PngColor::Srgb)).unwrap();
        assert_eq!(a, b);
        assert!(encode_png(&spec(&data[..15], PngColor::Srgb)).is_err());
        let mut bad = spec(&data, PngColor::Srgb);
        bad.texts = vec![("açaí".into(), "x".into())];
        assert!(encode_png(&bad).is_err(), "palavra-chave fora do Latin-1 imprimível ASCII que usamos");
    }

    #[test]
    fn atomic_write_replaces_without_leaving_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("a.png");
        write_atomic(&p, b"um").unwrap();
        write_atomic(&p, b"dois").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"dois");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn jpeg_encodes_rgb() {
        let rgb = vec![200u8; 8 * 8 * 3];
        let bytes = encode_jpeg(8, 8, &rgb, 90).unwrap();
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
        let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert!((img.get_pixel(3, 3).0[0] as i32 - 200).abs() < 4);
    }
}
