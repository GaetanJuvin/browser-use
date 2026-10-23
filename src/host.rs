//! Multiplex local clients onto the Chrome native-messaging pipe.
use crate::{registry, wire};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::UnixListener,
    sync::mpsc,
};

const MAX_CLIENTS: usize = 64;
const MAX_PENDING: usize = 4096;
const REQUEST_LIFETIME: Duration = Duration::from_secs(120);
struct Pending {
    client: u64,
    original: Value,
    since: Instant,
}
enum Event {
    Message(u64, Value),
    Gone(u64),
    Extension(Value),
    End(Option<String>),
}
struct ClientConnection {
    sender: mpsc::Sender<Value>,
    reader: tokio::task::AbortHandle,
    writer: tokio::task::AbortHandle,
}
impl ClientConnection {
    fn try_send(&self, value: Value) -> Result<(), mpsc::error::TrySendError<Value>> {
        self.sender.try_send(value)
    }
}

impl Drop for ClientConnection {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}
struct Cleanup {
    dir: PathBuf,
    socket: PathBuf,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        registry::cleanup(&self.dir, &self.socket);
    }
}
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub async fn run(dir: &Path) -> Result<()> {
    relay(tokio::io::stdin(), tokio::io::stdout(), dir).await
}

pub async fn relay<R, W>(mut input: R, mut output: W, dir: &Path) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    registry::ensure_private(dir)?;
    let socket = dir.join(format!("{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&socket).context("bind native host socket")?;
    let _cleanup = Cleanup {
        dir: dir.to_path_buf(),
        socket: socket.clone(),
    };
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    registry::write_active(dir, &socket)?;
    let (tx, mut rx) = mpsc::channel(256);
    let ext_tx = tx.clone();
    let extension_reader = tokio::spawn(async move {
        loop {
            match wire::read_async(&mut input).await {
                Ok(Some(v)) => {
                    if ext_tx.send(Event::Extension(v)).await.is_err() {
                        break;
                    }
                }
                end => {
                    let _ = ext_tx
                        .send(Event::End(end.err().map(|e| e.to_string())))
                        .await;
                    break;
                }
            }
        }
    });
    let mut clients: HashMap<u64, ClientConnection> = HashMap::new();
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let mut client_id = 0u64;
    let mut request_id = 0u64;
    let mut expiry = tokio::time::interval(Duration::from_secs(1));
    let result=async {
        loop {
            tokio::select! {
                incoming=listener.accept()=> {
                    let (stream,_)=incoming?;
                    if clients.len()>=MAX_CLIENTS { drop(stream); continue; }
                    client_id+=1;
                    let id=client_id;
                    let (mut reader,mut writer)=stream.into_split();
                    let (send,mut receive)=mpsc::channel::<Value>(64);
                    let reader_tx=tx.clone();
                    let reader_task=tokio::spawn(async move {
                        // Bound even partially transmitted frames from abandoned clients.
                        while let Ok(Ok(Some(v))) = tokio::time::timeout(Duration::from_secs(300),wire::read_async(&mut reader)).await {
                            if reader_tx.send(Event::Message(id,v)).await.is_err() { return; }
                        }
                        let _=reader_tx.send(Event::Gone(id)).await;
                    });
                    let writer_tx=tx.clone();
                    let writer_task=tokio::spawn(async move {
                        while let Some(v)=receive.recv().await {
                            if !matches!(tokio::time::timeout(Duration::from_secs(5),wire::write_async(&mut writer,&v)).await,Ok(Ok(()))) { break; }
                        }
                        let _=writer_tx.send(Event::Gone(id)).await;
                    });
                    clients.insert(id,ClientConnection { sender:send,reader:reader_task.abort_handle(),writer:writer_task.abort_handle() });
                }
                event=rx.recv()=>match event {
                    Some(Event::Message(client,mut v))=> {
                        let Some(send)=clients.get(&client) else { continue; };
                        let original=v.get("id").cloned();
                        if !v.is_object() || v["jsonrpc"]!="2.0" || !v["method"].is_string() || original.as_ref().is_some_and(|id| !id.is_string() && !id.is_number()) {
                            let _=send.try_send(error(original.unwrap_or(Value::Null),-32600,"invalid JSON-RPC request"));continue;
                        }
                        if pending.len()>=MAX_PENDING { let _=send.try_send(error(original.unwrap_or(Value::Null),-32000,"host busy"));continue; }
                        if let Some(original)=original {
                            request_id+=1;
                            let key=format!("rust:{request_id}");
                            v["id"]=json!(key);
                            pending.insert(key,Pending { client,original,since:Instant::now() });
                        }
                        tokio::time::timeout(Duration::from_secs(5),wire::write_async(&mut output,&v)).await.context("Chrome pipe stalled")??;
                    }
                    Some(Event::Gone(client))=> {
                        clients.remove(&client);pending.retain(|_,p|p.client!=client);
                    }
                    Some(Event::Extension(mut v))=> {
                        if let Some(key)=v.get("id").and_then(Value::as_str).map(str::to_owned) {
                            if let Some(p)=pending.remove(&key) {
                                v["id"]=p.original;
                                if let Some(send)=clients.get(&p.client) { if send.try_send(v).is_err() { clients.remove(&p.client); } }
                            }
                            // Unknown responses are late or malformed; never broadcast them.
                        } else if v.get("id").is_none() && v["method"].is_string() {
                            clients.retain(|_,send|send.try_send(v.clone()).is_ok());
                        }
                    }
                    Some(Event::End(Some(message)))=>bail!("Chrome native pipe: {message}"),
                    Some(Event::End(None))|None=>break,
                },
                _=expiry.tick()=> {
                    pending.retain(|_,p| {
                        if !clients.contains_key(&p.client) { return false; }
                        if p.since.elapsed()<REQUEST_LIFETIME { return true; }
                        if let Some(send)=clients.get(&p.client) { let _=send.try_send(error(p.original.clone(),-32000,"request expired")); }
                        false
                    });
                }
                _=tokio::signal::ctrl_c()=>break,
            }
        }
        Ok(())
    }.await;
    extension_reader.abort();
    drop(clients);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn evicting_a_connection_cancels_both_socket_tasks() {
        let (sender, _receiver) = mpsc::channel(1);
        let reader = tokio::spawn(std::future::pending::<()>());
        let writer = tokio::spawn(std::future::pending::<()>());
        let connection = ClientConnection {
            sender,
            reader: reader.abort_handle(),
            writer: writer.abort_handle(),
        };
        drop(connection);
        tokio::task::yield_now().await;
        assert!(reader.is_finished(), "evicted reader is still alive");
        assert!(writer.is_finished(), "evicted writer is still alive");
        reader.abort();
        writer.abort();
    }
}
