//! Blocking, session-scoped SDK. Calls never retry browser mutations.
use crate::{registry, wire};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Options {
    pub socket: Option<PathBuf>,
    pub socket_dir: PathBuf,
    pub session_id: String,
    pub turn_id: String,
    pub timeout: Duration,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            socket: None,
            socket_dir: registry::default_dir(),
            session_id: format!("rust-{}", uuid::Uuid::new_v4()),
            turn_id: uuid::Uuid::new_v4().to_string(),
            timeout: Duration::from_secs(15),
        }
    }
}

pub struct Client {
    stream: Option<UnixStream>,
    options: Options,
    next_id: u64,
    notifications: VecDeque<Value>,
}
impl Client {
    pub fn connect(options: Options) -> Result<Self> {
        if options.timeout.is_zero() || options.session_id.is_empty() || options.turn_id.is_empty()
        {
            bail!("timeout and session/turn IDs must be nonempty");
        }
        let stream = if let Some(path) = &options.socket {
            UnixStream::connect(path).with_context(|| format!("connect {}", path.display()))?
        } else {
            registry::connect(&options.socket_dir)?
        };
        Ok(Self {
            stream: Some(stream),
            options,
            next_id: 0,
            notifications: VecDeque::new(),
        })
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(method, params, self.options.timeout)
    }
    pub(crate) fn request_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if timeout.is_zero() {
            bail!("browser request timed out");
        }
        if method.is_empty() {
            bail!("method must not be empty");
        }
        let mut params = params
            .as_object()
            .cloned()
            .context("params must be a JSON object")?;
        params.insert("session_id".into(), json!(self.options.session_id));
        params.insert("turn_id".into(), json!(self.options.turn_id));
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let result = (|| -> Result<(Value, bool)> {
            let stream = self
                .stream
                .as_ref()
                .context("connection closed; create a new Client to reconnect")?;
            let mut stream = DeadlineStream {
                stream,
                deadline: Instant::now() + timeout.min(self.options.timeout),
            };
            wire::write_frame(&mut stream, &request)?;
            loop {
                let response =
                    wire::read_frame(&mut stream)?.context("native host disconnected")?;
                if response.get("id") == Some(&json!(id)) {
                    if let Some(error) = response.get("error") {
                        return Ok((error.clone(), true));
                    }
                    return Ok((
                        response
                            .get("result")
                            .cloned()
                            .context("response missing result")?,
                        false,
                    ));
                }
                if response.get("id").is_none() && response["method"].is_string() {
                    if self.notifications.len() == 256 {
                        self.notifications.pop_front();
                    }
                    self.notifications.push_back(response);
                } else {
                    bail!("unexpected JSON-RPC response ID");
                }
            }
        })();
        // A transport failure may leave half a frame unread. Never reuse that stream.
        if result.is_err() {
            self.stream.take();
        }
        let (value, is_error) = result.map_err(|e| anyhow!("browser request failed: {e:#}"))?;
        if is_error {
            bail!(
                "browser RPC error {}: {}",
                value["code"],
                value["message"].as_str().unwrap_or("unknown error")
            );
        }
        Ok(value)
    }
    pub fn drain_notifications(&mut self) -> Vec<Value> {
        self.notifications.drain(..).collect()
    }
    pub fn ping(&mut self) -> Result<Value> {
        self.request("ping", json!({}))
    }
    pub fn info(&mut self) -> Result<Value> {
        self.request("getInfo", json!({}))
    }
    pub fn tabs(&mut self) -> Result<Value> {
        self.request("getTabs", json!({}))
    }
    pub fn user_tabs(&mut self) -> Result<Value> {
        self.request("getUserTabs", json!({}))
    }
    pub fn create_tab(&mut self) -> Result<Value> {
        self.request("createTab", json!({}))
    }
    pub fn claim_tab(&mut self, tab_id: i64) -> Result<Value> {
        self.request("claimUserTab", json!({"tabId":tab_id}))
    }
    pub fn name_session(&mut self, name: &str) -> Result<Value> {
        self.request("nameSession", json!({"name":name}))
    }
    pub fn finalize_tabs(&mut self, keep: Value) -> Result<Value> {
        self.request("finalizeTabs", json!({"keep":keep}))
    }
    pub fn cdp(&mut self, tab_id: i64, method: &str, params: Value) -> Result<Value> {
        self.cdp_with_timeout(tab_id, method, params, self.options.timeout)
    }
    pub(crate) fn cdp_with_timeout(
        &mut self,
        tab_id: i64,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if tab_id < 0 || !params.is_object() {
            bail!("CDP requires a nonnegative tab ID and object params");
        }
        let deadline = Instant::now() + timeout.min(self.options.timeout);
        // The extension deliberately requires attachment before executeCdp.
        // Attach is idempotent and also repairs attachment after a worker restart.
        self.request_with_timeout("attach", json!({"tabId":tab_id}), timeout)?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("CDP request timed out")?;
        self.request_with_timeout(
            "executeCdp",
            json!({"target":{"tabId":tab_id},"method":method,"commandParams":params}),
            remaining,
        )
    }
}
struct DeadlineStream<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
}
impl DeadlineStream<'_> {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    anyhow!("browser request timed out"),
                )
            })
    }
}
impl Read for DeadlineStream<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        allow_closed_peer(self.stream.set_read_timeout(Some(self.remaining()?)))?;
        self.stream.read(b)
    }
}
impl Write for DeadlineStream<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        allow_closed_peer(self.stream.set_write_timeout(Some(self.remaining()?)))?;
        self.stream.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

// macOS can reject SO_RCVTIMEO/SO_SNDTIMEO on a disconnected Unix socket.
// Read buffered data (or EOF) instead of discarding the peer's final response.
fn allow_closed_peer(result: io::Result<()>) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    if result
        .as_ref()
        .is_err_and(|e| e.raw_os_error() == Some(libc::EINVAL))
    {
        return Ok(());
    }
    result
}
