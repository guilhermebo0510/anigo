//! Envelope do protocolo da bridge. É o MESMO do bridge Tauri legado (`bridge_client.rs` do `anigo-mcp`):
//!
//! ```text
//! -> {"id":"mcp-1","action":"APPLY_COMMAND","params":{...},"token":"<hex>"}\n
//! <- {"id":"mcp-1","success":true,"data":{...}}\n
//! <- {"id":"mcp-1","success":false,"error":"mensagem","code":-32602}\n
//! ```
//!
//! `code` é ADITIVO (clientes antigos o ignoram). Mensagens de auth contêm "Unauthorized"/"token" para o
//! cliente legado continuar produzindo a mensagem acionável.

use serde::Deserialize;
use serde_json::{json, Value};

/// Versão do protocolo (sobe só em mudança incompatível do envelope).
pub const PROTOCOL_VERSION: u32 = 1;

pub const MAX_REQUEST_BYTES: usize = 1 << 20;
pub const MAX_RESPONSE_BYTES: usize = 32 << 20;

/// Códigos de erro (estilo JSON-RPC).
pub mod code {
    pub const UNAUTHORIZED: i32 = -32001;
    pub const UI_TIMEOUT: i32 = -32002;
    pub const BUSY: i32 = -32003;
    pub const RATE_LIMITED: i32 = -32004;
    pub const TOO_LARGE: i32 = -32005;
    pub const SHUTTING_DOWN: i32 = -32006;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const UNKNOWN_ACTION: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const ACTION_FAILED: i32 = -32000;
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub token: Option<String>,
}

impl Request {
    /// Id em texto (o cliente legado usa string; aceitamos número por tolerância).
    pub fn id_string(&self) -> String {
        match &self.id {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeError {
    pub code: i32,
    pub message: String,
}

impl BridgeError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_PARAMS, message)
    }
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(code::ACTION_FAILED, message)
    }
    pub fn unknown_action(action: &str) -> Self {
        Self::new(code::UNKNOWN_ACTION, format!("ação desconhecida: {action}"))
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for BridgeError {}

pub type BridgeResult = Result<Value, BridgeError>;

/// Resposta serializada (uma linha NDJSON / corpo HTTP).
pub fn response_json(id: &str, result: &BridgeResult) -> Value {
    match result {
        Ok(data) => json!({ "id": id, "success": true, "data": data }),
        Err(e) => json!({ "id": id, "success": false, "error": e.message, "code": e.code }),
    }
}

/// Comparação em tempo constante (o tamanho do token não é segredo).
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Token aleatório de 256 bits em hexadecimal (64 caracteres). SEM fallback fraco: sem entropia do SO, erro.
pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("sem entropia do sistema operacional: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare_behaves() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn tokens_are_256_bit_hex_and_distinct() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn response_shape_is_backward_compatible() {
        let ok = response_json("x", &Ok(json!({"a": 1})));
        assert_eq!(ok["success"], true);
        assert_eq!(ok["data"]["a"], 1);
        let err = response_json("x", &Err(BridgeError::new(code::UNAUTHORIZED, "Unauthorized: token")));
        assert_eq!(err["success"], false);
        assert_eq!(err["error"], "Unauthorized: token");
        assert_eq!(err["code"], code::UNAUTHORIZED);
        assert!(err.get("data").is_none());
    }

    #[test]
    fn request_accepts_missing_optionals_and_numeric_ids() {
        let r: Request = serde_json::from_str(r#"{"action":"PING"}"#).unwrap();
        assert_eq!(r.action, "PING");
        assert!(r.token.is_none() && r.params.is_null());
        let r: Request = serde_json::from_str(r#"{"id":7,"action":"PING","params":{"a":1},"token":"t"}"#).unwrap();
        assert_eq!(r.id_string(), "7");
        assert_eq!(r.token.as_deref(), Some("t"));
        assert!(serde_json::from_str::<Request>(r#"{"id":"x"}"#).is_err(), "action é obrigatória");
    }
}
