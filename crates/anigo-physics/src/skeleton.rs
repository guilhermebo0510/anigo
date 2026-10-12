//! Hierarquia mínima de nós (TRS local + matrizes de mundo) usada pelo spring bone. Espelha o que o
//! VRM precisa: pose de repouso, pose animada, FK e recomputo parcial de subárvore.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::error::{PhysicsError, PhysicsResult};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkeletonNode {
    pub name: String,
    pub parent: Option<usize>,
    /// Pose de repouso local.
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for SkeletonNode {
    fn default() -> Self {
        Self { name: String::new(), parent: None, translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl SkeletonNode {
    pub fn new(name: impl Into<String>, parent: Option<usize>, translation: Vec3) -> Self {
        Self { name: name.into(), parent, translation, ..Default::default() }
    }
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    nodes: Vec<SkeletonNode>,
    children: Vec<Vec<usize>>,
    /// Pais antes dos filhos.
    order: Vec<usize>,
    depth: Vec<u32>,
    local_t: Vec<Vec3>,
    local_r: Vec<Quat>,
    local_s: Vec<Vec3>,
    world: Vec<Mat4>,
    rest_world: Vec<Mat4>,
}

impl Skeleton {
    /// Valida (índices, ciclos, números finitos) e constrói com a pose de repouso.
    pub fn new(mut nodes: Vec<SkeletonNode>) -> PhysicsResult<Self> {
        let n = nodes.len();
        for (i, node) in nodes.iter_mut().enumerate() {
            if let Some(p) = node.parent {
                if p >= n || p == i {
                    return Err(PhysicsError::BadHierarchy(format!("nó {i}: pai {p} inválido")));
                }
            }
            if !(node.translation.is_finite() && node.rotation.is_finite() && node.scale.is_finite()) {
                return Err(PhysicsError::BadValue { what: format!("nó {i} TRS") });
            }
            if node.rotation.length_squared() < 1e-12 {
                return Err(PhysicsError::BadValue { what: format!("nó {i} rotação nula") });
            }
            node.rotation = node.rotation.normalize();
        }
        let mut children = vec![Vec::new(); n];
        for (i, node) in nodes.iter().enumerate() {
            if let Some(p) = node.parent {
                children[p].push(i);
            }
        }
        // Ordem topológica (BFS a partir das raízes) + detecção de ciclos.
        let mut order = Vec::with_capacity(n);
        let mut depth = vec![0u32; n];
        let mut queue: std::collections::VecDeque<usize> =
            nodes.iter().enumerate().filter(|(_, nd)| nd.parent.is_none()).map(|(i, _)| i).collect();
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &c in &children[i] {
                depth[c] = depth[i] + 1;
                queue.push_back(c);
            }
        }
        if order.len() != n {
            return Err(PhysicsError::BadHierarchy("ciclo na hierarquia de nós".into()));
        }
        let local_t = nodes.iter().map(|nd| nd.translation).collect();
        let local_r = nodes.iter().map(|nd| nd.rotation).collect();
        let local_s = nodes.iter().map(|nd| nd.scale).collect();
        let mut sk = Self {
            nodes,
            children,
            order,
            depth,
            local_t,
            local_r,
            local_s,
            world: vec![Mat4::IDENTITY; n],
            rest_world: vec![Mat4::IDENTITY; n],
        };
        sk.update_world();
        sk.rest_world = sk.world.clone();
        Ok(sk)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[SkeletonNode] {
        &self.nodes
    }

    pub fn parent(&self, i: usize) -> Option<usize> {
        self.nodes.get(i).and_then(|n| n.parent)
    }

    pub fn depth(&self, i: usize) -> u32 {
        self.depth.get(i).copied().unwrap_or(0)
    }

    /// `true` se `ancestor` é ancestral estrito de `node`.
    pub fn is_ancestor(&self, ancestor: usize, node: usize) -> bool {
        let mut cursor = self.parent(node);
        while let Some(p) = cursor {
            if p == ancestor {
                return true;
            }
            cursor = self.parent(p);
        }
        false
    }

    pub fn rest_world(&self, i: usize) -> Mat4 {
        self.rest_world.get(i).copied().unwrap_or(Mat4::IDENTITY)
    }

    pub fn rest_local_matrix(&self, i: usize) -> Mat4 {
        match self.nodes.get(i) {
            Some(n) => Mat4::from_scale_rotation_translation(n.scale, n.rotation, n.translation),
            None => Mat4::IDENTITY,
        }
    }

    pub fn rest_local_rotation(&self, i: usize) -> Quat {
        self.nodes.get(i).map(|n| n.rotation).unwrap_or(Quat::IDENTITY)
    }

    pub fn local_rotation(&self, i: usize) -> Quat {
        self.local_r.get(i).copied().unwrap_or(Quat::IDENTITY)
    }

    pub fn local_translation(&self, i: usize) -> Vec3 {
        self.local_t.get(i).copied().unwrap_or(Vec3::ZERO)
    }

    pub fn world(&self, i: usize) -> Mat4 {
        self.world.get(i).copied().unwrap_or(Mat4::IDENTITY)
    }

    pub fn world_matrices(&self) -> &[Mat4] {
        &self.world
    }

    pub fn world_position(&self, i: usize) -> Vec3 {
        self.world(i).w_axis.truncate()
    }

    /// Define TRS local (valores não finitos são ignorados). Devolve `false` se rejeitou.
    pub fn set_local(&mut self, i: usize, translation: Vec3, rotation: Quat) -> bool {
        if i >= self.nodes.len() || !translation.is_finite() || !rotation.is_finite() || rotation.length_squared() < 1e-12 {
            return false;
        }
        self.local_t[i] = translation;
        self.local_r[i] = rotation.normalize();
        true
    }

    pub fn set_local_rotation(&mut self, i: usize, rotation: Quat) -> bool {
        if i >= self.nodes.len() || !rotation.is_finite() || rotation.length_squared() < 1e-12 {
            return false;
        }
        self.local_r[i] = rotation.normalize();
        true
    }

    pub fn set_local_translation(&mut self, i: usize, translation: Vec3) -> bool {
        if i >= self.nodes.len() || !translation.is_finite() {
            return false;
        }
        self.local_t[i] = translation;
        true
    }

    /// Volta toda a pose local para o repouso.
    pub fn reset_pose(&mut self) {
        for (i, n) in self.nodes.iter().enumerate() {
            self.local_t[i] = n.translation;
            self.local_r[i] = n.rotation;
            self.local_s[i] = n.scale;
        }
    }

    fn local_matrix(&self, i: usize) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.local_s[i], self.local_r[i], self.local_t[i])
    }

    /// FK completa.
    pub fn update_world(&mut self) {
        for k in 0..self.order.len() {
            let i = self.order[k];
            let local = self.local_matrix(i);
            self.world[i] = match self.nodes[i].parent {
                Some(p) => self.world[p] * local,
                None => local,
            };
        }
    }

    /// FK do nó `i` e de todos os descendentes (o pai deve estar atualizado).
    pub fn update_subtree(&mut self, i: usize) {
        if i >= self.nodes.len() {
            return;
        }
        let mut stack = vec![i];
        while let Some(n) = stack.pop() {
            let local = self.local_matrix(n);
            self.world[n] = match self.nodes[n].parent {
                Some(p) => self.world[p] * local,
                None => local,
            };
            stack.extend(self.children[n].iter().copied());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<SkeletonNode> {
        vec![
            SkeletonNode::new("root", None, Vec3::ZERO),
            SkeletonNode::new("a", Some(0), Vec3::new(0.0, 1.0, 0.0)),
            SkeletonNode::new("b", Some(1), Vec3::new(0.0, 1.0, 0.0)),
        ]
    }

    #[test]
    fn fk_acumula_translacao_e_rotacao() {
        let mut sk = Skeleton::new(chain()).unwrap();
        assert!((sk.world_position(2) - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-6);
        sk.set_local_rotation(0, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        sk.update_world();
        assert!((sk.world_position(2) - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-5);
        // repouso preservado
        assert!((sk.rest_world(2).w_axis.truncate() - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn subarvore_equivale_a_fk_completa() {
        let mut a = Skeleton::new(chain()).unwrap();
        let mut b = a.clone();
        a.set_local_rotation(1, Quat::from_rotation_x(0.7));
        a.update_world();
        b.set_local_rotation(1, Quat::from_rotation_x(0.7));
        b.update_subtree(1);
        for i in 0..3 {
            assert!((a.world(i) - b.world(i)).abs_diff_eq(Mat4::ZERO, 1e-6));
        }
    }

    #[test]
    fn hierarquia_invalida_e_rejeitada() {
        let mut nodes = chain();
        nodes[0].parent = Some(2); // ciclo 0 -> 2 -> 1 -> 0
        assert!(matches!(Skeleton::new(nodes), Err(PhysicsError::BadHierarchy(_))));
        let mut nodes = chain();
        nodes[1].parent = Some(9);
        assert!(Skeleton::new(nodes).is_err());
        let mut nodes = chain();
        nodes[1].parent = Some(1);
        assert!(Skeleton::new(nodes).is_err());
        let mut nodes = chain();
        nodes[1].translation = Vec3::splat(f32::NAN);
        assert!(Skeleton::new(nodes).is_err());
        assert!(Skeleton::new(vec![]).unwrap().is_empty());
    }

    #[test]
    fn setters_rejeitam_nan() {
        let mut sk = Skeleton::new(chain()).unwrap();
        assert!(!sk.set_local_rotation(1, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)));
        assert!(!sk.set_local_rotation(1, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)));
        assert!(!sk.set_local_translation(9, Vec3::ZERO));
        assert!(!sk.set_local(1, Vec3::splat(f32::INFINITY), Quat::IDENTITY));
        sk.update_world();
        assert!(sk.world(2).is_finite());
    }

    #[test]
    fn ancestralidade() {
        let sk = Skeleton::new(chain()).unwrap();
        assert!(sk.is_ancestor(0, 2) && sk.is_ancestor(1, 2));
        assert!(!sk.is_ancestor(2, 0) && !sk.is_ancestor(1, 1));
        assert_eq!(sk.depth(2), 2);
    }
}
