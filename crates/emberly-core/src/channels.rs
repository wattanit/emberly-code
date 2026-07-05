//! The engine/frontend channel boundary (Tech Spec §2, A-1).
//!
//! The engine owns all mutable session state and communicates *exclusively*
//! over these channels — there is no `Arc<Mutex<_>>` shared state. Events flow
//! out (`UiEvent`), commands flow in (`Command`). A frontend holds the mirror
//! ends; the engine cannot tell a TUI from a headless consumer apart, which is
//! the whole point (A-1, A-2).

use tokio::sync::mpsc;

use crate::command::Command;
use crate::event::UiEvent;

/// Default channel capacity. Bounded so a slow frontend exerts backpressure on
/// the engine rather than letting an unbounded queue grow without limit.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// The engine's ends of the channels: it sends events and receives commands.
pub struct EnginePorts {
    /// Events out to the frontend.
    pub events_tx: mpsc::Sender<UiEvent>,
    /// Commands in from the frontend.
    pub commands_rx: mpsc::Receiver<Command>,
}

/// A frontend's ends of the channels: it sends commands and receives events.
pub struct FrontendPorts {
    /// Commands in to the engine.
    pub commands_tx: mpsc::Sender<Command>,
    /// Events out from the engine.
    pub events_rx: mpsc::Receiver<UiEvent>,
}

/// Create a paired set of engine and frontend ports with
/// [`DEFAULT_CHANNEL_CAPACITY`].
#[must_use]
pub fn channel() -> (EnginePorts, FrontendPorts) {
    channel_with_capacity(DEFAULT_CHANNEL_CAPACITY)
}

/// Create a paired set of engine and frontend ports with an explicit capacity.
#[must_use]
pub fn channel_with_capacity(capacity: usize) -> (EnginePorts, FrontendPorts) {
    let (events_tx, events_rx) = mpsc::channel(capacity);
    let (commands_tx, commands_rx) = mpsc::channel(capacity);
    (
        EnginePorts {
            events_tx,
            commands_rx,
        },
        FrontendPorts {
            commands_tx,
            events_rx,
        },
    )
}
