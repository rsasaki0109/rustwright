use rustwright_bidi::BidiPage;
use std::time::Duration;

pub fn check(page: &BidiPage) {
    drop(page.wait_for_network_idle());
    drop(page.wait_for_network_idle_with_timeout(Duration::from_secs(1)));
}
