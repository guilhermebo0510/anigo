use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

pub struct LiveBridgeClient {
    pub addr: String,
}

impl LiveBridgeClient {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }

    pub async fn send_command(&self, action: &str, params: Value) -> Result<Value> {
        // P1-06: Timeout for connect (800ms) and read (2s)
        let connect_fut = TcpStream::connect(&self.addr);
        let stream = timeout(Duration::from_millis(800), connect_fut)
            .await
            .map_err(|_| anyhow::anyhow!("Bridge connect timeout 800ms to {} (ANIGO Live Studio not responding)", self.addr))?
            .with_context(|| format!("ANIGO Live Studio is not running on {}", self.addr))?;

        stream.set_nodelay(true).ok();

        let req_id = format!(
            "mcp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        let payload = json!({
            "id": req_id,
            "action": action,
            "params": params,
        });

        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');

        // Protect against huge payloads (P2-14)
        const MAX_LINE: usize = 1 << 20; // 1MB
        if line.len() > MAX_LINE {
            anyhow::bail!("Bridge payload too large: {} > {} bytes", line.len(), MAX_LINE);
        }

        let mut stream = stream;
        // Write with timeout
        timeout(Duration::from_secs(2), stream.write_all(line.as_bytes()))
            .await
            .map_err(|_| anyhow::anyhow!("Bridge write timeout to {}", self.addr))?
            .context("Failed to write to bridge")?;
        timeout(Duration::from_secs(1), stream.flush())
            .await
            .map_err(|_| anyhow::anyhow!("Bridge flush timeout"))?
            .context("Failed to flush bridge")?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();

        // Read with timeout 2s + max line length protection
        let read_fut = reader.read_line(&mut response_line);
        timeout(Duration::from_secs(2), read_fut)
            .await
            .map_err(|_| anyhow::anyhow!("Bridge read timeout (2s) from {} for action {}", self.addr, action))?
            .context("Failed to read from bridge")?;

        if response_line.len() > MAX_LINE {
            anyhow::bail!("Bridge response too large: {} > {} bytes", response_line.len(), MAX_LINE);
        }

        if response_line.trim().is_empty() {
            anyhow::bail!("Empty response from bridge for action {}", action);
        }

        let resp: Value = serde_json::from_str(&response_line)
            .with_context(|| format!("Failed to parse bridge response: {}", response_line))?;

        if resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(resp.get("data").cloned().unwrap_or(Value::Null))
        } else {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown bridge error");
            anyhow::bail!("Live bridge action failed [{}]: {}", action, err);
        }
    }

    pub async fn is_live(&self) -> bool {
        // is_live should be fast: use timeout already inside send_command
        match timeout(Duration::from_millis(900), self.send_command("PING", json!({}))).await {
            Ok(Ok(_)) => true,
            _ => false,
        }
    }
}
