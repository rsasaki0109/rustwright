#[path = "/workspace/rustwright/tests/support/browsers.rs"]
mod browser_policy;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let key = &args[1];
    let stage = &args[2];
    println!("policy_probe {key} {stage}");
    if stage == "discovery" {
        assert!(!browser_policy::available(key, Err::<std::path::PathBuf, _>("no installed candidate")));
    } else {
        assert!(browser_policy::optional_result(key, "launch", Err::<(), _>("browser exited immediately")).is_none());
    }
}
