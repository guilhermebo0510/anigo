//! ANIGO Procedural Spline Hair Generation Engine.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HairControlPoint {
    pub position: Vec3,
    pub width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HairStrand {
    pub id: String,
    pub control_points: Vec<HairControlPoint>,
    pub segment_resolution: u32,
}

impl HairStrand {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            control_points: Vec::new(),
            segment_resolution: 16,
        }
    }
}
