#![cfg(feature = "e2e-test")]

use fantoccini::wd::WebDriverCompatibleCommand;
use fantoccini::{Client, ClientBuilder, Locator};
use serial_test::serial;
use std::io;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
use url::{ParseError, Url};

type ServerTaskHandle = JoinHandle<Result<(), io::Error>>;

const TESTRUN_SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);

static WEBDRIVER_ADDRESS: LazyLock<String> = LazyLock::new(|| {
    let port = std::env::var("WEBDRIVER_PORT")
        .ok()
        .unwrap_or("4444".into());
    format!("http://localhost:{}", port)
});

async fn init_webdriver_client() -> Client {
    let mut chrome_args = Vec::new();
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
    ClientBuilder::native()
        .capabilities(caps)
        .connect(&WEBDRIVER_ADDRESS)
        .await
        .expect("web driver to be available")
}

fn init() -> (String, ServerTaskHandle) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = tokio::spawn(async move {
        let app = wewerewondering_api::new().await;
        let app = app.fallback_service(ServeDir::new(
            std::env::current_dir().unwrap().join("../client/dist"),
        ));
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        let assigned_addr = listener.local_addr().unwrap();
        tx.send(assigned_addr).unwrap();
        axum::serve(listener, app.into_make_service()).await
    });
    let assigned_addr = rx.recv_timeout(TESTRUN_SETUP_TIMEOUT).unwrap();
    let app_addr = format!("http://localhost:{}", assigned_addr.port());
    (app_addr, handle)
}

// With out tests setup, we've got isolated sessions and a dedicated
// app per test but we still cannot run certain tests in parallel, e.g.
// we are currently missing clipboard isolation and need to run tests
// that are accessing navigator.clipboard _sequentially_.
macro_rules! serial_test {
    ($test_name:ident, $test_fn:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
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

#[derive(Debug, Clone)]
struct GrantClipboardReadCmd;

impl WebDriverCompatibleCommand for GrantClipboardReadCmd {
    fn endpoint(&self, base_url: &Url, session_id: Option<&str>) -> Result<Url, ParseError> {
        base_url.join(format!("session/{}/permissions", session_id.as_ref().unwrap()).as_str())
    }

    fn method_and_body(&self, _: &url::Url) -> (http::Method, Option<String>) {
        (
            http::Method::POST,
            Some(
                serde_json::json!({"descriptor": {"name": "clipboard-read"}, "state": "granted"})
                    .to_string(),
            ),
        )
    }
}

// ------------------------------- TESTS --------------------------------------

async fn start_new_q_and_a_session(c: Client, url: String) {
    // the host novigates to the app's welcome page
    c.goto(&url).await.unwrap();
    assert_eq!(c.current_url().await.unwrap().as_ref(), format!("{}/", url));
    assert_eq!(c.title().await.unwrap(), "Q&A");
    let create_event_btn = c
        .wait()
        .at_most(DEFAULT_WAIT_TIMEOUT)
        .for_element(Locator::Css("[data-testid=create-event-button]"))
        .await
        .unwrap();

    // starts a new Q&A session and ...
    create_event_btn.click().await.unwrap();

    // ... gets redirected to the event's host view where they can ...
    let share_event_btn = c
        .wait()
        .at_most(DEFAULT_WAIT_TIMEOUT)
        .for_element(Locator::Css("[data-testid=share-event-button]"))
        .await
        .unwrap();
    let event_url_for_host = c.current_url().await.unwrap();
    let mut params = event_url_for_host.path_segments().unwrap();
    assert_eq!(params.next().unwrap(), "event");
    let event_id = params.next().unwrap();
    let _host_secret = params.next().unwrap();
    assert!(params.next().is_none());

    // ... grab the event's guest url to share it later with folks
    share_event_btn.click().await.unwrap();
    c.issue_cmd(GrantClipboardReadCmd).await.unwrap();
    let event_url_for_guest: Url = c
        .execute_async(
            r#"
                const [callback] = arguments;
                navigator.clipboard.readText().then((text) => callback(text));
            "#,
            vec![],
        )
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut params = event_url_for_guest.path_segments().unwrap();
    assert_eq!(params.next().unwrap(), "event");
    let event_id_for_guest = params.next().unwrap();

    assert_eq!(event_id_for_guest, event_id); // same event id
    assert!(params.next().is_none()); // but no secret

    // and there are currently no pending, answered, or hidden questions
    // related to the newly created event
    let pending_questions = c
        .find(Locator::Css("section[data-testid=pending-questions]"))
        .await
        .unwrap()
        .find_all(Locator::Css("article"))
        .await
        .unwrap();
    assert!(pending_questions.is_empty());
    assert!(c
        .find(Locator::Css("section[data-testid=answered-questions]"))
        .await
        .unwrap_err()
        .is_no_such_element());
    assert!(c
        .find(Locator::Css("section[data-testid=hidden-questions]"))
        .await
        .unwrap_err()
        .is_no_such_element());

    // let's make sure we are persisting the event
}

serial_test!(test_start_new_q_and_a_session, start_new_q_and_a_session);
serial_test!(test_start_new_q_and_a_session1, start_new_q_and_a_session);
serial_test!(test_start_new_q_and_a_session2, start_new_q_and_a_session);
serial_test!(test_start_new_q_and_a_session3, start_new_q_and_a_session);
