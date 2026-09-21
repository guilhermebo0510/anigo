pub mod culling;
pub mod device_recovery;
pub mod diagnostics;
pub mod headless;
pub mod mesh_validation;
pub mod render_contract;
pub mod render_graph;
pub mod uniforms;

pub use device_recovery::{
    DeviceRecreationReport, PresentationMode, ReconnectSchedule, UncapturedErrorBus,
    UncapturedErrorRecord, recent_errors,
};
pub use culling::{cull_volumes, CullReport, SceneVolume};
pub use headless::{HeadlessRenderer, RenderMetrics};
pub use diagnostics::{
    entries as diagnostics_entries, report as report_diagnostic, summary as diagnostics_summary,
    RenderDiagnostic, Severity as DiagnosticSeverity,
};
pub use render_contract::{
    culling_report, fnv1a64, offscreen_color_format, projection_modes, render_pass_order,
    toon_ramp_bytes, CullingContract, CONTRACT_JSON,
};
pub use render_graph::{
    graph_from_contract, AliasGroup, AliasingPlan, GraphError, GraphResource, GraphSettings, PassKind,
    PassNode, RenderGraph, ResourceKind, ResourceSize,
};
pub use uniforms::{CameraUniform, LightUniform, MaterialUniform, OutlineUniform};
