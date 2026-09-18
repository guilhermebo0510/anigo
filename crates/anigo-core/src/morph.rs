use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use crate::mesh::Vertex;

/// Packed 32-byte sparse morph delta representation.
/// Exactly 8 x 32-bit words:
/// - `vertex_index`: u32 (offset 0)
/// - `delta_position`: [f32; 3] (offset 4)
/// - `delta_normal`: [f32; 3] (offset 16)
/// - `_pad`: f32 (offset 28)
///
/// Guaranteed 4-byte aligned, 16-byte compatible, Pod and Zeroable for GPU buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
pub struct SparseMorphDelta {
    pub vertex_index: u32,
    pub delta_position: [f32; 3],
    pub delta_normal: [f32; 3],
    pub _pad: f32,
}

impl SparseMorphDelta {
    pub fn new(vertex_index: u32, delta_position: [f32; 3], delta_normal: [f32; 3]) -> Self {
        Self {
            vertex_index,
            delta_position,
            delta_normal,
            _pad: 0.0,
        }
    }
}

/// Active morph channel metadata for GPU compute pass.
/// Exactly 16 bytes (4 x 32-bit words):
/// - `weight`: f32 (offset 0)
/// - `start_offset`: u32 (offset 4) - index of first delta in delta buffer
/// - `delta_count`: u32 (offset 8) - number of deltas for this channel
/// - `_pad`: u32 (offset 12)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
pub struct MorphChannel {
    pub weight: f32,
    pub start_offset: u32,
    pub delta_count: u32,
    pub _pad: u32,
}

impl MorphChannel {
    pub fn new(weight: f32, start_offset: u32, delta_count: u32) -> Self {
        Self {
            weight,
            start_offset,
            delta_count,
            _pad: 0,
        }
    }
}

/// Global uniform header for sparse morph compute shader.
/// Exactly 16 bytes (4 x 32-bit words):
/// - `active_channel_count`: u32 (offset 0)
/// - `total_vertex_count`: u32 (offset 4)
/// - `total_delta_count`: u32 (offset 8)
/// - `_pad`: u32 (offset 12)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
pub struct SparseMorphHeader {
    pub active_channel_count: u32,
    pub total_vertex_count: u32,
    pub total_delta_count: u32,
    pub _pad: u32,
}

impl SparseMorphHeader {
    pub fn new(active_channel_count: u32, total_vertex_count: u32, total_delta_count: u32) -> Self {
        Self {
            active_channel_count,
            total_vertex_count,
            total_delta_count,
            _pad: 0,
        }
    }
}

/// Named morph target containing a collection of sparse deltas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphTarget {
    pub name: String,
    pub deltas: Vec<SparseMorphDelta>,
}

impl MorphTarget {
    pub fn new(name: impl Into<String>, mut deltas: Vec<SparseMorphDelta>) -> Self {
        deltas.sort_by_key(|d| d.vertex_index);
        Self {
            name: name.into(),
            deltas,
        }
    }
}

/// Repository of sparse morph targets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SparseMorphSet {
    pub targets: Vec<MorphTarget>,
}

impl SparseMorphSet {
    pub fn new() -> Self {
        Self {
            targets: Vec::new(),
        }
    }

    pub fn add_target(&mut self, name: impl Into<String>, deltas: Vec<SparseMorphDelta>) -> usize {
        let index = self.targets.len();
        self.targets.push(MorphTarget::new(name, deltas));
        index
    }

    pub fn find_target(&self, name: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.name == name)
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Packs active morph channels and deltas for GPU upload.
    /// Filters out channels with weight magnitude < `epsilon` (default 1e-6) to minimize GPU workload.
    pub fn pack_active_channels(
        &self,
        weights: &[f32],
        total_vertex_count: u32,
        epsilon: f32,
    ) -> (SparseMorphHeader, Vec<MorphChannel>, Vec<SparseMorphDelta>) {
        let mut active_channels = Vec::new();
        let mut packed_deltas = Vec::new();

        for (i, target) in self.targets.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(0.0);
            if weight.abs() > epsilon && !target.deltas.is_empty() {
                let start_offset = packed_deltas.len() as u32;
                let delta_count = target.deltas.len() as u32;
                packed_deltas.extend_from_slice(&target.deltas);
                active_channels.push(MorphChannel::new(weight, start_offset, delta_count));
            }
        }

        let header = SparseMorphHeader::new(
            active_channels.len() as u32,
            total_vertex_count,
            packed_deltas.len() as u32,
        );

        (header, active_channels, packed_deltas)
    }

    /// Packs ALL channels (active and inactive) for pre-allocated static GPU storage buffers.
    pub fn pack_all_channels(
        &self,
        weights: &[f32],
        total_vertex_count: u32,
    ) -> (SparseMorphHeader, Vec<MorphChannel>, Vec<SparseMorphDelta>) {
        let mut channels = Vec::with_capacity(self.targets.len());
        let mut packed_deltas = Vec::new();

        for (i, target) in self.targets.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(0.0);
            let start_offset = packed_deltas.len() as u32;
            let delta_count = target.deltas.len() as u32;
            packed_deltas.extend_from_slice(&target.deltas);
            channels.push(MorphChannel::new(weight, start_offset, delta_count));
        }

        let header = SparseMorphHeader::new(
            channels.len() as u32,
            total_vertex_count,
            packed_deltas.len() as u32,
        );

        (header, channels, packed_deltas)
    }

    /// CPU reference evaluation for verification, unit testing, and headless fallback.
    /// Accumulates:
    /// $$p_{\text{out}} = p_{\text{base}} + \sum_k w_k \Delta p_k$$
    /// $$n_{\text{out}} = \text{normalize}(n_{\text{base}} + \sum_k w_k \Delta n_k)$$
    pub fn apply_cpu(
        &self,
        weights: &[f32],
        base_vertices: &[Vertex],
        out_vertices: &mut [Vertex],
    ) {
        assert_eq!(
            base_vertices.len(),
            out_vertices.len(),
            "Vertex buffer sizes must match"
        );

        // Initialize output with base vertices
        out_vertices.copy_from_slice(base_vertices);

        // Accumulate active deltas per vertex
        for (i, target) in self.targets.iter().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(0.0);
            if weight.abs() < 1e-7 {
                continue;
            }

            for delta in &target.deltas {
                let v_idx = delta.vertex_index as usize;
                if v_idx < out_vertices.len() {
                    let v = &mut out_vertices[v_idx];
                    v.position[0] += weight * delta.delta_position[0];
                    v.position[1] += weight * delta.delta_position[1];
                    v.position[2] += weight * delta.delta_position[2];

                    v.normal[0] += weight * delta.delta_normal[0];
                    v.normal[1] += weight * delta.delta_normal[1];
                    v.normal[2] += weight * delta.delta_normal[2];
                }
            }
        }

        // Normalize normals
        for v in out_vertices.iter_mut() {
            let len_sq = v.normal[0] * v.normal[0]
                + v.normal[1] * v.normal[1]
                + v.normal[2] * v.normal[2];
            if len_sq > 1e-12 {
                let inv_len = 1.0 / len_sq.sqrt();
                v.normal[0] *= inv_len;
                v.normal[1] *= inv_len;
                v.normal[2] *= inv_len;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_morph_structures_pod_and_zeroable() {
        assert_eq!(std::mem::size_of::<SparseMorphDelta>(), 32);
        assert_eq!(std::mem::align_of::<SparseMorphDelta>(), 4);

        assert_eq!(std::mem::size_of::<MorphChannel>(), 16);
        assert_eq!(std::mem::align_of::<MorphChannel>(), 4);

        assert_eq!(std::mem::size_of::<SparseMorphHeader>(), 16);
        assert_eq!(std::mem::align_of::<SparseMorphHeader>(), 4);

        let delta = SparseMorphDelta::new(42, [1.0, 2.0, 3.0], [0.0, 1.0, 0.0]);
        let bytes = bytemuck::bytes_of(&delta);
        assert_eq!(bytes.len(), 32);

        let roundtrip: SparseMorphDelta = *bytemuck::from_bytes(bytes);
        assert_eq!(delta, roundtrip);
    }

    #[test]
    fn test_sparse_morph_set_pack_active() {
        let mut set = SparseMorphSet::new();

        // Channel 0: Nose bridge (affects vertex 2)
        set.add_target(
            "nose_bridge",
            vec![SparseMorphDelta::new(2, [0.0, 0.05, 0.02], [0.0, 0.0, 0.1])],
        );

        // Channel 1: Jaw width (affects vertices 0 and 1)
        set.add_target(
            "jaw_width",
            vec![
                SparseMorphDelta::new(0, [-0.04, 0.0, 0.0], [-0.1, 0.0, 0.0]),
                SparseMorphDelta::new(1, [0.04, 0.0, 0.0], [0.1, 0.0, 0.0]),
            ],
        );

        // Channel 2: Inactive (weight 0.0)
        set.add_target(
            "eye_scale",
            vec![SparseMorphDelta::new(3, [0.0, 0.02, 0.0], [0.0, 0.0, 0.0])],
        );

        let weights = [0.8, 0.5, 0.0];
        let (header, active_channels, packed_deltas) =
            set.pack_active_channels(&weights, 10, 1e-5);

        assert_eq!(header.active_channel_count, 2);
        assert_eq!(header.total_vertex_count, 10);
        assert_eq!(header.total_delta_count, 3);
        assert_eq!(active_channels.len(), 2);
        assert_eq!(packed_deltas.len(), 3);

        // Channel 0
        assert_eq!(active_channels[0].weight, 0.8);
        assert_eq!(active_channels[0].start_offset, 0);
        assert_eq!(active_channels[0].delta_count, 1);

        // Channel 1
        assert_eq!(active_channels[1].weight, 0.5);
        assert_eq!(active_channels[1].start_offset, 1);
        assert_eq!(active_channels[1].delta_count, 2);
    }

    #[test]
    fn test_sparse_morph_apply_cpu() {
        let mut set = SparseMorphSet::new();
        set.add_target(
            "chin_forward",
            vec![SparseMorphDelta::new(0, [0.0, 0.0, 0.2], [0.0, 0.0, 0.5])],
        );

        let base_vertices = vec![
            Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
            Vertex::new([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5]),
        ];
        let mut out_vertices = vec![Vertex::new([0.0; 3], [0.0; 3], [0.0; 2]); 2];

        set.apply_cpu(&[0.5], &base_vertices, &mut out_vertices);

        // Vertex 0: position moved by 0.5 * 0.2 = 0.1 on Z
        assert_eq!(out_vertices[0].position, [0.0, 0.0, 0.1]);
        // Normal accumulated: [0, 0, 1] + 0.5 * [0, 0, 0.5] = [0, 0, 1.25] -> normalized to [0, 0, 1]
        assert_eq!(out_vertices[0].normal, [0.0, 0.0, 1.0]);

        // Vertex 1: untouched
        assert_eq!(out_vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(out_vertices[1].normal, [0.0, 1.0, 0.0]);
    }
}
