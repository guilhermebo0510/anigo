//! ANIGO Procedural Spline Hair Generation Engine.
//!
//! Camada de AUTORIA do cabelo (fios/curvas); a simulação mora em `anigo-physics` e é reexportada
//! aqui. O conversor [`chain_from_curve`] transforma uma `anigo_modeling::curve::Curve` (a mesma
//! que alimenta `hair_card`, `curve_to_ribbon` e `hair_clump`) em uma cadeia neutra
//! ([`ChainGuide`]) pronta para simular, sem dependência cíclica: `anigo-physics` não conhece
//! `anigo-modeling`.

use anigo_modeling::curve::{Curve, CurveSample};
use anigo_modeling::{ModelError, ModelResult};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

pub use anigo_physics::{
    apply_chain_bindings, bind_to_chains, compliance_from_stiffness, demo_strands, ChainBinding, ChainFrames, ChainGuide,
    HairChainDef, HairGroupDef, HairParams, HairSim, PhysicsPreset,
};

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

    /// Fio -> cadeia de simulação (um ponto por ponto de controle; `width/2` vira o raio de colisão).
    /// `None` se há menos de 2 pontos ou algum não finito.
    pub fn to_chain_guide(&self) -> Option<ChainGuide> {
        if self.control_points.len() < 2 || self.control_points.iter().any(|p| !p.position.is_finite() || !p.width.is_finite()) {
            return None;
        }
        Some(ChainGuide {
            points: self.control_points.iter().map(|p| p.position).collect(),
            radii: self.control_points.iter().map(|p| (p.width * 0.5).max(0.0)).collect(),
        })
    }
}

/// Curva -> cadeia: `points` amostras uniformes em comprimento de arco (>= 2), levadas ao espaço do
/// osso-raiz por `to_bone_local` (ex.: `head_world.inverse()` se a curva foi desenhada no mundo).
/// O raio de colisão de cada ponto é `radius_scale * curve.radius` (multiplicador de largura do ponto).
pub fn chain_from_curve(curve: &Curve, points: usize, to_bone_local: Mat4, radius_scale: f32) -> ModelResult<ChainGuide> {
    if !to_bone_local.is_finite() || !radius_scale.is_finite() || radius_scale < 0.0 {
        return Err(ModelError::InvalidParameter("transformação ou radius_scale inválido".into()));
    }
    let samples: Vec<CurveSample> = curve.sample_uniform(points)?;
    Ok(ChainGuide {
        points: samples.iter().map(|s| to_bone_local.transform_point3(s.pos)).collect(),
        radii: samples.iter().map(|s| (s.radius * radius_scale).max(0.0)).collect(),
    })
}

/// Várias curvas -> um grupo de cabelo simulável, todas presas ao mesmo osso.
pub fn hair_group_from_curves(
    name: &str,
    curves: &[Curve],
    points_per_chain: usize,
    root_bone: Option<usize>,
    to_bone_local: Mat4,
    params: HairParams,
) -> ModelResult<HairGroupDef> {
    let mut chains = Vec::with_capacity(curves.len());
    for curve in curves {
        chains.push(HairChainDef { guide: chain_from_curve(curve, points_per_chain, to_bone_local, 0.01)?, root_bone });
    }
    Ok(HairGroupDef { name: name.to_string(), chains, params, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::curve::{CurveKind, CurvePoint};
    use anigo_physics::{ColliderSet, PhysicsWorld};
    use glam::Quat;

    fn drooping_curve() -> Curve {
        Curve::from_positions(
            CurveKind::CatmullRom,
            &[Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.05, 1.5, 0.0), Vec3::new(0.08, 1.35, 0.0), Vec3::new(0.08, 1.2, 0.0)],
            false,
        )
    }

    #[test]
    fn curva_vira_cadeia_com_o_mesmo_comprimento() {
        let curve = drooping_curve();
        let guide = chain_from_curve(&curve, 10, Mat4::IDENTITY, 0.01).unwrap();
        assert_eq!(guide.points.len(), 10);
        assert_eq!(guide.radii.len(), 10);
        let rel = (guide.length() - curve.length()).abs() / curve.length();
        assert!(rel < 0.03, "comprimento da cadeia vs curva: {rel}");
        assert!((guide.points[0] - Vec3::new(0.0, 1.6, 0.0)).length() < 1e-5, "raiz = primeiro ponto");
        assert!((guide.points[9] - Vec3::new(0.08, 1.2, 0.0)).length() < 1e-4, "ponta = ultimo ponto");
    }

    #[test]
    fn transformacao_para_o_espaco_do_osso() {
        let head = Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(0.0, 1.5, 0.0));
        let curve = drooping_curve();
        let guide = chain_from_curve(&curve, 6, head.inverse(), 0.01).unwrap();
        // voltando para o mundo recupera a curva
        let world = chain_from_curve(&curve, 6, Mat4::IDENTITY, 0.01).unwrap();
        for (l, w) in guide.points.iter().zip(&world.points) {
            assert!((head.transform_point3(*l) - *w).length() < 1e-5);
        }
    }

    #[test]
    fn entradas_invalidas_viram_erro() {
        let curve = drooping_curve();
        assert!(chain_from_curve(&curve, 1, Mat4::IDENTITY, 0.01).is_err());
        assert!(chain_from_curve(&curve, 5, Mat4::from_translation(Vec3::splat(f32::NAN)), 0.01).is_err());
        assert!(chain_from_curve(&curve, 5, Mat4::IDENTITY, f32::NAN).is_err());
        let degenerate = Curve::new(CurveKind::Polyline, vec![CurvePoint::new(Vec3::ZERO), CurvePoint::new(Vec3::ZERO)], false);
        assert!(chain_from_curve(&degenerate, 4, Mat4::IDENTITY, 0.01).is_err());
        let mut strand = HairStrand::new("a");
        assert!(strand.to_chain_guide().is_none());
        strand.control_points.push(HairControlPoint { position: Vec3::ZERO, width: 0.02 });
        strand.control_points.push(HairControlPoint { position: Vec3::new(0.0, -0.1, 0.0), width: f32::NAN });
        assert!(strand.to_chain_guide().is_none());
    }

    #[test]
    fn curvas_do_hair_clump_simulam_no_physics_world() {
        use anigo_modeling::curve::{hair_clump, ClumpParams};
        let guide = drooping_curve();
        let strands = hair_clump(&guide, &ClumpParams { count: 6, points: 8, ..Default::default() }).unwrap();
        let head = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0));
        let def = hair_group_from_curves("clump", &strands, 8, Some(0), head.inverse(), PhysicsPreset::HairLong.hair_params().unwrap()).unwrap();
        assert_eq!(def.chains.len(), 6);
        let mut world = PhysicsWorld::default();
        let handle = world.add_hair(&def, &[head]).unwrap();
        for f in 0..180 {
            let t = f as f32 / 60.0;
            let pose = Mat4::from_translation(Vec3::new((t * 3.0).sin() * 0.2, 1.5, 0.0));
            world.step(1.0 / 60.0, &[pose]);
        }
        let hair = world.hair(handle).unwrap();
        assert!(hair.positions().iter().all(|p| p.is_finite()));
        assert!(hair.max_stretch_ratio() < 0.1);
        let _ = ColliderSet::new();
    }

    #[test]
    fn card_de_hair_card_acompanha_a_cadeia() {
        use anigo_modeling::curve::{hair_card, HairCardParams};
        let curve = drooping_curve();
        let mesh = hair_card(&curve, &HairCardParams { width: 0.04, segments: 12, ..Default::default() }).unwrap();
        let verts: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
        let guide = chain_from_curve(&curve, 8, Mat4::IDENTITY, 0.01).unwrap();
        let reference = Vec3::Z;
        let bindings = bind_to_chains(&verts, &[&guide.points], &[reference]).unwrap();
        // repouso: a malha de render é reproduzida (erro limitado pela discretização da cadeia)
        let rest: Vec<Vec3> = apply_chain_bindings(&bindings, &[&guide.points], &[reference]).into_iter().map(|p| p.unwrap()).collect();
        let err = verts.iter().zip(&rest).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        assert!(err < 1e-4, "{err}");
    }
}
