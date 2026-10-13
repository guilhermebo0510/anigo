//! # anigo-bridge
//!
//! Servidor local (loopback + token por sessão) pelo qual o `anigo-mcp` controla o app NATIVO.
//! Decisões e ameaças: `docs/native/pesquisa/mcp_bridge_nativa.md`; API: `docs/native/MCP_BRIDGE_NATIVA.md`.
//!
//! * [`protocol`] — envelope NDJSON (compatível com o `bridge_client.rs` legado), erros, token;
//! * [`http`] — HTTP/1.1 mínimo e estrito (Host/Origin);
//! * [`server`] — threads + limites + auth; fala NDJSON e HTTP na mesma porta;
//! * [`host`] — ponte para a thread da UI (canal limitado, prazo, cancelamento);
//! * [`core_ops`] — `GET_PROJECT/APPLY_COMMAND/UNDO/REDO/GET_LOG/GET_SNAPSHOT` sobre a `CoreSession`;
//! * [`discovery`] — arquivo `bridge.json` (porta + token) com permissão restrita.

pub mod core_ops;
pub mod discovery;
pub mod host;
pub mod http;
pub mod protocol;
pub mod server;

pub use host::{channel, ChannelBackend, HostQueue, HostRequest, Reply};
pub use protocol::{BridgeError, BridgeResult, Request, PROTOCOL_VERSION};
pub use server::{Backend, BridgeServer, ServerConfig};
