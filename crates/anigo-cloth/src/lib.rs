//! ANIGO Real-Time Cloth and Tailoring Simulation Engine.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClothParticle {
    pub position: Vec3,
    pub prev_position: Vec3,
    pub acceleration: Vec3,
    pub inv_mass: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistanceConstraint {
    pub p1: usize,
    pub p2: usize,
    pub rest_length: f32,
    pub stiffness: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClothMesh {
    pub name: String,
    pub particles: Vec<ClothParticle>,
    pub constraints: Vec<DistanceConstraint>,
}
