//! Estatísticas reais do personagem base (números citados em docs/native/PERSONAGEM_BASE.md).
use anigo_charbase::export::summarize;
use anigo_charbase::morphs::{build_morphs, MorphGroup};
use anigo_charbase::parts::parts_topology;
use anigo_charbase::qmesh::QuadMesh;
use anigo_charbase::topology::reference;
use anigo_charbase::{generate, Gender};
use std::collections::BTreeMap;

fn main() {
    let t = reference();
    for g in [Gender::Female, Gender::Male] {
        let ch = generate(g);
        let m = QuadMesh { pos: ch.positions.clone(), faces: t.faces.clone() };
        let val = m.valences();
        let mut hist: BTreeMap<u32, usize> = BTreeMap::new();
        for v in &val {
            *hist.entry(*v).or_insert(0) += 1;
        }
        let (mn, mx) = ch.positions.iter().fold((glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
        println!("{g:?}: V={} F={} E={} euler={} boundary_loops={} bbox min={mn:?} max={mx:?}", m.pos.len(), m.faces.len(), m.edge_count(), m.euler(), m.boundary_loops().len());
        println!("  valencias {hist:?}  regulares={:.2}%", 100.0 * *hist.get(&4).unwrap_or(&0) as f32 / val.len() as f32);
        let head = ch.skeleton.pos(ch.skeleton.index("head").unwrap());
        let top = ch.positions.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        println!("  altura {:.3} m, cabecas (altura / (topo - queixo)): {:.2}", top, top / (top - t.landmark("lm_chin").unwrap().eval(&ch.positions).y));
        println!("  osso head y={:.3}", head.y);
        let ms = build_morphs(&ch);
        let mut by_group: BTreeMap<&str, (usize, usize, f32, usize)> = BTreeMap::new();
        for tg in &ms.targets {
            let k = match tg.group {
                MorphGroup::Macro => "macro",
                MorphGroup::Face => "face",
                MorphGroup::Expression => "expressao",
            };
            let e = by_group.entry(k).or_insert((0, 0, 0.0, 0));
            e.0 += 1;
            e.1 += tg.deltas.len();
            e.2 = e.2.max(tg.max_delta());
            e.3 = e.3.max(tg.deltas.len());
        }
        println!("  morphs (grupo: n alvos, soma de deltas, delta max (m), max deltas num alvo): {by_group:?}");
        println!("  {}", serde_json::to_string(&summarize(&ch, &ms)).unwrap());
    }
    let mut regions: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &t.region {
        *regions.entry(r.name()).or_insert(0) += 1;
    }
    println!("regioes: {regions:?}");
    println!("gaiola: {} vertices", t.n_cage);
    let parts: Vec<String> = parts_topology().iter().map(|(n, m, tr, uv)| format!("{n}({m}): {} v, {} tri", uv.len(), tr.len())).collect();
    println!("pecas: {parts:?}");
    println!("laços: {}", t.loops.len());
    println!("landmarks: {}", t.landmarks.len());
    let mean_infl: f32 = t.weights.iter().map(|w| w.iter().filter(|x| x.1 > 0.0).count() as f32).sum::<f32>() / t.n_verts as f32;
    println!("influencias medias por vertice: {mean_infl:.2}");
    println!("conectividade hash {:016x}", t.connectivity_hash());
}
