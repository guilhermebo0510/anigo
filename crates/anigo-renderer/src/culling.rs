//! Frustum culling (Fase 1, issue #13): nada é submetido ao `RenderPass` sem
//! antes passar por um teste de volume envolvente.
//!
//! O problema que isto resolve é simples de medir: numa cena com roupa em
//! camadas, cabelo e cenário, a maior parte dos nós está fora do cone de visão
//! (atrás da câmera, acima do topo, fora da lateral). Sem filtro, cada um deles
//! consome vértice, rasterização e banda — e o custo cresce com a cena.
//!
//! O desenho é deliberadamente conservador:
//!
//! * o volume local nasce da malha (`Mesh::local_bounds`), é **transformado**
//!   pela matriz mundial do nó (a mesma que o renderer usa como `model`) e o
//!   teste é feito em espaço de mundo;
//! * esfera e caixa são testadas contra 6 planos normalizados extraídos da
//!   `view_proj` — rejeita-se só quando o volume está **inteiramente** fora de
//!   **algum** plano;
//! * um volume degenerado ou uma matriz não finita **não** rejeita nada: o pior
//!   caso possível é desenhar demais, nunca apagar geometria visível.
//!
//! A telemetria (`RenderMetrics`) publica `draw_calls`/`culled_draw_calls`, o
//! que torna o efeito verificável em teste e em tela.

use anigo_core::math::{Aabb, BoundingSphere, Frustum};

/// Volume envolvente de um nó, já em espaço de mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneVolume {
    /// Caixa alinhada aos eixos do mundo (o nó pode ter rotação/escala).
    pub aabb: Aabb,
    /// Esfera que contém a caixa — o teste rápido.
    pub sphere: BoundingSphere,
}

impl SceneVolume {
    /// Volume de uma malha local transformado pela matriz mundial do nó.
    pub fn from_local(local: &Aabb, world_matrix: &glam::Mat4) -> Self {
        let aabb = local.transformed(world_matrix);
        Self {
            aabb,
            sphere: aabb.bounding_sphere(),
        }
    }

    /// Esfera envolvente transformada direto (útil quando só a esfera é
    /// conhecida, p.ex. o volume de uma cadeia de ossos).
    pub fn from_sphere(local: &BoundingSphere, world_matrix: &glam::Mat4) -> Self {
        let sphere = local.transformed(world_matrix);
        let half = sphere.radius / 3.0_f32.sqrt();
        let aabb = Aabb {
            min: sphere.center - glam::Vec3::splat(half),
            max: sphere.center + glam::Vec3::splat(half),
        };
        Self { aabb, sphere }
    }

    /// Teste do nó contra o frustum: `true` = desenhar.
    ///
    /// Esfera primeiro (mais barato, e um "dentro" já encerra); a caixa só é
    /// consultada quando a esfera é interceptada — o caso em que o volume
    /// encosta no plano e a decisão precisa ser mais fina.
    pub fn is_visible(&self, frustum: &Frustum) -> bool {
        // Um volume não finito (matriz corrompida, malha com NaN) é tratado como
        // visível: o pior caso possível é desenhar demais, nunca apagar
        // geometria que deveria estar na tela.
        if !self.sphere.center.is_finite()
            || !self.sphere.radius.is_finite()
            || !self.aabb.min.is_finite()
            || !self.aabb.max.is_finite()
        {
            return true;
        }
        if !frustum.intersects_sphere(&self.sphere) {
            return false;
        }
        frustum.intersects_aabb(&self.aabb)
    }
}

/// Resultado de um quadro de filtragem: o que desenhar e o que foi economizado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CullReport {
    /// Nós que passaram no teste.
    pub drawn: usize,
    /// Nós recusados pelo frustum.
    pub culled: usize,
}

impl CullReport {
    /// Total de nós considerados.
    pub fn considered(&self) -> usize {
        self.drawn + self.culled
    }

    /// `true` quando o filtro não economizou nenhuma draw call.
    pub fn is_noop(&self) -> bool {
        self.culled == 0
    }
}

/// Filtra uma lista de volumes, devolvendo os visíveis e o relatório.
///
/// A ordem de entrada é preservada — culling não pode mexer na ordem de
/// desenho determinística que o contrato do renderer exige.
pub fn cull_volumes(volumes: &[SceneVolume], frustum: &Frustum) -> (Vec<usize>, CullReport) {
    let mut visible = Vec::with_capacity(volumes.len());
    let mut report = CullReport::default();
    for (index, volume) in volumes.iter().enumerate() {
        if volume.is_visible(frustum) {
            visible.push(index);
            report.drawn += 1;
        } else {
            report.culled += 1;
        }
    }
    (visible, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::math::Camera;
    use glam::{Mat4, Vec3};

    fn unit_cube() -> Aabb {
        Aabb {
            min: Vec3::splat(-0.5),
            max: Vec3::splat(0.5),
        }
    }

    fn camera() -> Camera {
        // Mesma câmera canônica do contrato: olha o manequim da frente.
        Camera::default()
    }

    #[test]
    fn object_far_behind_the_camera_is_culled() {
        // Critério de aceitação #3 do issue: algo em z = -100 (atrás da câmera,
        // que está em z = +3.5 olhando para a origem) não entra no desenho.
        let frustum = camera().frustum();
        let behind = SceneVolume::from_local(&unit_cube(), &Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0)));
        assert!(!behind.is_visible(&frustum), "objeto atrás da câmera precisa ser recusado");

        let (visible, report) = cull_volumes(&[behind], &frustum);
        assert!(visible.is_empty());
        assert_eq!(report.culled, 1);
        assert_eq!(report.considered(), 1);
        assert!(!report.is_noop());
    }

    #[test]
    fn object_in_front_of_the_camera_is_drawn() {
        let frustum = camera().frustum();
        let in_front = SceneVolume::from_local(&unit_cube(), &Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)));
        assert!(in_front.is_visible(&frustum));
        let (visible, report) = cull_volumes(&[in_front], &frustum);
        assert_eq!(visible, vec![0]);
        assert!(report.is_noop());
    }

    #[test]
    fn side_objects_are_culled_but_the_boundary_is_respected() {
        let frustum = camera().frustum();
        // Muito à direita: fora do cone horizontal.
        let far_right = SceneVolume::from_local(&unit_cube(), &Mat4::from_translation(Vec3::new(60.0, 1.0, 0.0)));
        assert!(!far_right.is_visible(&frustum));

        // Exatamente sobre o plano direito (na altura/alvo do meio): o teste
        // conservador **precisa** aceitar — recusar aqui cortaria geometria
        // encostada na borda da tela.
        let plane = frustum.plane(anigo_core::math::FrustumPlane::Right);
        let on_plane_x = -(plane.y * 1.0 + plane.w) / plane.x;
        let borderline = SceneVolume::from_local(
            &unit_cube(),
            &Mat4::from_translation(Vec3::new(on_plane_x, 1.0, 0.0)),
        );
        assert!(
            borderline.is_visible(&frustum),
            "volume sobre o plano (x = {on_plane_x}) precisa continuar desenhando"
        );

        // Um pouco além já é recusado (o teste não é frouxo).
        let past_plane = SceneVolume::from_local(
            &unit_cube(),
            &Mat4::from_translation(Vec3::new(on_plane_x + 2.0, 1.0, 0.0)),
        );
        assert!(!past_plane.is_visible(&frustum));
    }

    #[test]
    fn a_node_rotation_keeps_the_volume_conservative() {
        let frustum = camera().frustum();
        // Um cubo de 4 unidades girado 45°: a caixa alinhada cresce, e o teste
        // continua aceitando o objeto (nunca recorta geometria visível).
        let big = Aabb {
            min: Vec3::splat(-2.0),
            max: Vec3::splat(2.0),
        };
        let world = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4)
            * Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let volume = SceneVolume::from_local(&big, &world);
        assert!(volume.sphere.radius > 3.0);
        assert!(volume.is_visible(&frustum));
    }

    #[test]
    fn degenerate_or_non_finite_matrices_never_reject() {
        let frustum = camera().frustum();
        let broken = Mat4::from_scale(Vec3::new(f32::NAN, 1.0, 1.0));
        let volume = SceneVolume::from_local(&unit_cube(), &broken);
        // A decisão segura é desenhar: um falso negativo apagaria o personagem.
        assert!(
            volume.is_visible(&frustum),
            "matriz inválida não pode virar sumiço de geometria"
        );

        // O mesmo vale quando o volume em si já vem corrompido.
        let poisoned = SceneVolume {
            aabb: Aabb {
                min: Vec3::new(f32::INFINITY, 0.0, 0.0),
                max: Vec3::new(f32::NAN, 1.0, 1.0),
            },
            sphere: BoundingSphere::new(Vec3::new(f32::NAN, 0.0, 0.0), f32::NAN),
        };
        assert!(poisoned.is_visible(&frustum));
        let (visible, report) = cull_volumes(&[poisoned], &frustum);
        assert_eq!(visible, vec![0]);
        assert_eq!(report.culled, 0);
    }

    #[test]
    fn orthographic_camera_culls_by_the_same_rule() {
        // A câmera ortográfica tem o mesmo frustum (só muda a projeção), então o
        // filtro vale igual para os dois modos.
        let mut ortho = camera();
        ortho.set_projection_mode(anigo_core::math::OrthographicBounds::from_height(2.0, 16.0 / 9.0).into());
        let frustum = ortho.frustum();
        let inside = SceneVolume::from_local(&unit_cube(), &Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)));
        let behind = SceneVolume::from_local(&unit_cube(), &Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0)));
        assert!(inside.is_visible(&frustum));
        assert!(!behind.is_visible(&frustum));
    }
}
