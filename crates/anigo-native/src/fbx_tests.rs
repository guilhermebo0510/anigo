//! Testes do import FBX. Os arquivos são gerados em memória como FBX ASCII mínimo
//! (sem binários no repositório); o ufbx lê o formato ASCII exatamente como o binário.

use super::*;

/// Gerador de FBX ASCII mínimo.
struct Fixture {
    /// 1 = Y-up, 2 = Z-up (convenção `UpAxis` do FBX).
    up_axis: i32,
    /// `UnitScaleFactor` do FBX: 1 = centímetros, 100 = metros.
    unit_scale: f64,
    verts: Vec<[f64; 3]>,
    polys: Vec<Vec<usize>>,
    /// Normal por corner, na ordem de `polys` achatada.
    normals: Vec<[f64; 3]>,
    uv_table: Vec<[f64; 2]>,
    /// Índice em `uv_table` por corner.
    uv_index: Vec<usize>,
    skin: bool,
    blend: bool,
    anim: bool,
}

impl Fixture {
    /// Caixa `w × d × h` (X, Y, Z do arquivo) com quads CCW, normais e UVs por corner.
    fn boxed(w: f64, d: f64, h: f64) -> Self {
        let verts = vec![
            [0.0, 0.0, 0.0],
            [w, 0.0, 0.0],
            [w, d, 0.0],
            [0.0, d, 0.0],
            [0.0, 0.0, h],
            [w, 0.0, h],
            [w, d, h],
            [0.0, d, h],
        ];
        let polys = vec![
            vec![0, 3, 2, 1], // -Z
            vec![4, 5, 6, 7], // +Z
            vec![0, 1, 5, 4], // -Y
            vec![3, 7, 6, 2], // +Y
            vec![0, 4, 7, 3], // -X
            vec![1, 2, 6, 5], // +X
        ];
        let face_normals = [
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let mut normals = Vec::new();
        let mut uv_index = Vec::new();
        for n in face_normals {
            for k in 0..4 {
                normals.push(n);
                uv_index.push(k);
            }
        }
        Self {
            up_axis: 1,
            unit_scale: 100.0,
            verts,
            polys,
            normals,
            uv_table: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            uv_index,
            skin: false,
            blend: false,
            anim: false,
        }
    }

    fn nums(v: &[f64]) -> String {
        v.iter()
            .map(|x| format!("{x}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn to_ascii(&self) -> String {
        let (up, front) = if self.up_axis == 2 { (2, 1) } else { (1, 2) };
        let flat_verts: Vec<f64> = self.verts.iter().flatten().copied().collect();
        let mut pvi: Vec<i64> = Vec::new();
        for p in &self.polys {
            for (k, &v) in p.iter().enumerate() {
                pvi.push(if k + 1 == p.len() {
                    -(v as i64) - 1
                } else {
                    v as i64
                });
            }
        }
        let pvi_s = pvi
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let flat_normals: Vec<f64> = self.normals.iter().flatten().copied().collect();
        let flat_uv: Vec<f64> = self.uv_table.iter().flatten().copied().collect();
        let uvi = self
            .uv_index
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut s = String::new();
        s.push_str("; FBX 7.4.0 project file\n");
        s.push_str("FBXHeaderExtension:  {\n\tFBXHeaderVersion: 1003\n\tFBXVersion: 7400\n\tCreator: \"ANIGO test fixture\"\n}\n");
        s.push_str(&format!(
            "GlobalSettings:  {{\n\tVersion: 1000\n\tProperties70:  {{\n\
             \t\tP: \"UpAxis\", \"int\", \"Integer\", \"\",{up}\n\
             \t\tP: \"UpAxisSign\", \"int\", \"Integer\", \"\",1\n\
             \t\tP: \"FrontAxis\", \"int\", \"Integer\", \"\",{front}\n\
             \t\tP: \"FrontAxisSign\", \"int\", \"Integer\", \"\",{}\n\
             \t\tP: \"CoordAxis\", \"int\", \"Integer\", \"\",0\n\
             \t\tP: \"CoordAxisSign\", \"int\", \"Integer\", \"\",1\n\
             \t\tP: \"OriginalUpAxis\", \"int\", \"Integer\", \"\",{up}\n\
             \t\tP: \"OriginalUpAxisSign\", \"int\", \"Integer\", \"\",1\n\
             \t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",{}\n\
             \t\tP: \"OriginalUnitScaleFactor\", \"double\", \"Number\", \"\",{}\n\
             \t}}\n}}\n",
            if self.up_axis == 2 { -1 } else { 1 },
            self.unit_scale,
            self.unit_scale
        ));
        s.push_str("Definitions:  {\n\tVersion: 100\n\tCount: 1\n}\n");
        s.push_str("Objects:  {\n");
        s.push_str(&format!(
            "\tGeometry: 100, \"Geometry::Box\", \"Mesh\" {{\n\
             \t\tVertices: *{} {{\n\t\t\ta: {}\n\t\t}}\n\
             \t\tPolygonVertexIndex: *{} {{\n\t\t\ta: {}\n\t\t}}\n\
             \t\tGeometryVersion: 124\n\
             \t\tLayerElementNormal: 0 {{\n\t\t\tVersion: 101\n\t\t\tName: \"\"\n\
             \t\t\tMappingInformationType: \"ByPolygonVertex\"\n\
             \t\t\tReferenceInformationType: \"Direct\"\n\
             \t\t\tNormals: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\t\t}}\n\
             \t\tLayerElementUV: 0 {{\n\t\t\tVersion: 101\n\t\t\tName: \"UVMap\"\n\
             \t\t\tMappingInformationType: \"ByPolygonVertex\"\n\
             \t\t\tReferenceInformationType: \"IndexToDirect\"\n\
             \t\t\tUV: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\
             \t\t\tUVIndex: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\t\t}}\n\
             \t\tLayerElementMaterial: 0 {{\n\t\t\tVersion: 101\n\t\t\tName: \"\"\n\
             \t\t\tMappingInformationType: \"AllSame\"\n\
             \t\t\tReferenceInformationType: \"IndexToDirect\"\n\
             \t\t\tMaterials: *1 {{\n\t\t\t\ta: 0\n\t\t\t}}\n\t\t}}\n\
             \t\tLayer: 0 {{\n\t\t\tVersion: 100\n\
             \t\t\tLayerElement:  {{\n\t\t\t\tType: \"LayerElementNormal\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}\n\
             \t\t\tLayerElement:  {{\n\t\t\t\tType: \"LayerElementMaterial\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}\n\
             \t\t\tLayerElement:  {{\n\t\t\t\tType: \"LayerElementUV\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}\n\
             \t\t}}\n\t}}\n",
            flat_verts.len(),
            Self::nums(&flat_verts),
            pvi.len(),
            pvi_s,
            flat_normals.len(),
            Self::nums(&flat_normals),
            flat_uv.len(),
            Self::nums(&flat_uv),
            self.uv_index.len(),
            uvi
        ));
        s.push_str("\tModel: 200, \"Model::Box\", \"Mesh\" {\n\t\tVersion: 232\n\t\tProperties70:  {\n\t\t}\n\t}\n");
        s.push_str("\tMaterial: 300, \"Material::Red\", \"\" {\n\t\tVersion: 102\n\t\tShadingModel: \"lambert\"\n\t\tProperties70:  {\n\t\t\tP: \"DiffuseColor\", \"Color\", \"\", \"A\",1,0,0\n\t\t}\n\t}\n");

        let mut conns =
            String::from("\tC: \"OO\",200,0\n\tC: \"OO\",100,200\n\tC: \"OO\",300,200\n");

        if self.skin {
            let h = self.verts.iter().map(|v| v[2]).fold(0.0, f64::max);
            // dois ossos: A na origem, B a meia altura (filho de A)
            for (id, aid, name, t) in [(210, 215, "BoneA", 0.0), (211, 216, "BoneB", h * 0.5)] {
                s.push_str(&format!(
                    "\tNodeAttribute: {aid}, \"NodeAttribute::\", \"LimbNode\" {{\n\t\tTypeFlags: \"Skeleton\"\n\t}}\n\
                     \tModel: {id}, \"Model::{name}\", \"LimbNode\" {{\n\t\tVersion: 232\n\t\tProperties70:  {{\n\
                     \t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,0,{t}\n\t\t}}\n\t}}\n"
                ));
                conns.push_str(&format!("\tC: \"OO\",{aid},{id}\n"));
            }
            conns.push_str("\tC: \"OO\",210,0\n\tC: \"OO\",211,210\n");
            s.push_str("\tDeformer: 230, \"Deformer::Skin\", \"Skin\" {\n\t\tVersion: 101\n\t\tLink_DeformAcuracy: 50\n\t}\n");
            conns.push_str("\tC: \"OO\",230,100\n");
            let n = self.verts.len();
            let low: Vec<usize> = (0..n).filter(|&i| self.verts[i][2] < h * 0.5).collect();
            let high: Vec<usize> = (0..n).filter(|&i| self.verts[i][2] >= h * 0.5).collect();
            let list = |v: &[usize]| {
                v.iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let ones = |c: usize| vec!["1"; c].join(",");
            let id16 = "1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1".to_string();
            // No FBX, `Transform` do cluster é a matriz malha→osso (bind inversa) e
            // `TransformLink` é osso→mundo na pose de bind.
            let link_b = format!("1,0,0,0,0,1,0,0,0,0,1,0,0,0,{},1", h * 0.5);
            let inv_b = format!("1,0,0,0,0,1,0,0,0,0,1,0,0,0,{},1", -h * 0.5);
            s.push_str(&format!(
                "\tDeformer: 231, \"SubDeformer::BoneA\", \"Cluster\" {{\n\t\tVersion: 100\n\t\tUserData: \"\", \"\"\n\
                 \t\tIndexes: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t\tWeights: *{} {{\n\t\t\ta: {}\n\t\t}}\n\
                 \t\tTransform: *16 {{\n\t\t\ta: {id16}\n\t\t}}\n\t\tTransformLink: *16 {{\n\t\t\ta: {id16}\n\t\t}}\n\t}}\n",
                low.len(),
                list(&low),
                low.len(),
                ones(low.len())
            ));
            s.push_str(&format!(
                "\tDeformer: 232, \"SubDeformer::BoneB\", \"Cluster\" {{\n\t\tVersion: 100\n\t\tUserData: \"\", \"\"\n\
                 \t\tIndexes: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t\tWeights: *{} {{\n\t\t\ta: {}\n\t\t}}\n\
                 \t\tTransform: *16 {{\n\t\t\ta: {inv_b}\n\t\t}}\n\t\tTransformLink: *16 {{\n\t\t\ta: {link_b}\n\t\t}}\n\t}}\n",
                high.len(),
                list(&high),
                high.len(),
                ones(high.len())
            ));
            conns.push_str("\tC: \"OO\",231,230\n\tC: \"OO\",232,230\n\tC: \"OO\",210,231\n\tC: \"OO\",211,232\n");
        }

        if self.blend {
            let h = self.verts.iter().map(|v| v[2]).fold(0.0, f64::max);
            s.push_str("\tDeformer: 220, \"Deformer::BlendShape\", \"BlendShape\" {\n\t\tVersion: 100\n\t}\n");
            s.push_str("\tDeformer: 221, \"SubDeformer::Smile\", \"BlendShapeChannel\" {\n\t\tVersion: 100\n\t\tDeformPercent: 25\n\t\tFullWeights: *1 {\n\t\t\ta: 100\n\t\t}\n\t}\n");
            s.push_str(&format!(
                "\tGeometry: 222, \"Geometry::Smile\", \"Shape\" {{\n\t\tVersion: 100\n\t\tIndexes: *1 {{\n\t\t\ta: 6\n\t\t}}\n\
                 \t\tVertices: *3 {{\n\t\t\ta: 0,0,{}\n\t\t}}\n\t\tNormals: *3 {{\n\t\t\ta: 0,0,0\n\t\t}}\n\t}}\n",
                h * 0.5
            ));
            conns.push_str("\tC: \"OO\",220,100\n\tC: \"OO\",221,220\n\tC: \"OO\",222,221\n");
        }

        if self.anim {
            // translação X do modelo: 0 → 100 unidades do arquivo em 1 s (KTime 46186158000)
            s.push_str(
                "\tAnimationStack: 400, \"AnimStack::Take 001\", \"\" {\n\t\tProperties70:  {\n\
                 \t\t\tP: \"LocalStart\", \"KTime\", \"Time\", \"\",0\n\
                 \t\t\tP: \"LocalStop\", \"KTime\", \"Time\", \"\",46186158000\n\
                 \t\t\tP: \"ReferenceStart\", \"KTime\", \"Time\", \"\",0\n\
                 \t\t\tP: \"ReferenceStop\", \"KTime\", \"Time\", \"\",46186158000\n\t\t}\n\t}\n",
            );
            s.push_str("\tAnimationLayer: 401, \"AnimLayer::BaseLayer\", \"\" {\n\t}\n");
            s.push_str(
                "\tAnimationCurveNode: 402, \"AnimCurveNode::T\", \"\" {\n\t\tProperties70:  {\n\
                 \t\t\tP: \"d|X\", \"Number\", \"\", \"A\",0\n\t\t\tP: \"d|Y\", \"Number\", \"\", \"A\",0\n\t\t\tP: \"d|Z\", \"Number\", \"\", \"A\",0\n\t\t}\n\t}\n",
            );
            s.push_str(
                "\tAnimationCurve: 403, \"AnimCurve::\", \"\" {\n\t\tDefault: 0\n\t\tKeyVer: 4008\n\
                 \t\tKeyTime: *2 {\n\t\t\ta: 0,46186158000\n\t\t}\n\
                 \t\tKeyValueFloat: *2 {\n\t\t\ta: 0,100\n\t\t}\n\
                 \t\tKeyAttrFlags: *1 {\n\t\t\ta: 4\n\t\t}\n\
                 \t\tKeyAttrDataFloat: *4 {\n\t\t\ta: 0,0,0,0\n\t\t}\n\
                 \t\tKeyAttrRefCount: *1 {\n\t\t\ta: 2\n\t\t}\n\t}\n",
            );
            conns.push_str("\tC: \"OO\",401,400\n\tC: \"OO\",402,401\n\tC: \"OP\",403,402, \"d|X\"\n\tC: \"OP\",402,200, \"Lcl Translation\"\n");
        }

        s.push_str("}\nConnections:  {\n");
        s.push_str(&conns);
        s.push_str("}\n");
        s
    }
}

fn extents(m: &EditMesh) -> Vec3 {
    let (lo, hi) = m.bounds().expect("malha com vértices");
    hi - lo
}

/// Volume com sinal (teorema da divergência, leque por face): > 0 = normais para fora.
fn signed_volume(m: &EditMesh) -> f32 {
    let mut vol = 0.0;
    for f in &m.faces {
        let p: Vec<Vec3> = f
            .corners
            .iter()
            .map(|c| m.verts[c.vert as usize].co)
            .collect();
        for k in 1..p.len() - 1 {
            vol += p[0].dot(p[k].cross(p[k + 1])) / 6.0;
        }
    }
    vol
}

#[test]
fn imports_box_with_ngons_normals_uv_and_material() {
    let bytes = Fixture::boxed(1.0, 2.0, 3.0).to_ascii().into_bytes();
    let scene = import_fbx(&bytes).expect("importa");
    assert!(scene.info.ascii);
    assert_eq!(scene.meshes.len(), 1);
    let fm = &scene.meshes[0];
    let m = &fm.mesh;
    m.validate().expect("EditMesh válido");
    assert_eq!(m.vert_count(), 8);
    // quads preservados (não triangulados): 6 faces de 4 corners
    assert_eq!(m.face_count(), 6);
    assert!(
        m.faces.iter().all(|f| f.len() == 4),
        "n-gons devem ser preservados"
    );
    assert_eq!(scene.info.dropped_faces, 0);

    // normais por corner == normal geométrica da face; UV por corner distintos em [0,1]
    for (fi, f) in m.faces.iter().enumerate() {
        let gn = m.face_normal(fi as u32);
        let mut uvs = Vec::new();
        for c in &f.corners {
            let n = c.normal.expect("normal por corner");
            assert!(
                n.dot(gn) > 0.999,
                "normal do corner diverge da face {fi}: {n:?} vs {gn:?}"
            );
            assert!((0.0..=1.0).contains(&c.uv.x) && (0.0..=1.0).contains(&c.uv.y));
            uvs.push((c.uv.x.to_bits(), c.uv.y.to_bits()));
        }
        uvs.sort();
        uvs.dedup();
        assert_eq!(uvs.len(), 4, "UV por corner colapsou na face {fi}");
    }
    // enrolamento CCW visto de fora
    assert!(signed_volume(m) > 0.0, "normais para dentro");
    assert!(
        (signed_volume(m) - 6.0).abs() < 1e-3,
        "volume 1×2×3 = 6, veio {}",
        signed_volume(m)
    );

    // material por face
    assert!(m.faces.iter().all(|f| f.material == 0));
    assert_eq!(fm.material_slots, vec![0]);
    assert_eq!(scene.materials.len(), 1);
    assert_eq!(scene.materials[0].name, "Red");
    let c = scene.materials[0].base_color;
    assert!(c[0] > 0.99 && c[1] < 0.01 && c[2] < 0.01, "cor base {c:?}");

    // hierarquia: um nó com a malha, sem o nó raiz sintético
    assert_eq!(scene.nodes.len(), 1);
    assert_eq!(scene.roots, vec![0]);
    assert_eq!(scene.nodes[0].name, "Box");
    assert_eq!(scene.nodes[0].mesh, Some(0));
    assert!(scene.nodes[0].translation.length() < 1e-6);
    assert!(scene.nodes[0].scale.abs_diff_eq(Vec3::ONE, 1e-6));
}

#[test]
fn z_up_centimeters_become_y_up_meters() {
    let mut fx = Fixture::boxed(100.0, 200.0, 300.0); // cm; Z é a altura
    fx.up_axis = 2;
    fx.unit_scale = 1.0;
    let scene = import_fbx(fx.to_ascii().as_bytes()).expect("importa");
    assert_eq!(scene.info.original_up_axis, "PositiveZ");
    assert!((scene.info.original_unit_meters - 0.01).abs() < 1e-9);

    // unidade: embutida na geometria local (cm → m); orientação ainda a do arquivo
    let local = &scene.meshes[0].mesh;
    let le = extents(local);
    assert!(
        (le - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-3,
        "geometria local {le:?}"
    );
    // eixos: a rotação Z-up → Y-up está no TRS do nó (e a escala NÃO é herdada)
    let node = &scene.nodes[0];
    assert!(
        node.rotation.angle_between(Quat::IDENTITY) > 1.0,
        "nó sem rotação de eixos: {:?}",
        node.rotation
    );
    assert!(
        node.scale.abs_diff_eq(Vec3::ONE, 1e-4),
        "escala herdada: {:?}",
        node.scale
    );

    // espaço do mundo: Y para cima, metros
    let m = scene
        .mesh_in_world_space(0)
        .expect("malha do nó 0 em espaço do mundo");
    let e = extents(&m);
    assert!((e.x - 1.0).abs() < 1e-3, "largura {e:?}");
    assert!((e.y - 3.0).abs() < 1e-3, "altura {e:?}");
    assert!((e.z - 2.0).abs() < 1e-3, "profundidade {e:?}");
    // enrolamento e normais continuam para fora após a rotação de eixos
    assert!(
        signed_volume(&m) > 0.0,
        "conversão de eixos virou a malha do avesso"
    );
    assert!(
        (signed_volume(&m) - 6.0).abs() < 1e-2,
        "volume {}",
        signed_volume(&m)
    );
    for (fi, f) in m.faces.iter().enumerate() {
        let gn = m.face_normal(fi as u32);
        for c in &f.corners {
            assert!(
                c.normal.expect("normal").dot(gn) > 0.999,
                "normal não acompanhou a conversão"
            );
        }
    }
    // a face de cima do arquivo (+Z) virou +Y
    let top = (0..m.face_count() as u32)
        .max_by(|a, b| m.face_center(*a).y.total_cmp(&m.face_center(*b).y))
        .expect("face");
    assert!(m.face_normal(top).dot(Vec3::Y) > 0.999);
    assert!(scene.mesh_in_world_space(99).is_none());
}

#[test]
fn single_ngon_is_not_triangulated() {
    // hexágono plano (um único n-gon de 6 corners) + normais/UV por corner
    let verts: Vec<[f64; 3]> = (0..6)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 6.0;
            [a.cos(), 0.0, a.sin()]
        })
        .collect();
    let mut fx = Fixture::boxed(1.0, 1.0, 1.0);
    fx.verts = verts;
    fx.polys = vec![vec![0, 5, 4, 3, 2, 1]];
    fx.normals = vec![[0.0, 1.0, 0.0]; 6];
    fx.uv_table = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, 1.0],
        [0.0, 1.0],
        [0.5, 0.5],
        [0.25, 0.75],
    ];
    fx.uv_index = vec![0, 1, 2, 3, 4, 5];
    let scene = import_fbx(fx.to_ascii().as_bytes()).expect("importa");
    let m = &scene.meshes[0].mesh;
    assert_eq!(m.face_count(), 1);
    assert_eq!(m.faces[0].len(), 6, "hexágono preservado como n-gon");
    assert_eq!(m.triangle_count(), 4);
    assert!(m.faces[0].corners.iter().all(|c| c.normal.is_some()));
    let uvs: Vec<Vec2> = m.faces[0].corners.iter().map(|c| c.uv).collect();
    assert_eq!(uvs.len(), 6);
    assert!(uvs.contains(&Vec2::new(0.25, 0.75)));
}

#[test]
fn skin_weights_top4_and_bones() {
    let mut fx = Fixture::boxed(1.0, 1.0, 2.0);
    fx.skin = true;
    let scene = import_fbx(fx.to_ascii().as_bytes()).expect("importa");
    assert_eq!(scene.skins.len(), 1);
    let skin = &scene.skins[0];
    assert_eq!(skin.mesh, 0);
    assert_eq!(skin.joints.len(), 2);
    assert_eq!(scene.meshes[0].skin, Some(0));
    let names: Vec<&str> = skin.joints.iter().map(|j| j.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.contains("BoneA")) && names.iter().any(|n| n.contains("BoneB")),
        "{names:?}"
    );
    for j in &skin.joints {
        let n = j.node.expect("cluster aponta para um nó");
        assert!(scene.nodes[n].is_bone);
    }
    // hierarquia: BoneB é filho de BoneA
    let a = scene
        .nodes
        .iter()
        .position(|n| n.name == "BoneA")
        .expect("BoneA");
    let b = scene
        .nodes
        .iter()
        .position(|n| n.name == "BoneB")
        .expect("BoneB");
    assert_eq!(scene.nodes[b].parent, Some(a));
    assert!(scene.nodes[a].children.contains(&b));
    // pesos: soma 1; vértices de baixo → BoneA, de cima → BoneB (arquivo Y-up: o eixo
    // longo da caixa é Z, em metros, sem rotação nos nós)
    let m = &scene.meshes[0].mesh;
    for v in &m.verts {
        let sum: f32 = v.weights.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-5,
            "pesos não somam 1: {:?}",
            v.weights
        );
    }
    let bone_of = |v: &EVert| {
        skin.joints[v.joints[0] as usize]
            .node
            .map(|n| scene.nodes[n].name.as_str())
    };
    assert!(m
        .verts
        .iter()
        .filter(|v| v.co.z < 0.5)
        .all(|v| bone_of(v) == Some("BoneA")));
    assert!(m
        .verts
        .iter()
        .filter(|v| v.co.z > 1.5)
        .all(|v| bone_of(v) == Some("BoneB")));
    // bind inverso do BoneB: translação oposta a meia altura (1 m) no espaço do osso
    let jb = skin
        .joints
        .iter()
        .find(|j| j.name.contains("BoneB"))
        .expect("joint B");
    let t = jb.inverse_bind.w_axis.truncate();
    assert!(
        (t - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-3,
        "inverse_bind de B: {t:?}"
    );
    // e o bind→mundo de B está a 1 m de altura
    let w = jb.bind_to_world.w_axis.truncate();
    assert!(
        (w - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-3,
        "bind_to_world de B: {w:?}"
    );
}

#[test]
fn blend_shape_has_sparse_deltas_in_converted_space() {
    let mut fx = Fixture::boxed(100.0, 100.0, 200.0);
    fx.up_axis = 2;
    fx.unit_scale = 1.0;
    fx.blend = true;
    let scene = import_fbx(fx.to_ascii().as_bytes()).expect("importa");
    let shapes = &scene.meshes[0].blend_shapes;
    assert_eq!(shapes.len(), 1);
    let s = &shapes[0];
    assert!(s.name.contains("Smile"), "nome: {}", s.name);
    assert!(
        (s.default_weight - 0.25).abs() < 1e-4,
        "peso inicial {}",
        s.default_weight
    );
    assert_eq!(s.deltas.len(), 1, "deltas esparsos");
    assert_eq!(s.deltas[0].vertex, 6);
    // delta de +100 cm em Z (altura do arquivo) = +1 m em Z local (unidade embutida) …
    assert!(
        (s.deltas[0].position - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-3,
        "delta local {:?}",
        s.deltas[0].position
    );
    // … e +1 m em Y no mundo, depois da rotação de eixos do nó
    let world = scene
        .node_world_matrix(0)
        .expect("matriz")
        .transform_vector3(s.deltas[0].position);
    assert!(
        (world - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-3,
        "delta no mundo {world:?}"
    );
}

#[test]
fn animation_clip_is_baked_in_meters() {
    let mut fx = Fixture::boxed(1.0, 1.0, 1.0);
    fx.anim = true;
    fx.unit_scale = 1.0; // cm: 100 → 1 m
    let scene = import_fbx(fx.to_ascii().as_bytes()).expect("importa");
    assert_eq!(scene.clips.len(), 1);
    let clip = &scene.clips[0];
    assert!(clip.name.contains("Take 001"));
    assert!(
        (clip.duration - 1.0).abs() < 1e-3,
        "duração {}",
        clip.duration
    );
    let track = clip
        .tracks
        .iter()
        .find(|t| scene.nodes[t.node].name == "Box")
        .expect("trilha do nó");
    let first = track.translation.first().expect("chave inicial");
    let last = track.translation.last().expect("chave final");
    assert!(
        first.0.abs() < 1e-4 && first.1.length() < 1e-4,
        "início {first:?}"
    );
    assert!((last.0 - 1.0).abs() < 1e-3, "tempo final {}", last.0);
    assert!(
        (last.1.x - 1.0).abs() < 1e-3,
        "X final {:?} (100 cm = 1 m)",
        last.1
    );
    // sem animação quando desligada
    let off = import_fbx_with(
        fx.to_ascii().as_bytes(),
        &FbxImportOptions {
            import_animations: false,
            ..FbxImportOptions::default()
        },
    )
    .expect("importa");
    assert!(off.clips.is_empty());
}

#[test]
fn import_is_deterministic() {
    let mut fx = Fixture::boxed(1.0, 2.0, 3.0);
    fx.skin = true;
    fx.blend = true;
    fx.anim = true;
    let bytes = fx.to_ascii().into_bytes();
    assert_eq!(
        import_fbx(&bytes).expect("a"),
        import_fbx(&bytes).expect("b")
    );
}

#[test]
fn garbage_empty_and_truncated_bytes_return_err() {
    assert!(matches!(import_fbx(&[]), Err(NativeError::EmptyInput(_))));
    assert!(matches!(
        import_fbx(b"isto nao e um fbx"),
        Err(NativeError::Backend { .. })
    ));
    assert!(import_fbx(&[0xff; 4096]).is_err());
    // cabeçalho binário legítimo com lixo depois
    let mut fake = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
    fake.extend_from_slice(&7400u32.to_le_bytes());
    fake.extend_from_slice(&[0xAB; 600]);
    assert!(import_fbx(&fake).is_err());
    // pseudo-aleatório determinístico
    let mut x = 0x1234_5678u32;
    let noise: Vec<u8> = (0..8192)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect();
    assert!(import_fbx(&noise).is_err());
    // FBX ASCII truncado no meio não pode dar pânico (Ok parcial ou Err)
    let ascii = Fixture::boxed(1.0, 1.0, 1.0).to_ascii();
    for cut in [10, 80, 400, ascii.len() / 2, ascii.len() - 5] {
        let _ = import_fbx(&ascii.as_bytes()[..cut]);
    }
}

#[test]
fn limits_are_enforced() {
    let bytes = Fixture::boxed(1.0, 1.0, 1.0).to_ascii().into_bytes();
    let tiny = FbxImportOptions {
        max_input_bytes: 16,
        ..FbxImportOptions::default()
    };
    assert!(matches!(
        import_fbx_with(&bytes, &tiny),
        Err(NativeError::LimitExceeded { .. })
    ));
    let bad_rate = FbxImportOptions {
        animation_sample_rate: 0.0,
        ..FbxImportOptions::default()
    };
    assert!(matches!(
        import_fbx_with(&bytes, &bad_rate),
        Err(NativeError::InvalidInput(_))
    ));
    let nan_rate = FbxImportOptions {
        animation_sample_rate: f64::NAN,
        ..FbxImportOptions::default()
    };
    assert!(import_fbx_with(&bytes, &nan_rate).is_err());
}

#[test]
fn imported_mesh_feeds_the_render_bridge() {
    // EditMesh importado deve converter para anigo_core::Mesh sem erro
    let scene = import_fbx(Fixture::boxed(1.0, 2.0, 3.0).to_ascii().as_bytes()).expect("importa");
    let core = anigo_modeling::convert::to_core_mesh(
        &scene.meshes[0].mesh,
        &anigo_modeling::convert::ToCoreOptions::default(),
    )
    .expect("ponte para o render");
    assert_eq!(core.indices.len() / 3, 12);
}
