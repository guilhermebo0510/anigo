//! Depuração: imprime pesos e atributos de um vértice. `dev_debug <idx>`
use anigo_charbase::topology::reference;
use anigo_charbase::{generate, Gender};

fn main() {
    let v: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let t = reference();
    let c = generate(Gender::Female);
    for &i in &[v, t.mirror[v] as usize] {
        println!(
            "v={i} region={:?} attr={:?} pos={:?}",
            t.region[i], t.attr[i], c.positions[i]
        );
        for w in &t.weights[i] {
            println!("   {} {}", c.skeleton.bones[w.0 as usize].name, w.1);
        }
    }
}
