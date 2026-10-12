//! # anigo-charbase
//!
//! Mesh base de personagem (masculino e feminino, estilo anime) da Gen-0: gerador procedural
//! determinístico de um `Skin` (corpo e cabeça soldados, só quads), peças auxiliares (olhos em
//! camadas, cílios, sobrancelhas, interior da boca), esqueleto VRM humanoid completo com
//! localizadores de junta, pesos de skin iniciais, morphs de macro esparsos e o contrato de
//! topologia para o usuário esculpir posições localmente.
//!
//! Nada aqui altera o mesh canônico do `anigo-core`: é uma base *nova*, ao lado dele.

pub mod cage;
pub mod dims;
pub mod params;
pub mod qmesh;
pub mod subdiv;

pub use params::{FaceParams, Gender, Params};
