use crate::{Client, Options, browser::Tab};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

#[derive(Parser, Debug)]
#[command(
    name = "browser-use",
    version,
    about = "Automate your real Chrome browser with Rust"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
    #[arg(long, global = true)]
    pub socket_dir: Option<PathBuf>,
    #[arg(long, global = true, default_value = "browser-use-cli")]
    pub session_id: String,
    #[arg(long, global = true)]
    pub turn_id: Option<String>,
    #[arg(long, global = true, default_value_t = 15000)]
    pub timeout_ms: u64,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Chrome native-messaging relay (normally launched by Chrome).
    Host,
    /// Register the installed binary for your unpacked Chrome extension.
    Setup {
        #[arg(long)]
        extension_id: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Print a Chrome native-messaging host manifest.
    Manifest {
        #[arg(long)]
        extension_id: String,
    },
    /// Run a newline-delimited JSON-RPC MCP server on stdin/stdout.
    Mcp,
    /// Run one action per line; open-tab/claim-tab select the default tab.
    Run {
        #[arg(
            short = 'c',
            long,
            conflicts_with = "file",
            required_unless_present = "file"
        )]
        commands: Option<String>,
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    #[command(flatten)]
    Action(Action),
}
#[derive(Subcommand, Debug, Clone)]
pub enum Action {
    Ping,
    Info,
    Tabs,
    UserTabs,
    History {
        #[arg(default_value = "")]
        query: String,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    OpenTab {
        url: Option<String>,
    },
    ClaimTab {
        #[arg(long)]
        tab_id: i64,
    },
    Navigate {
        #[arg(long)]
        tab_id: Option<i64>,
        url: String,
    },
    Cdp {
        #[arg(long)]
        tab_id: Option<i64>,
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Evaluate JavaScript and return its JSON value.
    Eval {
        #[arg(long)]
        tab_id: Option<i64>,
        expression: String,
    },
    /// Read visible document text.
    Snapshot {
        #[arg(long)]
        tab_id: Option<i64>,
    },
    /// Return a base64 PNG in {data: ...}.
    Screenshot {
        #[arg(long)]
        tab_id: Option<i64>,
    },
    Click {
        #[arg(long)]
        tab_id: Option<i64>,
        selector: String,
    },
    Fill {
        #[arg(long)]
        tab_id: Option<i64>,
        selector: String,
        text: String,
    },
    WaitLoad {
        #[arg(long)]
        tab_id: Option<i64>,
        #[arg(default_value = "load")]
        state: String,
    },
    PageInfo {
        #[arg(long)]
        tab_id: Option<i64>,
    },
    NameSession {
        name: String,
    },
    FinalizeTabs {
        #[arg(long, default_value = "[]")]
        keep: String,
    },
    TurnEnded,
    Attach {
        #[arg(long)]
        tab_id: Option<i64>,
    },
    Detach {
        #[arg(long)]
        tab_id: Option<i64>,
    },
    MoveMouse {
        #[arg(long)]
        tab_id: Option<i64>,
        x: f64,
        y: f64,
    },
    /// Send any extension JSON-RPC method.
    Call {
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
}

pub struct Runner {
    pub options: Options,
    client: Option<Client>,
    default_tab: Option<i64>,
}
impl Runner {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            client: None,
            default_tab: None,
        }
    }
    fn client(&mut self) -> Result<&mut Client> {
        if self
            .client
            .as_ref()
            .is_none_or(|client| !client.is_connected())
        {
            self.client = Some(Client::connect(self.options.clone())?);
        }
        Ok(self.client.as_mut().unwrap())
    }
    fn tab_id(&self, id: Option<i64>) -> Result<i64> {
        let id = id
            .or(self.default_tab)
            .context("provide --tab-id or open/claim a tab earlier in this action plan")?;
        if id < 0 {
            bail!("tab ID must be non-negative");
        }
        Ok(id)
    }
    pub fn execute(&mut self, action: Action) -> Result<Value> {
        use Action::*;
        match action {
            Ping => self.client()?.ping(),
            Info => self.client()?.info(),
            Tabs => self.client()?.tabs(),
            UserTabs => self.client()?.user_tabs(),
            History { query, limit } => self
                .client()?
                .request("getUserHistory", json!({"query":query,"limit":limit})),
            NameSession { name } => self.client()?.name_session(&name),
            FinalizeTabs { keep } => {
                let keep: Value = serde_json::from_str(&keep)?;
                if !keep.is_array() {
                    bail!("keep must be a JSON array");
                }
                self.client()?.finalize_tabs(keep)
            }
            TurnEnded => self.client()?.request("turnEnded", json!({})),
            OpenTab { url } => {
                let result = self.client()?.create_tab()?;
                let id = result["id"]
                    .as_i64()
                    .context("createTab returned no numeric ID")?;
                self.default_tab = Some(id);
                if let Some(url) = url {
                    Tab {
                        client: self.client()?,
                        id,
                    }
                    .goto(&url)?;
                }
                Ok(result)
            }
            ClaimTab { tab_id } => {
                let result = self.client()?.claim_tab(tab_id)?;
                self.default_tab = Some(tab_id);
                Ok(result)
            }
            Navigate { tab_id, url } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .goto(&url)
            }
            Cdp {
                tab_id,
                method,
                params,
            } => {
                let id = self.tab_id(tab_id)?;
                self.client()?
                    .cdp(id, &method, serde_json::from_str(&params)?)
            }
            Call { method, params } => self
                .client()?
                .request(&method, serde_json::from_str(&params)?),
            Eval { tab_id, expression } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .evaluate(&expression)
            }
            Snapshot { tab_id } => {
                let id = self.tab_id(tab_id)?;
                Ok(json!(
                    Tab {
                        client: self.client()?,
                        id
                    }
                    .text()?
                ))
            }
            Screenshot { tab_id } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .screenshot()
            }
            Click { tab_id, selector } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .click(&selector)
            }
            Fill {
                tab_id,
                selector,
                text,
            } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .fill(&selector, &text)
            }
            WaitLoad { tab_id, state } => {
                let id = self.tab_id(tab_id)?;
                let timeout = self.options.timeout;
                Tab {
                    client: self.client()?,
                    id,
                }
                .wait_for_load(&state, timeout)?;
                Ok(json!({"loaded":true}))
            }
            PageInfo { tab_id } => {
                let id = self.tab_id(tab_id)?;
                Tab {
                    client: self.client()?,
                    id,
                }
                .evaluate(
                    "({title:document.title,url:location.href,readyState:document.readyState})",
                )
            }
            Attach { tab_id } => {
                let id = self.tab_id(tab_id)?;
                self.client()?.request("attach", json!({"tabId":id}))
            }
            Detach { tab_id } => {
                let id = self.tab_id(tab_id)?;
                self.client()?.request("detach", json!({"tabId":id}))
            }
            MoveMouse { tab_id, x, y } => {
                let id = self.tab_id(tab_id)?;
                self.client()?.request(
                    "moveMouse",
                    json!({"tabId":id,"x":x,"y":y,"waitForArrival":true}),
                )
            }
        }
    }
    /// Parse all lines before executing any actions; invalid syntax has no browser side effects.
    pub fn parse_plan(text: &str) -> Result<Vec<Action>> {
        let mut actions = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words =
                shell_words::split(line).with_context(|| format!("action plan line {}", i + 1))?;
            // Accept the upstream positional finalize-tabs [] form in action plans.
            if words.first().is_some_and(|w| w == "finalize-tabs")
                && words.get(1).is_some_and(|w| !w.starts_with('-'))
            {
                words.insert(1, "--keep".into());
            }
            let cli = Cli::try_parse_from(std::iter::once("browser-use".to_string()).chain(words))
                .map_err(|e| anyhow::anyhow!("action plan line {}: {e}", i + 1))?;
            let Command::Action(action) = cli.command else {
                bail!(
                    "action plan line {}: only browser actions are allowed",
                    i + 1
                );
            };
            actions.push(action);
        }
        Ok(actions)
    }
    pub fn run_plan(&mut self, text: &str) -> Result<Vec<Value>> {
        Self::parse_plan(text)?
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                self.execute(a)
                    .with_context(|| format!("action {} failed", i + 1))
            })
            .collect()
    }
}

pub fn run(cli: Cli) -> Result<()> {
    let options = Options {
        socket: cli.socket,
        socket_dir: cli.socket_dir.unwrap_or_else(crate::registry::default_dir),
        session_id: cli.session_id,
        turn_id: cli
            .turn_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        timeout: Duration::from_millis(cli.timeout_ms),
    };
    let mut runner = Runner::new(options);
    match cli.command {
        Command::Host => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(crate::host::run(&runner.options.socket_dir));
            runtime.shutdown_timeout(Duration::from_millis(100));
            result?;
        }
        Command::Setup {
            extension_id,
            output,
        } => {
            let path = crate::setup::install(&extension_id, output)?;
            println!(
                "{}",
                json!({"manifest":path,"next":"Reload the Browser Use extension in chrome://extensions, then run browser-use ping"})
            );
        }
        Command::Manifest { extension_id } => println!(
            "{}",
            serde_json::to_string_pretty(&crate::setup::manifest(
                &extension_id,
                &std::env::current_exe()?
            )?)?
        ),
        Command::Mcp => crate::mcp::serve(&mut runner)?,
        Command::Run { commands, file } => {
            let text = commands
                .map(Ok)
                .unwrap_or_else(|| std::fs::read_to_string(file.unwrap()))?;
            // Emit successful results as they happen, even if a later action fails.
            for (i, action) in Runner::parse_plan(&text)?.into_iter().enumerate() {
                println!(
                    "{}",
                    runner
                        .execute(action)
                        .with_context(|| format!("action {} failed", i + 1))?
                );
            }
        }
        Command::Action(action) => println!("{}", runner.execute(action)?),
    }
    Ok(())
}
