use clap::Parser;
fn main() -> anyhow::Result<()> {
    // Chrome passes its extension origin, rather than a configurable subcommand.
    if std::env::args()
        .nth(1)
        .is_some_and(|s| s.starts_with("chrome-extension://"))
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let result =
            runtime.block_on(browser_use::host::run(&browser_use::registry::default_dir()));
        runtime.shutdown_timeout(std::time::Duration::from_millis(100));
        return result;
    }
    browser_use::cli::run(browser_use::cli::Cli::parse())
}
