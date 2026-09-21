use anyhow::{bail, ensure, Result};
use reqwest::{Client, Url};
use serde_json::Value;
use std::time::Duration;
pub fn credential(name: &str) -> Result<String> {
    crate::credentials::required(name)
}
pub fn client(timeout: u64) -> Result<Client> {
    ensure!(timeout > 0, "service timeout must be positive");
    Client::builder()
        .timeout(Duration::from_secs(timeout))
        .connect_timeout(Duration::from_secs(timeout.min(20)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow::anyhow!("Could not construct HTTP client"))
}
pub async fn post(client: &Client, endpoint: &str, key: &str, payload: &Value) -> Result<Value> {
    let url = Url::parse(endpoint).map_err(|_| anyhow::anyhow!("Invalid service endpoint URL"))?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Service endpoint must not contain credentials, query, or fragment"
    );
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "Service endpoint requires HTTPS (except loopback test servers)"
    );
    for attempt in 0..3u64 {
        let call = crate::ui::request_started();
        let service = "Jev";
        let activity = crate::ui::Activity::new(format!(
            "{service} API call {call} · attempt {} of 3",
            attempt + 1
        ));
        let response = client
            .post(url.clone())
            .bearer_auth(key)
            .json(payload)
            .send()
            .await;
        match response {
            Ok(mut response) => {
                let status = response.status();
                if status.is_success() {
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response
                        .chunk()
                        .await
                        .map_err(|_| anyhow::anyhow!("Service response read failed"))?
                    {
                        ensure!(
                            bytes.len() + chunk.len() <= 8 * 1024 * 1024,
                            "Service response exceeds 8 MiB limit"
                        );
                        bytes.extend_from_slice(&chunk);
                    }
                    return serde_json::from_slice(&bytes)
                        .map_err(|_| anyhow::anyhow!("Service returned malformed JSON"));
                }
                if attempt < 2 && (status.as_u16() == 429 || status.is_server_error()) {
                    let retry = response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(1 << attempt)
                        .min(30);
                    drop(activity);
                    crate::ui::retry(format!("HTTP {} · retrying in {retry}s…", status.as_u16()));
                    tokio::time::sleep(Duration::from_secs(retry)).await;
                    continue;
                }
                bail!(
                    "Service request failed with HTTP {} (response body omitted)",
                    status.as_u16()
                );
            }
            Err(error) => {
                if attempt < 2 && (error.is_timeout() || error.is_connect()) {
                    drop(activity);
                    crate::ui::retry(format!(
                        "Connection interrupted · retrying in {}s…",
                        1 << attempt
                    ));
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    continue;
                }
                bail!(
                    "Service transport failed{}; endpoint and response details omitted",
                    if error.is_timeout() { " (timeout)" } else { "" }
                );
            }
        }
    }
    bail!("Service retry limit reached")
}
