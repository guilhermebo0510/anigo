//! ANIGO Autonomous AI Agent and Studio Automation Engine.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationPrompt {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub style_preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub command: String,
    pub status: String,
}
