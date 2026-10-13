//! Confere uma pele esculpida contra o contrato de topologia e re-deriva esqueleto/pesos.
//!
//! Uso: `cargo run -p anigo-charbase --example check_sculpt -- <esculpido.glb> [saida.glb]`
//!
//! 1. lê o primeiro primitivo do GLB e solda as costuras de UV;
//! 2. `validate_topology_tris`: Exact = mesma contagem/ordem/conectividade (tudo do contrato vale);
//! 3. `rederive`: re-localiza as juntas pelos localizadores (Exact) ou por vizinho mais próximo
//!    (topologia diferente) e transfere os pesos;
//! 4. se informado `saida.glb` e a topologia é Exact, grava um GLB com a sua base esculpida, o
//!    esqueleto re-derivado, os pesos do contrato e os morphs do gerador reaplicados como deltas
//!    aditivos sobre a base nova.

use anigo_charbase::contract::{rederive_tris, validate_topology_tris, RederiveMethod};
use anigo_charbase::export::{export_glb, ExportOptions};
use anigo_charbase::import::import_glb_skin;
use anigo_charbase::morphs::build_morphs;
use anigo_charbase::qmesh::QuadMesh;
use anigo_charbase::topology::reference;
use anigo_charbase::{generate, Gender};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("uso: check_sculpt <esculpido.glb> [saida.glb]");
        std::process::exit(2);
    };
    let out = args.next();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("não consegui ler {path}: {e}");
            std::process::exit(1);
        }
    };
    let skin = match import_glb_skin(&bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("GLB inválido: {e}");
            std::process::exit(1);
        }
    };
    println!("arquivo: {} vértices (após soldar costuras: {}), {} triângulos", skin.file_vertices, skin.positions.len(), skin.tris.len());
    let rep = validate_topology_tris(&skin.positions, &skin.tris);
    println!("topologia: {:?}  (esperado {} vértices / {} triângulos)", rep.status, rep.expected_vertex_count, rep.expected_face_count * 2);
    for m in &rep.messages {
        println!("  - {m}");
    }
    match rederive_tris(&skin.positions, &skin.tris) {
        Ok(r) => {
            println!("re-derivação: {:?}", r.method);
            for n in ["hips", "head", "leftUpperArm", "leftHand", "leftLowerLeg", "leftFoot", "leftEye", "jaw"] {
                if let Some(i) = r.skeleton.index(n) {
                    let p = r.skeleton.bones[i].world;
                    println!("  {n:14} ({:.3}, {:.3}, {:.3})", p[0], p[1], p[2]);
                }
            }
            if let (Some(out), true) = (out, r.method == RederiveMethod::Exact && rep.is_exact()) {
                // base = geração feminina/masculina mais próxima; as posições vêm do esculpido
                let mut ch = generate(Gender::Female);
                ch.positions = skin.positions.clone();
                ch.normals = QuadMesh { pos: ch.positions.clone(), faces: reference().faces.clone() }.vertex_normals();
                ch.skeleton = r.skeleton.clone();
                let morphs = build_morphs(&generate(Gender::Female));
                match export_glb(&ch, &morphs, &ExportOptions { vrm: true, title: "ANIGO esculpido".into(), author: "ANIGO".into() }) {
                    Ok(glb) => {
                        std::fs::write(&out, glb).expect("gravar");
                        println!("gravado {out}");
                    }
                    Err(e) => eprintln!("exportação falhou: {e}"),
                }
            }
        }
        Err(e) => {
            eprintln!("re-derivação impossível: {e}");
            std::process::exit(1);
        }
    }
}
