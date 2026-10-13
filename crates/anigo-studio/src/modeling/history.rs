//! Histórico genérico por snapshot (undo/redo do documento de modelagem).

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct History<T: Clone> {
    undo: VecDeque<(String, T)>,
    redo: Vec<(String, T)>,
    limit: usize,
}

impl<T: Clone> History<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Guarda o estado ANTES de uma operação; invalida o redo.
    pub fn record(&mut self, label: impl Into<String>, before: &T) {
        self.redo.clear();
        self.undo.push_back((label.into(), before.clone()));
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
    }

    /// Remove a última entrada de undo SEM aplicá-la (usado ao refazer uma operação com novos parâmetros).
    pub fn pop_undo(&mut self) -> Option<String> {
        self.undo.pop_back().map(|(l, _)| l)
    }

    /// Remove a entrada de redo mais recente SEM aplicá-la.
    pub fn pop_redo(&mut self) -> Option<String> {
        self.redo.pop().map(|(l, _)| l)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, current: &mut T) -> Option<String> {
        let (label, prev) = self.undo.pop_back()?;
        let now = std::mem::replace(current, prev);
        self.redo.push((label.clone(), now));
        Some(label)
    }

    pub fn redo(&mut self, current: &mut T) -> Option<String> {
        let (label, next) = self.redo.pop()?;
        let now = std::mem::replace(current, next);
        self.undo.push_back((label.clone(), now));
        Some(label)
    }

    /// Rótulos do mais antigo ao mais recente (painel Histórico).
    pub fn labels(&self) -> Vec<&str> {
        self.undo.iter().map(|(l, _)| l.as_str()).collect()
    }

    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo.iter().rev().map(|(l, _)| l.as_str()).collect()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_limit() {
        let mut h = History::new(2);
        let mut v = 0;
        for i in 1..=3 {
            h.record(format!("set {i}"), &v);
            v = i;
        }
        assert_eq!(h.labels(), ["set 2", "set 3"]);
        assert_eq!(h.undo(&mut v).as_deref(), Some("set 3"));
        assert_eq!(v, 2);
        assert_eq!(h.redo(&mut v).as_deref(), Some("set 3"));
        assert_eq!(v, 3);
        h.undo(&mut v);
        h.record("novo", &v);
        assert!(!h.can_redo());
    }
}
