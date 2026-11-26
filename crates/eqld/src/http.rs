use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Every request eqld makes to eqls is awaited inside the daemon tick, so an
/// answer that never comes must end in an error rather than a stalled tick.
pub fn client(request_timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT.min(request_timeout))
        .timeout(request_timeout)
        .build()
}
