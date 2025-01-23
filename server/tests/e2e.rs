#![cfg(feature = "e2e-test")]

use fantoccini::{Client, ClientBuilder, Locator};
use std::io;
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
use wewerewondering_api::build_app;

type ServerTaskHandle = JoinHandle<Result<(), io::Error>>;

const TESTRUN_SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);

static WEBDRIVER_ADDRESS: LazyLock<String> = LazyLock::new(|| {
    let port = std::env::var("WEBDRIVER_PORT")
        .ok()
        .unwrap_or("4444".into());
    format!("http://localhost:{}", port)
});

static SERVER_TASK_HANDLE: OnceLock<(String, ServerTaskHandle)> = OnceLock::new();

async fn init_webdriver_client() -> Client {
    let mut chrome_args = vec!["--disable-notifications", "--enable-automation"];
    if std::env::var("HEADLESS").ok().is_some() {
        chrome_args.extend(["--headless", "--disable-gpu", "--disable-dev-shm-usage"]);
    }
    let mut caps = serde_json::map::Map::new();
    caps.insert(
        "goog:chromeOptions".to_string(),
        serde_json::json!({
            "args": chrome_args,
        }),
    );
    println!("{:?}", caps);
    ClientBuilder::native()
        .capabilities(caps)
        .connect(&WEBDRIVER_ADDRESS)
        .await
        .expect("web driver to be available")
}

fn init() -> &'static (String, ServerTaskHandle) {
    SERVER_TASK_HANDLE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = tokio::spawn(async move {
            let app = build_app().await;
            let app = app.fallback_service(ServeDir::new(format!(
                "{}/client/dist",
                std::env::current_dir()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )));
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            let assigned_addr = listener.local_addr().unwrap();
            tx.send(assigned_addr).unwrap();
            axum::serve(listener, app.into_make_service()).await
        });
        let assigned_addr = rx.recv_timeout(TESTRUN_SETUP_TIMEOUT).unwrap();
        let app_addr = format!("http://localhost:{}", assigned_addr.port());
        (app_addr, handle)
    })
}

macro_rules! test {
    ($test_name:ident, $test_fn:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $test_name() {
            let (app_addr, _) = init();
            let c = init_webdriver_client().await;
            // run the test as a task catching any errors
            let res = tokio::spawn($test_fn(c.clone(), app_addr)).await;
            // clean up and ...
            c.close().await.unwrap();
            //  ... fail the test, if errors returned from the task
            if let Err(e) = res {
                std::panic::resume_unwind(Box::new(e));
            }
        }
    };
}

// ------------------------------- TESTS --------------------------------------

async fn start_new_q_and_a_session(c: Client, url: &String) {
    // the host novigates to the app's welcome page
    c.goto(url).await.unwrap();
    assert_eq!(c.current_url().await.unwrap().as_ref(), format!("{}/", url));
    assert_eq!(c.title().await.unwrap(), "Q&A");
    let new_event_btn = c.find(Locator::Css("button")).await.unwrap();
    assert_eq!(
        new_event_btn.text().await.unwrap().to_lowercase(),
        "open new q&a session"
    );

    // clicks the "Open new Q&A session" and ...
    new_event_btn.click().await.unwrap();

    // ... gets to the event's host view where they can
    let share_event_btn = c
        .wait()
        .at_most(DEFAULT_WAIT_TIMEOUT)
        .for_element(Locator::Css("[data-testid=share-event-button]"))
        .await
        .unwrap();
    let path = c.current_url().await.unwrap();
    let mut params = path.path_segments().unwrap();
    assert_eq!(params.next().unwrap(), "event");
    let _event_id = params.next().unwrap();
    let _host_secret = params.next().unwrap();
    assert!(params.next().is_none());

    // where there are initially no pending questions
    let r = c
        .find(Locator::Css("[data-testid=pending-questions]"))
        .await;

    share_event_btn.click().await.unwrap();

    let clipboard_content = c
        .execute_async("navigator.clipboard.read()", vec![])
        .await
        .unwrap();

    println!("{:}", clipboard_content);
}

test!(test_start_new_q_and_a_session, start_new_q_and_a_session);
