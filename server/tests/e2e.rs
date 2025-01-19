use fantoccini::{ClientBuilder, Locator};
use std::sync::LazyLock;

static WEBDRIVER_ADDRESS: LazyLock<String> = LazyLock::new(|| {
    let port = std::env::var("WEBDRIVER_PORT")
        .ok()
        .unwrap_or("4444".into());
    format!("http://localhost:{}", port)
});

#[ignore]
#[tokio::test]
async fn main() -> Result<(), fantoccini::error::CmdError> {
    let c = ClientBuilder::native()
        .connect(&WEBDRIVER_ADDRESS)
        .await
        .expect("failed to connect to WebDriver");

    // first, go to the Wikipedia page for Foobar
    c.goto("https://en.wikipedia.org/wiki/Foobar").await?;
    let url = c.current_url().await?;
    assert_eq!(url.as_ref(), "https://en.wikipedia.org/wiki/Foobar");

    // click "Foo (disambiguation)"
    c.find(Locator::Css(".mw-disambig")).await?.click().await?;

    // click "Foo Lake"
    c.find(Locator::LinkText("Foo Lake")).await?.click().await?;

    let url = c.current_url().await?;
    assert_eq!(url.as_ref(), "https://en.wikipedia.org/wiki/Foo_Lake");

    c.close().await
}
