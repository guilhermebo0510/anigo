//! Colliders: esfera, cápsula, plano e os "inside" (esfera/cápsula por dentro) de
//! `VRMC_springBone_extended_collider`. Formas são definidas no espaço local do osso a que estão
//! presas e transformadas para o mundo a cada quadro.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::math::{safe_normalize, uniform_scale_of};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColliderShape {
    Sphere { center: Vec3, radius: f32 },
    Capsule { a: Vec3, b: Vec3, radius: f32 },
    /// Mantém a partícula DENTRO da esfera.
    InsideSphere { center: Vec3, radius: f32 },
    /// Mantém a partícula DENTRO da cápsula.
    InsideCapsule { a: Vec3, b: Vec3, radius: f32 },
    /// Semi-espaço: a partícula fica do lado de `normal` (unitária).
    Plane { point: Vec3, normal: Vec3 },
}

impl ColliderShape {
    /// `true` quando todos os números são finitos e raios >= 0.
    pub fn is_valid(&self) -> bool {
        match *self {
            ColliderShape::Sphere { center, radius } | ColliderShape::InsideSphere { center, radius } => {
                center.is_finite() && radius.is_finite() && radius >= 0.0
            }
            ColliderShape::Capsule { a, b, radius } | ColliderShape::InsideCapsule { a, b, radius } => {
                a.is_finite() && b.is_finite() && radius.is_finite() && radius >= 0.0
            }
            ColliderShape::Plane { point, normal } => {
                point.is_finite() && normal.is_finite() && normal.length_squared() > 1e-12
            }
        }
    }

    /// Forma no mundo, dado o transform do osso (rígido; escala uniforme aproximada pelo maior eixo).
    pub fn transformed(&self, m: &Mat4) -> ColliderShape {
        let s = uniform_scale_of(m);
        match *self {
            ColliderShape::Sphere { center, radius } => {
                ColliderShape::Sphere { center: m.transform_point3(center), radius: radius * s }
            }
            ColliderShape::InsideSphere { center, radius } => {
                ColliderShape::InsideSphere { center: m.transform_point3(center), radius: radius * s }
            }
            ColliderShape::Capsule { a, b, radius } => ColliderShape::Capsule {
                a: m.transform_point3(a),
                b: m.transform_point3(b),
                radius: radius * s,
            },
            ColliderShape::InsideCapsule { a, b, radius } => ColliderShape::InsideCapsule {
                a: m.transform_point3(a),
                b: m.transform_point3(b),
                radius: radius * s,
            },
            ColliderShape::Plane { point, normal } => ColliderShape::Plane {
                point: m.transform_point3(point),
                normal: safe_normalize(m.transform_vector3(normal), Vec3::Y),
            },
        }
    }

    /// Folga assinada entre um ponto de raio `margin` e a forma (já no mundo) e a direção em que
    /// empurrar para sair. `folga < 0` = penetrando; o empurrão é `ponto -= direcao * folga`.
    /// Fórmulas da spec `VRMC_springBone` (+ extended collider).
    pub fn gap(&self, p: Vec3, margin: f32) -> (f32, Vec3) {
        match *self {
            ColliderShape::Sphere { center, radius } => {
                let delta = p - center;
                (delta.length() - radius - margin, safe_normalize(delta, Vec3::Y))
            }
            ColliderShape::InsideSphere { center, radius } => {
                let delta = p - center;
                (radius - margin - delta.length(), -safe_normalize(delta, Vec3::Y))
            }
            ColliderShape::Capsule { a, b, radius } => {
                let delta = capsule_delta(p, a, b);
                (delta.length() - radius - margin, safe_normalize(delta, Vec3::Y))
            }
            ColliderShape::InsideCapsule { a, b, radius } => {
                let delta = capsule_delta(p, a, b);
                (radius - margin - delta.length(), -safe_normalize(delta, Vec3::Y))
            }
            ColliderShape::Plane { point, normal } => {
                let n = safe_normalize(normal, Vec3::Y);
                ((p - point).dot(n) - margin, n)
            }
        }
    }
}

/// Vetor do ponto mais próximo do eixo (segmento) da cápsula até `p` (pseudocódigo da spec).
#[inline]
fn capsule_delta(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let mut delta = p - a;
    let dot = ab.dot(delta);
    let sq = ab.length_squared();
    if dot < 0.0 || !(sq > 1e-20) {
        // lado da cabeça: nada a subtrair
    } else if dot > sq {
        delta -= ab;
    } else {
        delta -= ab * (dot / sq);
    }
    delta
}

/// Collider preso a um osso (`bone`) ou fixo no mundo (`None`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collider {
    pub shape: ColliderShape,
    pub bone: Option<usize>,
    /// Atrito de Coulomb simplificado (0 = gelo, 1 = cola).
    pub friction: f32,
    pub enabled: bool,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere { center: Vec3::ZERO, radius: 0.0 },
            bone: None,
            friction: 0.2,
            enabled: true,
        }
    }
}

impl Collider {
    pub fn on_bone(bone: usize, shape: ColliderShape) -> Self {
        Self { shape, bone: Some(bone), ..Default::default() }
    }

    pub fn fixed(shape: ColliderShape) -> Self {
        Self { shape, bone: None, ..Default::default() }
    }
}

/// Caixa envolvente (sem margem) de uma forma no mundo; `None` para formas ilimitadas (plano, inside).
fn shape_aabb(shape: &ColliderShape) -> Option<(Vec3, Vec3)> {
    match *shape {
        ColliderShape::Sphere { center, radius } => Some((center - Vec3::splat(radius), center + Vec3::splat(radius))),
        ColliderShape::Capsule { a, b, radius } => Some((a.min(b) - Vec3::splat(radius), a.max(b) + Vec3::splat(radius))),
        _ => None,
    }
}

/// Conjunto de colliders com cache das formas no mundo (e caixas para descarte rápido).
#[derive(Debug, Clone, Default)]
pub struct ColliderSet {
    colliders: Vec<Collider>,
    world: Vec<ColliderShape>,
    aabb: Vec<Option<(Vec3, Vec3)>>,
}

impl ColliderSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adiciona um collider; devolve o índice, ou `None` se a forma é inválida (NaN, raio < 0).
    pub fn add(&mut self, collider: Collider) -> Option<usize> {
        if !collider.shape.is_valid() || !collider.friction.is_finite() {
            return None;
        }
        self.colliders.push(collider);
        self.world.push(collider.shape);
        self.aabb.push(shape_aabb(&collider.shape));
        Some(self.colliders.len() - 1)
    }

    pub fn len(&self) -> usize {
        self.colliders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colliders.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Collider> {
        self.colliders.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Collider> {
        self.colliders.get_mut(index)
    }

    pub fn colliders(&self) -> &[Collider] {
        &self.colliders
    }

    /// Formas no mundo (mesma ordem de `colliders`).
    pub fn world_shapes(&self) -> &[ColliderShape] {
        &self.world
    }

    /// Atualiza as formas no mundo a partir das matrizes dos ossos. Osso inexistente/NaN = mantém a
    /// última forma válida (o personagem não "perde" o collider por um quadro ruim).
    pub fn update(&mut self, bone_world: &[Mat4]) {
        for ((collider, world), aabb) in self.colliders.iter().zip(self.world.iter_mut()).zip(self.aabb.iter_mut()) {
            match collider.bone {
                None => *world = collider.shape,
                Some(b) => {
                    if let Some(m) = bone_world.get(b) {
                        if m.is_finite() {
                            let s = collider.shape.transformed(m);
                            if s.is_valid() {
                                *world = s;
                            }
                        }
                    }
                }
            }
            *aabb = shape_aabb(world);
        }
    }

    /// Resolve a penetração de uma partícula (raio `margin`) contra os colliders habilitados de
    /// `indices` (ou todos se `None`). Devolve a posição corrigida e se houve contato; também o
    /// maior atrito dos contatos.
    pub fn push_out(&self, mut p: Vec3, margin: f32, indices: Option<&[usize]>) -> (Vec3, bool, f32) {
        let mut hit = false;
        let mut friction = 0.0f32;
        let mut apply = |i: usize, p: &mut Vec3| {
            if let (Some(c), Some(shape)) = (self.colliders.get(i), self.world.get(i)) {
                if !c.enabled {
                    return;
                }
                // descarte rápido pela caixa (a margem é o raio da partícula)
                if let Some(Some((lo, hi))) = self.aabb.get(i) {
                    if p.x < lo.x - margin || p.y < lo.y - margin || p.z < lo.z - margin || p.x > hi.x + margin || p.y > hi.y + margin || p.z > hi.z + margin {
                        return;
                    }
                }
                let (gap, dir) = shape.gap(*p, margin);
                if gap < 0.0 {
                    *p -= dir * gap;
                    hit = true;
                    friction = friction.max(c.friction);
                }
            }
        };
        match indices {
            Some(list) => list.iter().for_each(|&i| apply(i, &mut p)),
            None => (0..self.colliders.len()).for_each(|i| apply(i, &mut p)),
        }
        (p, hit, friction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esfera_distancia_e_direcao() {
        let s = ColliderShape::Sphere { center: Vec3::ZERO, radius: 1.0 };
        let (gap, dir) = s.gap(Vec3::new(3.0, 0.0, 0.0), 0.5);
        assert!((gap - 1.5).abs() < 1e-6);
        assert!((dir - Vec3::X).length() < 1e-6);
        let (gap, _) = s.gap(Vec3::new(1.2, 0.0, 0.0), 0.5);
        assert!(gap < 0.0, "ponto de raio 0.5 a 1.2 do centro penetra a esfera de raio 1");
    }

    #[test]
    fn capsula_regioes_cabeca_corpo_cauda() {
        let c = ColliderShape::Capsule { a: Vec3::ZERO, b: Vec3::new(0.0, 2.0, 0.0), radius: 0.5 };
        // lado do corpo: distancia ao eixo
        let (gap, dir) = c.gap(Vec3::new(1.5, 1.0, 0.0), 0.0);
        assert!((gap - 1.0).abs() < 1e-6 && (dir - Vec3::X).length() < 1e-6);
        // tampa da cabeca (y<0): distancia ao ponto a
        let (gap, dir) = c.gap(Vec3::new(0.0, -2.0, 0.0), 0.0);
        assert!((gap - 1.5).abs() < 1e-6 && (dir + Vec3::Y).length() < 1e-6);
        // tampa da cauda (y>2): distancia ao ponto b
        let (gap, dir) = c.gap(Vec3::new(0.0, 5.0, 0.0), 0.0);
        assert!((gap - 2.5).abs() < 1e-6 && (dir - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn capsula_degenerada_vira_esfera() {
        let c = ColliderShape::Capsule { a: Vec3::ONE, b: Vec3::ONE, radius: 1.0 };
        let (gap, _) = c.gap(Vec3::new(3.0, 1.0, 1.0), 0.0);
        assert!((gap - 1.0).abs() < 1e-6);
    }

    #[test]
    fn inside_mantem_dentro() {
        let s = ColliderShape::InsideSphere { center: Vec3::ZERO, radius: 1.0 };
        let (gap, dir) = s.gap(Vec3::new(0.5, 0.0, 0.0), 0.1);
        assert!((gap - 0.4).abs() < 1e-6);
        assert!((dir + Vec3::X).length() < 1e-6, "empurra para o centro");
        let (gap, dir) = s.gap(Vec3::new(1.5, 0.0, 0.0), 0.0);
        assert!(gap < 0.0);
        let mut p = Vec3::new(1.5, 0.0, 0.0);
        p -= dir * gap;
        assert!((p.x - 1.0).abs() < 1e-6, "volta para a superficie interna");
    }

    #[test]
    fn plano_semi_espaco() {
        let p = ColliderShape::Plane { point: Vec3::new(0.0, 1.0, 0.0), normal: Vec3::Y };
        let (gap, dir) = p.gap(Vec3::new(5.0, 3.0, 0.0), 0.5);
        assert!((gap - 1.5).abs() < 1e-6 && dir == Vec3::Y);
        let (gap, _) = p.gap(Vec3::new(0.0, 0.0, 0.0), 0.0);
        assert!((gap + 1.0).abs() < 1e-6);
    }

    #[test]
    fn transformacao_move_e_escala() {
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let s = ColliderShape::Sphere { center: Vec3::new(1.0, 0.0, 0.0), radius: 0.5 }.transformed(&m);
        match s {
            ColliderShape::Sphere { center, radius } => {
                assert!((center - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
                assert!((radius - 1.0).abs() < 1e-6);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn conjunto_rejeita_formas_invalidas_e_empurra() {
        let mut set = ColliderSet::new();
        assert!(set.add(Collider::fixed(ColliderShape::Sphere { center: Vec3::splat(f32::NAN), radius: 1.0 })).is_none());
        assert!(set.add(Collider::fixed(ColliderShape::Sphere { center: Vec3::ZERO, radius: -1.0 })).is_none());
        assert!(set.add(Collider::fixed(ColliderShape::Plane { point: Vec3::ZERO, normal: Vec3::ZERO })).is_none());
        let id = set.add(Collider::fixed(ColliderShape::Sphere { center: Vec3::ZERO, radius: 1.0 })).unwrap();
        let (p, hit, _) = set.push_out(Vec3::new(0.5, 0.0, 0.0), 0.0, None);
        assert!(hit && (p.x - 1.0).abs() < 1e-6);
        set.get_mut(id).unwrap().enabled = false;
        let (p2, hit2, _) = set.push_out(Vec3::new(0.5, 0.0, 0.0), 0.0, None);
        assert!(!hit2 && p2.x == 0.5);
    }

    #[test]
    fn update_ignora_osso_invalido() {
        let mut set = ColliderSet::new();
        set.add(Collider::on_bone(1, ColliderShape::Sphere { center: Vec3::ZERO, radius: 1.0 }));
        set.update(&[Mat4::IDENTITY, Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0))]);
        let before = set.world_shapes()[0];
        set.update(&[Mat4::IDENTITY, Mat4::from_translation(Vec3::splat(f32::NAN))]);
        assert_eq!(set.world_shapes()[0], before, "NaN mantem a ultima forma valida");
        set.update(&[Mat4::IDENTITY]);
        assert_eq!(set.world_shapes()[0], before, "osso fora da faixa mantem a forma");
    }
}
