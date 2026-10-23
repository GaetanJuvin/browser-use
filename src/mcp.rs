//! MCP stdio transport. Standard output is exclusively newline JSON-RPC.
use crate::{
    cli::{Action, Runner},
    wire::MAX_FRAME,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn tools() -> Value {
    let string = json!({"type":"string"});
    let tab = json!({"type":"integer","minimum":0});
    let mut list = Vec::new();
    for (name, description) in [
        ("ping", "Check extension connectivity"),
        ("info", "Extension metadata"),
        ("tabs", "List tabs in this session"),
        ("user_tabs", "List user tabs before claiming one"),
        ("turn_ended", "End this turn"),
    ] {
        list.push(
            json!({"name":name,"description":description,"inputSchema":schema(json!({}),&[])}),
        );
    }
    for (name, description, props, required) in [
        (
            "open_tab",
            "Create a session tab, optionally navigating",
            json!({"url":string}),
            vec![],
        ),
        (
            "claim_tab",
            "Claim an existing user tab",
            json!({"tab_id":tab}),
            vec!["tab_id"],
        ),
        (
            "navigate",
            "Navigate a session-owned tab",
            json!({"tab_id":tab,"url":string}),
            vec!["tab_id", "url"],
        ),
        (
            "cdp",
            "Execute a Chrome DevTools Protocol command",
            json!({"tab_id":tab,"method":string,"params":{"type":"object"}}),
            vec!["tab_id", "method"],
        ),
        (
            "evaluate",
            "Evaluate JavaScript in a session-owned tab",
            json!({"tab_id":tab,"expression":string}),
            vec!["tab_id", "expression"],
        ),
        (
            "snapshot",
            "Read visible page text",
            json!({"tab_id":tab}),
            vec!["tab_id"],
        ),
        (
            "screenshot",
            "Capture a base64 PNG screenshot",
            json!({"tab_id":tab}),
            vec!["tab_id"],
        ),
        (
            "click",
            "Click the first CSS selector match",
            json!({"tab_id":tab,"selector":string}),
            vec!["tab_id", "selector"],
        ),
        (
            "fill",
            "Fill an input or textarea and dispatch input/change events",
            json!({"tab_id":tab,"selector":string,"text":string}),
            vec!["tab_id", "selector", "text"],
        ),
        (
            "wait_load",
            "Wait for load or domcontentloaded",
            json!({"tab_id":tab,"state":{"type":"string","enum":["load","domcontentloaded"]}}),
            vec!["tab_id"],
        ),
        (
            "page_info",
            "Get document title, URL and load state",
            json!({"tab_id":tab}),
            vec!["tab_id"],
        ),
        (
            "history",
            "Search Chrome history",
            json!({"query":string,"limit":{"type":"integer","minimum":1,"maximum":10000}}),
            vec![],
        ),
        (
            "name_session",
            "Name the session tab group",
            json!({"name":string}),
            vec!["name"],
        ),
        (
            "finalize_tabs",
            "Close agent tabs or release claimed user tabs, optionally retaining handoff/deliverable tabs",
            json!({"keep":{"type":"array","items":{"type":"object","properties":{"tabId":tab,"status":{"type":"string","enum":["handoff","deliverable"]}},"required":["tabId","status"],"additionalProperties":false}}}),
            vec![],
        ),
        (
            "call",
            "Call any extension JSON-RPC method",
            json!({"method":string,"params":{"type":"object"}}),
            vec!["method"],
        ),
        (
            "run_action_plan",
            "Execute one CLI browser action per line with shared default tab",
            json!({"commands":string}),
            vec!["commands"],
        ),
    ] {
        list.push(
            json!({"name":name,"description":description,"inputSchema":schema(props,&required)}),
        );
    }
    json!(list)
}
fn required_string(args: &Value, key: &str) -> Result<String> {
    args[key]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("{key} must be a string"))
}
fn required_tab(args: &Value) -> Result<Option<i64>> {
    let id = args["tab_id"]
        .as_i64()
        .filter(|id| *id >= 0)
        .context("tab_id must be a nonnegative integer")?;
    Ok(Some(id))
}
fn object_param(args: &Value) -> Result<String> {
    let p = args.get("params").cloned().unwrap_or(json!({}));
    if !p.is_object() {
        bail!("params must be an object");
    }
    Ok(p.to_string())
}
fn execute(runner: &mut Runner, name: &str, args: &Value) -> Result<Value> {
    // Validate the public schema's required fields and basic types before connecting.
    let tool = tools()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .cloned()
        .context("unknown tool")?;
    let object = args.as_object().context("arguments must be an object")?;
    let schema = &tool["inputSchema"];
    for key in schema["required"].as_array().unwrap() {
        if !object.contains_key(key.as_str().unwrap()) {
            bail!("missing required argument {key}");
        }
    }
    for (key, value) in object {
        let field = &schema["properties"][key];
        if field.is_null() {
            bail!("unknown argument {key}");
        }
        let valid = match field["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.as_i64().is_some(),
            Some("object") => value.is_object(),
            Some("array") => value.is_array(),
            _ => false,
        };
        if !valid {
            bail!("invalid type for {key}");
        }
        if let Some(min) = field["minimum"].as_i64() {
            if value.as_i64().unwrap() < min {
                bail!("{key} is below minimum");
            }
        }
        if let Some(max) = field["maximum"].as_i64() {
            if value.as_i64().unwrap() > max {
                bail!("{key} is above maximum");
            }
        }
        if let Some(choices) = field["enum"].as_array() {
            if !choices.contains(value) {
                bail!("invalid {key}");
            }
        }
    }
    let action = match name {
        "ping" => Action::Ping,
        "info" => Action::Info,
        "tabs" => Action::Tabs,
        "user_tabs" => Action::UserTabs,
        "turn_ended" => Action::TurnEnded,
        "open_tab" => Action::OpenTab {
            url: args
                .get("url")
                .map(|_| required_string(args, "url"))
                .transpose()?,
        },
        "claim_tab" => Action::ClaimTab {
            tab_id: required_tab(args)?.unwrap(),
        },
        "navigate" => Action::Navigate {
            tab_id: required_tab(args)?,
            url: required_string(args, "url")?,
        },
        "cdp" => Action::Cdp {
            tab_id: required_tab(args)?,
            method: required_string(args, "method")?,
            params: object_param(args)?,
        },
        "call" => Action::Call {
            method: required_string(args, "method")?,
            params: object_param(args)?,
        },
        "evaluate" => Action::Eval {
            tab_id: required_tab(args)?,
            expression: required_string(args, "expression")?,
        },
        "snapshot" => Action::Snapshot {
            tab_id: required_tab(args)?,
        },
        "screenshot" => Action::Screenshot {
            tab_id: required_tab(args)?,
        },
        "click" => Action::Click {
            tab_id: required_tab(args)?,
            selector: required_string(args, "selector")?,
        },
        "fill" => Action::Fill {
            tab_id: required_tab(args)?,
            selector: required_string(args, "selector")?,
            text: required_string(args, "text")?,
        },
        "wait_load" => Action::WaitLoad {
            tab_id: required_tab(args)?,
            state: args["state"].as_str().unwrap_or("load").into(),
        },
        "page_info" => Action::PageInfo {
            tab_id: required_tab(args)?,
        },
        "history" => Action::History {
            query: args["query"].as_str().unwrap_or("").into(),
            limit: args["limit"].as_u64().unwrap_or(100).try_into()?,
        },
        "name_session" => Action::NameSession {
            name: required_string(args, "name")?,
        },
        "finalize_tabs" => Action::FinalizeTabs {
            keep: args.get("keep").cloned().unwrap_or(json!([])).to_string(),
        },
        "run_action_plan" => {
            return Ok(json!(runner.run_plan(&required_string(args, "commands")?)?));
        }
        _ => bail!("unknown tool"),
    };
    runner.execute(action)
}
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub fn serve(runner: &mut Runner) -> Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut initialized = false;
    loop {
        let mut bytes = Vec::new();
        if (&mut reader)
            .take(MAX_FRAME as u64 + 1)
            .read_until(b'\n', &mut bytes)?
            == 0
        {
            break;
        }
        if bytes.len() > MAX_FRAME {
            writeln!(
                writer,
                "{}",
                error(Value::Null, -32700, "MCP line exceeds 1 MiB")
            )?;
            writer.flush()?;
            break;
        }
        let response = match serde_json::from_slice::<Value>(&bytes) {
            Err(_) => Some(error(Value::Null, -32700, "invalid JSON")),
            Ok(request) => {
                let id = request.get("id").cloned();
                if !request.is_object()
                    || request["jsonrpc"] != "2.0"
                    || !request["method"].is_string()
                    || id
                        .as_ref()
                        .is_some_and(|id| !id.is_string() && !id.is_number() && !id.is_null())
                {
                    Some(error(Value::Null, -32600, "invalid JSON-RPC request"))
                } else if let Some(id) = id {
                    let result = match request["method"].as_str().unwrap() {
                        "initialize" => {
                            initialized = true;
                            let requested =
                                request["params"]["protocolVersion"].as_str().unwrap_or("");
                            let version =
                                if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                                    .contains(&requested)
                                {
                                    requested
                                } else {
                                    "2025-06-18"
                                };
                            Ok(
                                json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"browser-use","version":env!("CARGO_PKG_VERSION")}}),
                            )
                        }
                        "ping" => Ok(json!({})),
                        "tools/list" if initialized => Ok(json!({"tools":tools()})),
                        "tools/call" if initialized => {
                            let name = request["params"]["name"].as_str().unwrap_or("");
                            let args = request["params"]
                                .get("arguments")
                                .cloned()
                                .unwrap_or(json!({}));
                            match execute(runner, name, &args) {
                                Ok(result) => Ok(
                                    json!({"content":[{"type":"text","text":result.to_string()}],"isError":false}),
                                ),
                                Err(e) => Ok(
                                    json!({"content":[{"type":"text","text":format!("{e:#}")}],"isError":true}),
                                ),
                            }
                        }
                        "tools/list" | "tools/call" => Err((-32002, "initialize first")),
                        _ => Err((-32601, "method not found")),
                    };
                    Some(match result {
                        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                        Err((code, msg)) => error(id, code, msg),
                    })
                } else {
                    None
                }
            }
        };
        if let Some(response) = response {
            writeln!(writer, "{response}")?;
            writer.flush()?;
        }
    }
    Ok(())
}
