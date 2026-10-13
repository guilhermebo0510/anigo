//! Núcleo puro do workspace Posing (sem egui): edição de pose (FK, IK, simetria, quadril, pés), picking de
//! juntas, visualização de limites e o gesto de arrasto (`GestureTracker`).
//!
//! A UI (`app::posing_ui`) só costura estes módulos e emite `Command::SetPose` para o núcleo. Toda a
//! matemática de "como a pose muda" mora aqui e tem testes numéricos (comprimentos de osso, limites,
//! pés travados, ausência de NaN).

pub mod edit;
pub mod gesture;
pub mod limits_vis;
pub mod pick;

pub use edit::{EditOpts, HipsMove};
pub use gesture::GestureTracker;
