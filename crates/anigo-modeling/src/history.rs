//! Undo/redo do edit mode.
//!
//! * [`SnapshotHistory`]: cada passo guarda a malha inteira (simples e
//!   robusto; malhas de personagem anime têm dezenas de milhares de faces).
//! * [`PositionDelta`]: delta esparso de posições para traços de escultura,
//!   onde um snapshot por pincelada seria desperdício.

use crate::mesh::EditMesh;
use glam::Vec3;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct SnapshotHistory {
    undo: VecDeque<(String, EditMesh)>,
    redo: Vec<(String, EditMesh)>,
    limit: usize,
}

impl SnapshotHistory {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Registra o estado **antes** de uma operação. Invalida o redo.
    pub fn push(&mut self, label: impl Into<String>, before: &EditMesh) {
        self.redo.clear();
        self.undo.push_back((label.into(), before.clone()));
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|(l, _)| l.as_str())
    }

    /// Desfaz: troca `current` pelo estado anterior e guarda `current` no redo.
    pub fn undo(&mut self, current: &mut EditMesh) -> Option<String> {
        let (label, prev) = self.undo.pop_back()?;
        let now = std::mem::replace(current, prev);
        self.redo.push((label.clone(), now));
        Some(label)
    }

    pub fn redo(&mut self, current: &mut EditMesh) -> Option<String> {
        let (label, next) = self.redo.pop()?;
        let now = std::mem::replace(current, next);
        self.undo.push_back((label.clone(), now));
        Some(label)
    }

    pub fn depth(&self) -> usize {
        self.undo.len()
    }
}

/// Delta esparso de posições (índice, antes, depois).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PositionDelta {
    pub entries: Vec<(u32, Vec3, Vec3)>,
}

impl PositionDelta {
    /// Constrói o delta comparando duas listas de posições.
    pub fn between(before: &[Vec3], after: &[Vec3]) -> Self {
        let entries = before
            .iter()
            .zip(after)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i as u32, *a, *b))
            .collect();
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn apply(&self, mesh: &mut EditMesh) {
        for &(i, _, after) in &self.entries {
            if let Some(v) = mesh.verts.get_mut(i as usize) {
                v.co = after;
            }
        }
    }

    pub fn revert(&self, mesh: &mut EditMesh) {
        for &(i, before, _) in &self.entries {
            if let Some(v) = mesh.verts.get_mut(i as usize) {
                v.co = before;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn undo_redo_roundtrip_and_redo_invalidation() {
        let mut mesh = primitives::cube(1.0);
        let original = mesh.clone();
        let mut h = SnapshotHistory::new(8);

        h.push("mover", &mesh);
        mesh.verts[0].co.x += 5.0;
        let edited = mesh.clone();

        assert_eq!(h.undo(&mut mesh).as_deref(), Some("mover"));
        assert_eq!(mesh, original);
        assert_eq!(h.redo(&mut mesh).as_deref(), Some("mover"));
        assert_eq!(mesh, edited);

        h.undo(&mut mesh);
        h.push("outra", &mesh); // nova edição invalida redo
        assert!(!h.can_redo());
    }

    #[test]
    fn history_respects_limit() {
        let mesh = primitives::cube(1.0);
        let mut h = SnapshotHistory::new(3);
        for i in 0..10 {
            h.push(format!("op{i}"), &mesh);
        }
        assert_eq!(h.depth(), 3);
        assert_eq!(h.undo_label(), Some("op9"));
    }

    #[test]
    fn position_delta_apply_revert() {
        let mut mesh = primitives::cube(1.0);
        let before: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
        mesh.verts[3].co += Vec3::splat(0.5);
        let after: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
        let d = PositionDelta::between(&before, &after);
        assert_eq!(d.entries.len(), 1);
        d.revert(&mut mesh);
        assert_eq!(mesh.verts[3].co, before[3]);
        d.apply(&mut mesh);
        assert_eq!(mesh.verts[3].co, after[3]);
    }
}
