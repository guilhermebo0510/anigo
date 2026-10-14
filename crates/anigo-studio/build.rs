//! Embute o ícone e as informações de versão no `anigo-studio.exe` (só no Windows, só se o `rc.exe` existir).

fn main() {
    println!("cargo:rerun-if-changed=assets/brand/anigo.rc");
    println!("cargo:rerun-if-changed=assets/brand/anigo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // sem `rc.exe` (ambiente sem o Windows SDK) o build segue sem o ícone do arquivo; o da janela vem do `brand::window_icon`
        let _ = embed_resource::compile("assets/brand/anigo.rc", embed_resource::NONE).manifest_optional();
    }
}
