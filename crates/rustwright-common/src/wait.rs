//! Load states and element wait states.

/// A page load state, mirroring Chrome's lifecycle events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadState {
    /// The `load` event has fired.
    Load,
    /// `DOMContentLoaded` has fired.
    DomContentLoaded,
    /// The network has been idle for 500 ms.
    NetworkIdle,
}

impl LoadState {
    /// The CDP `Page.lifecycleEvent` name for this state.
    pub fn lifecycle_name(self) -> &'static str {
        match self {
            LoadState::Load => "load",
            LoadState::DomContentLoaded => "DOMContentLoaded",
            LoadState::NetworkIdle => "networkIdle",
        }
    }

    /// A human-readable name.
    pub fn as_str(self) -> &'static str {
        match self {
            LoadState::Load => "load",
            LoadState::DomContentLoaded => "domcontentloaded",
            LoadState::NetworkIdle => "networkidle",
        }
    }
}

/// The state of an element to wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaitState {
    /// The element exists in the DOM.
    Attached,
    /// The element does not exist in the DOM.
    Detached,
    /// The element exists and is visible.
    Visible,
    /// The element is absent or not visible.
    Hidden,
}

impl WaitState {
    /// The state name understood by the injected page script.
    pub fn as_str(self) -> &'static str {
        match self {
            WaitState::Attached => "attached",
            WaitState::Detached => "detached",
            WaitState::Visible => "visible",
            WaitState::Hidden => "hidden",
        }
    }
}
