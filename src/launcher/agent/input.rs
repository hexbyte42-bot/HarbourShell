//! Where the user's words come from. The keyboard is the line editor in the chat screen; a
//! microphone (speech to text) later implements [`InputSource`] and hands finished utterances to
//! the Agent, which treats them exactly like a typed line.

use std::collections::VecDeque;

pub trait InputSource {
    /// A finished utterance, if one is ready (never blocks).
    fn poll(&mut self) -> Option<String>;
    fn name(&self) -> &str;
}

/// A queue fed from elsewhere (tests; a future speech thread can push into one through a channel).
#[derive(Default)]
pub struct QueueInput {
    pub pending: VecDeque<String>,
}

impl InputSource for QueueInput {
    fn poll(&mut self) -> Option<String> {
        self.pending.pop_front()
    }
    fn name(&self) -> &str {
        "queue"
    }
}
