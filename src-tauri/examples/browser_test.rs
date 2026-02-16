use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Launching browser...");
    let (mut browser, mut handler) =
        Browser::launch(BrowserConfig::builder().with_head().build()?).await?;
    eprintln!("Browser launched.");

    let handle = tokio::spawn(async move {
        loop {
            match handler.next().await {
                Some(Ok(())) => {}
                Some(Err(e)) => eprintln!("[handler] error: {e}"),
                None => {
                    eprintln!("[handler] stream ended");
                    break;
                }
            }
        }
    });

    // fetch_targets registers existing targets with the handler
    eprintln!("Calling fetch_targets...");
    let targets = browser.fetch_targets().await?;
    eprintln!("Found {} targets", targets.len());

    // Wait for handler to process the attach commands
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Now pages() should find the attached target
    eprintln!("Getting pages...");
    let pages = browser.pages().await?;
    eprintln!("Found {} pages", pages.len());

    let page = if let Some(p) = pages.into_iter().next() {
        eprintln!("Using existing page.");
        p
    } else {
        return Err("no pages available".into());
    };

    // Try a raw CDP navigate instead of page.goto()
    eprintln!("Navigating via CDP...");
    let nav_result = page
        .execute(NavigateParams::new("https://example.com"))
        .await?;
    eprintln!("Navigate result: frame_id={:?}", nav_result.result.frame_id);

    // Wait for the page to load
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    eprintln!("Evaluating document.title...");
    let result = page.evaluate("document.title").await?;
    let title: String = result.into_value()?;
    eprintln!("Title: {title}");

    let result = page
        .evaluate("document.querySelector('h1')?.textContent || ''")
        .await?;
    let h1: String = result.into_value()?;
    eprintln!("H1: {h1}");

    eprintln!("Done! Closing...");
    drop(browser);
    handle.abort();
    Ok(())
}
