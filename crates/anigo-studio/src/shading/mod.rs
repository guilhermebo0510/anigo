//! Lógica pura do workspace Shading (sem egui): parâmetros do material toon, paleta, shader ball /
//! layout do preview e presets. A UI vive em `crate::app::shading_ui`.

pub mod ball;
pub mod palette;
pub mod params;
pub mod presets;

#[cfg(test)]
mod visual_tests;
