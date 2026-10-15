//! Minimal newline-delimited JSON client for the herdr socket API.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

pub struct Client {
    socket: PathBuf,
    next_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    pub agent_status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl Client {
    pub fn from_env() -> io::Result<Self> {
        let socket = match std::env::var_os("HERDR_SOCKET_PATH") {
            Some(path) => PathBuf::from(path),
            None => default_socket()?,
        };
        Ok(Self { socket, next_id: 0 })
    }

    fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.next_id += 1;
        let id = format!("oiia_{}", self.next_id);
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut line = json!({ "id": id, "method": method, "params": params }).to_string();
        line.push('\n');
        stream.write_all(line.as_bytes())?;

        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response)?;
        let mut value: Value = serde_json::from_str(&response)?;
        if let Some(error) = value.get("error") {
            return Err(io::Error::other(format!("{method}: {error}")));
        }
        Ok(value["result"].take())
    }

    pub fn agents(&mut self) -> io::Result<Vec<Agent>> {
        let mut result = self.request("agent.list", json!({}))?;
        Ok(serde_json::from_value(result["agents"].take())?)
    }

    /// Set (`Some`) or clear (`None`) pane tokens in one report.
    pub fn set_tokens(
        &mut self,
        pane_id: &str,
        tokens: &[(String, Option<&str>)],
        ttl_ms: Option<u64>,
    ) -> io::Result<()> {
        let tokens: HashMap<&str, Option<&str>> =
            tokens.iter().map(|(name, value)| (name.as_str(), *value)).collect();
        let mut params = json!({
            "pane_id": pane_id,
            "source": "plugin:oiia.status",
            "tokens": tokens,
        });
        if let Some(ttl) = ttl_ms {
            params["ttl_ms"] = json!(ttl);
        }
        self.request("pane.report_metadata", params).map(drop)
    }
}

fn default_socket() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(
            std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?,
        )
        .join(".config"),
    };
    Ok(base.join("herdr").join("herdr.sock"))
}
