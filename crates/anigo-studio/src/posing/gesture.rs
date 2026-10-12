//! Gesto de edição de pose: durante o arrasto cada passo é um `preview` (fora do histórico) calculado
//! pela diferença entre a pose AO VIVO e a pose desejada; ao soltar vira UM `commit_preview` calculado
//! entre a pose do início e a última pose desejada. Esc/cancelar restaura o início.

use anigo_core::command::Command;
use anigo_core::pose::{plan_pose_change, PoseOp, PoseState};

/// Estado de um gesto aberto.
#[derive(Debug, Clone)]
pub struct GestureTracker {
    /// Pose no início do gesto.
    pub base: PoseState,
    /// Última pose desejada.
    pub last: PoseState,
    pub op: PoseOp,
    /// Algum passo realmente mudou a pose.
    pub moved: bool,
}

impl GestureTracker {
    pub fn begin(base: PoseState, op: PoseOp) -> Self {
        GestureTracker { last: base.clone(), base, op, moved: false }
    }

    /// Comando de PREVIEW que leva a pose ao vivo (`live`) até a desejada `target`. `None` se já é igual.
    pub fn step(&mut self, live: &PoseState, target: PoseState) -> Option<Command> {
        let cmd = plan_pose_change(live, &target, self.op);
        self.last = target;
        if cmd.is_some() {
            self.moved = true;
        }
        cmd
    }

    /// Comando final (início → última desejada). `None` quando o resultado líquido é nenhuma mudança.
    pub fn finish(&self) -> Option<Command> {
        plan_pose_change(&self.base, &self.last, self.op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::pose::bone;
    use anigo_session::CoreSession;
    use glam::Quat;

    fn drag(session: &mut CoreSession, g: &mut GestureTracker, target: PoseState) {
        if let Some(cmd) = g.step(session.pose(), target) {
            session.preview(cmd).expect("preview");
        }
    }

    #[test]
    fn a_drag_is_many_previews_and_one_undo_step() {
        let mut s = CoreSession::new();
        let mut g = GestureTracker::begin(s.pose().clone(), PoseOp::Edit);
        s.begin_preview();
        for deg in [10.0f32, 35.0, 60.0, 90.0] {
            let mut t = g.base.clone();
            t.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-deg.to_radians()));
            drag(&mut s, &mut g, t);
            assert_eq!(s.history().undo_depth(), 0);
        }
        let fin = g.finish().expect("houve mudança líquida");
        s.commit_preview(fin).expect("commit");
        assert_eq!(s.history().undo_depth(), 1);
        assert!((s.pose().rotation(bone::L_ELBOW).to_axis_angle().1.to_degrees() - 90.0).abs() < 1e-3);
        s.undo().unwrap();
        assert!(s.pose().is_rest());
    }

    #[test]
    fn bones_touched_by_an_earlier_step_are_restored_when_the_target_changes_back() {
        // IK mexe no ombro+cotovelo; depois o alvo volta e o cotovelo precisa voltar também (diff contra a pose AO VIVO)
        let mut s = CoreSession::new();
        let mut g = GestureTracker::begin(s.pose().clone(), PoseOp::Ik);
        s.begin_preview();
        let mut t1 = g.base.clone();
        t1.set_rotation(bone::L_SHOULDER, Quat::from_rotation_z(1.0));
        t1.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-1.0));
        drag(&mut s, &mut g, t1);
        let mut t2 = g.base.clone();
        t2.set_rotation(bone::L_SHOULDER, Quat::from_rotation_z(0.3));
        drag(&mut s, &mut g, t2.clone());
        assert!(s.pose().rotation(bone::L_ELBOW).dot(Quat::IDENTITY).abs() > 0.99999, "cotovelo voltou ao repouso na pose ao vivo");
        s.commit_preview(g.finish().unwrap()).unwrap();
        assert_eq!(s.pose(), &t2);
        assert_eq!(s.history().undo_depth(), 1);
    }

    #[test]
    fn a_drag_that_ends_where_it_started_leaves_no_history() {
        let mut s = CoreSession::new();
        let mut g = GestureTracker::begin(s.pose().clone(), PoseOp::Edit);
        s.begin_preview();
        let mut t = g.base.clone();
        t.set_rotation(bone::HEAD, Quat::from_rotation_y(0.8));
        drag(&mut s, &mut g, t);
        let back = g.base.clone();
        drag(&mut s, &mut g, back);
        assert!(g.moved);
        assert!(g.finish().is_none());
        s.cancel_preview();
        assert_eq!(s.history().undo_depth(), 0);
        assert!(s.pose().is_rest());
    }
}
