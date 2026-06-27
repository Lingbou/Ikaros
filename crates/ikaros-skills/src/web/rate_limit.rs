// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::time::sleep;
use url::Url;

const WEB_RATE_LIMIT_INTERVAL: Duration = Duration::from_millis(750);
static WEB_RATE_LIMIT: OnceLock<Mutex<BTreeMap<String, Instant>>> = OnceLock::new();

pub(in crate::web) async fn wait_for_web_rate_limit(kind: &str, raw_url: &str) {
    let host = Url::parse(raw_url)
        .ok()
        .and_then(|url| url.host_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".into());
    let key = format!("{kind}:{host}");
    let wait = {
        let mut state = WEB_RATE_LIMIT
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let wait = state
            .get(&key)
            .and_then(|last| last.checked_add(WEB_RATE_LIMIT_INTERVAL))
            .and_then(|next| next.checked_duration_since(now))
            .unwrap_or_default();
        state.insert(key, now + wait);
        wait
    };
    if !wait.is_zero() {
        sleep(wait).await;
    }
}
