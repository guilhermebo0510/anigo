//! Matemática de viewport: câmera orbital, projeção e picking (sem GPU, testável).

pub mod camera;
pub mod nav_gizmo;
pub mod picking;

pub use camera::{OrbitCamera, ViewPreset};
