//! ANIGO VRM and glTF 2.0 Anime Standard Engine.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VrmMeta {
    pub title: String,
    pub version: String,
    pub author: String,
    pub contact_information: Option<String>,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendShapeGroup {
    pub name: String,
    pub preset_name: String,
    pub weight: f32,
}
