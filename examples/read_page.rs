use browser_use::{Browser, Options};
use serde_json::json;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut browser = Browser::connect(Options {
        session_id: format!("read-page-{}", uuid::Uuid::new_v4()),
        ..Options::default()
    })?;
    let result = (|| -> anyhow::Result<()> {
        let mut tab = browser.new_tab()?;
        tab.goto("https://example.com")?;
        tab.wait_for_load("domcontentloaded", Duration::from_secs(15))?;
        println!("{}\n{}", tab.title()?, tab.text()?);
        Ok(())
    })();
    let cleanup = browser.client.finalize_tabs(json!([]));
    result?;
    cleanup?;
    Ok(())
}
