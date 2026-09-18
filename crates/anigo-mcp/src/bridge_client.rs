use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct LiveBridgeClient {
    pub addr: String,
}

impl LiveBridgeClient {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    pub async fn send_command(&self, action: &str, params: Value) -> Result<Value> {
        let mut stream = TcpStream::connect(&self.addr)
            .await
            .with_context(|| format!("ANIGO Live Studio is not running on {}", self.addr))?;

        let req_id = format!(
            "mcp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
        );

        let payload = json!({
            "id": req_id,
            "action": action,
            "params": params,
        });

        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');

        stream.write_all(line.as_bytes()).await?;
        stream.flush().await?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        let resp: Value = serde_json::from_str(&response_line)
            .with_context(|| format!("Failed to parse bridge response: {}", response_line))?;

        if resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(resp.get("data").cloned().unwrap_or(Value::Null))
        } else {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown bridge error");
            anyhow::bail!("Live bridge action failed: {}", err);
        }
    }

    pub async fn is_live(&self) -> bool {
        match self.send_command("PING", json!({})).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }
}
