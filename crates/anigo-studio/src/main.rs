use eframe::egui;

struct Spike {
    frame: u32,
    shot: Option<String>,
}

impl eframe::App for Spike {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.label("ANIGO spike");
        });
        egui::Panel::left("left").show(ui, |ui| {
            ui.label("tools");
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(format!("frame {}", self.frame));
        });
        self.frame += 1;
        let ctx = ui.ctx().clone();
        if self.frame == 5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shots: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        for img in shots {
            if let Some(path) = &self.shot {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = std::fs::write(format!("{path}.raw"), &rgba);
                eprintln!("screenshot {}x{}", img.size[0], img.size[1]);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let shot = std::env::args().nth(1);
    eframe::run_native(
        "spike",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(Spike { frame: 0, shot }))),
    )
}
