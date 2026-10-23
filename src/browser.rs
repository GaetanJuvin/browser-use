use crate::{Client, Options};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub struct Browser {
    pub client: Client,
}
impl Browser {
    pub fn connect(options: Options) -> Result<Self> {
        Ok(Self {
            client: Client::connect(options)?,
        })
    }
    pub fn new_tab(&mut self) -> Result<Tab<'_>> {
        let id = self.client.create_tab()?["id"]
            .as_i64()
            .context("createTab did not return a numeric id")?;
        Ok(self.tab(id))
    }
    pub fn tab(&mut self, id: i64) -> Tab<'_> {
        Tab {
            client: &mut self.client,
            id,
        }
    }
}
pub struct Tab<'a> {
    pub client: &'a mut Client,
    pub id: i64,
}
impl Tab<'_> {
    pub fn goto(&mut self, url: &str) -> Result<Value> {
        let result = self
            .client
            .cdp(self.id, "Page.navigate", json!({"url":url}))?;
        if let Some(error) = result.get("errorText") {
            bail!("navigation failed: {error}");
        }
        Ok(result)
    }
    pub fn evaluate(&mut self, expression: &str) -> Result<Value> {
        self.evaluate_with_timeout(expression, None)
    }
    fn evaluate_with_timeout(
        &mut self,
        expression: &str,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let params = json!({"expression":expression,"returnByValue":true,"awaitPromise":true});
        let result = if let Some(timeout) = timeout {
            self.client
                .cdp_with_timeout(self.id, "Runtime.evaluate", params, timeout)?
        } else {
            self.client.cdp(self.id, "Runtime.evaluate", params)?
        };
        if let Some(details) = result.get("exceptionDetails") {
            bail!("JavaScript exception: {details}");
        }
        if result["result"].get("unserializableValue").is_some() {
            bail!("JavaScript result cannot be represented as JSON");
        }
        Ok(result["result"]
            .get("value")
            .cloned()
            .unwrap_or(Value::Null))
    }
    pub fn title(&mut self) -> Result<String> {
        self.evaluate("document.title")?
            .as_str()
            .map(str::to_owned)
            .context("title is not a string")
    }
    pub fn url(&mut self) -> Result<String> {
        self.evaluate("location.href")?
            .as_str()
            .map(str::to_owned)
            .context("URL is not a string")
    }
    /// Visible page text, not a Playwright accessibility-tree snapshot.
    pub fn text(&mut self) -> Result<String> {
        self.evaluate("document.body?.innerText ?? ''")?
            .as_str()
            .map(str::to_owned)
            .context("page text is not a string")
    }
    pub fn wait_for_load(&mut self, state: &str, timeout: Duration) -> Result<()> {
        let expression = match state {
            "domcontentloaded" => "document.readyState !== 'loading'",
            "load" => "document.readyState === 'complete'",
            _ => bail!("load state must be load or domcontentloaded"),
        };
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .context("page load timed out")?;
            if self.evaluate_with_timeout(expression, Some(remaining))? == true {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("page load timed out");
            }
            std::thread::sleep(
                Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    pub fn click(&mut self, selector: &str) -> Result<Value> {
        let selector = serde_json::to_string(selector)?;
        self.evaluate(&format!("(()=>{{const e=document.querySelector({selector});if(!e)throw Error('element not found');e.click();return true}})()"))
    }
    pub fn fill(&mut self, selector: &str, text: &str) -> Result<Value> {
        let selector = serde_json::to_string(selector)?;
        let text = serde_json::to_string(text)?;
        self.evaluate(&format!("(()=>{{const e=document.querySelector({selector});if(!e)throw Error('element not found');const p=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;const set=Object.getOwnPropertyDescriptor(p,'value').set;set.call(e,{text});e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return true}})()"))
    }
    pub fn screenshot(&mut self) -> Result<Value> {
        self.client
            .cdp(self.id, "Page.captureScreenshot", json!({"format":"png"}))
    }
    pub fn close(&mut self) -> Result<Value> {
        self.client.cdp(self.id, "Page.close", json!({}))
    }
}
