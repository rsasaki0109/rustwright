fn main() {
    assert!(rustwright_common::INJECTED_SCRIPT.contains("prepareClick"));
    let _ = std::mem::size_of::<rustwright::AnyPage>();
    let _ = std::mem::size_of::<rustwright_bidi::BidiPage>();
    let _ = std::mem::size_of::<rustwright_browser::Chrome>();
    let _ = std::mem::size_of::<rustwright_cdp::CdpConnection>();
    let _ = std::mem::size_of::<rustwright_core::Browser>();
    let _ = std::mem::size_of::<rustwright_test::TestContext>();
    println!("Packaged Rustwright consumer loaded both backends and injected.js");
}
