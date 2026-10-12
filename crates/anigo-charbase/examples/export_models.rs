//! Gera `assets/models_v2/` (GLB e VRM masculino/feminino) e o `topology_contract.json`.
//! Uso: `cargo run -p anigo-charbase --example export_models [pasta_de_saida]`
//! (padrão: `<workspace>/assets/models_v2`). Determinístico: rodar duas vezes gera os mesmos bytes.

use anigo_charbase::contract::contract_json_string;
use anigo_charbase::export::{export_glb, summarize, ExportOptions};
use anigo_charbase::morphs::build_morphs;
use anigo_charbase::{generate, Gender};
use std::path::PathBuf;

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models_v2"));
    std::fs::create_dir_all(&out).expect("criar pasta de saída");
    for (name, g) in [("anime_male_v2", Gender::Male), ("anime_female_v2", Gender::Female)] {
        let ch = generate(g);
        let morphs = build_morphs(&ch);
        let title = if matches!(g, Gender::Male) { "ANIGO Anime Male v2 (Gen-0)" } else { "ANIGO Anime Female v2 (Gen-0)" };
        // Um arquivo por gênero: GLB com skin de 65 ossos, morph targets nomeados e a camada VRMC_vrm 1.0
        // (humanoid, expressões, lookAt). Qualquer visualizador glTF ignora a extensão; leitores VRM a usam.
        let glb = export_glb(&ch, &morphs, &ExportOptions { vrm: true, title: title.into(), author: "ANIGO".into() }).expect("exportar");
        std::fs::write(out.join(format!("{name}.glb")), &glb).expect("gravar glb");
        println!("{name}: {} KB", glb.len() / 1024);
        println!("{}", serde_json::to_string(&summarize(&ch, &morphs)).unwrap());
    }
    std::fs::write(out.join("topology_contract.json"), contract_json_string()).expect("gravar contrato");
    println!("contrato: {} KB", contract_json_string().len() / 1024);
}
