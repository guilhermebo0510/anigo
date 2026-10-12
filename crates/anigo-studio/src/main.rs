use anigo_studio::app::StudioApp;
use anigo_studio::cli;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,wgpu=warn,naga=warn".into()))
        .init();
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("erro: {e}\nuso: anigo-studio [--workspace <chave>] [--screenshot <png>] [--frames <n>] [--size LxA] [--scenario <nome>] [--session-file <json>] [--no-session] [--lang pt|en] [--bridge] [--bridge-port <n>] [--bridge-discovery <arquivo>] [--layout-file <json>] [--keymap-file <json>]");
            std::process::exit(2);
        }
    };
    let (w, h) = cli.size.unwrap_or((1440.0, 900.0));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ANIGO Studio")
            .with_inner_size([w, h])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("anigo-studio", options, Box::new(move |cc| Ok(Box::new(StudioApp::new(cc, cli)))))
}
