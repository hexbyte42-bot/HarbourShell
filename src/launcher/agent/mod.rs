//! The on-device Agent: a chat with an LLM that is handed the Harbour API as tools.
//!
//! * [`config`]   `~/.config/harbour/agent.json` (providers, keys by env var)
//! * [`provider`] the `Provider` trait, the wire formats ([`openai`], [`anthropic`]), the mock
//! * [`http`]     the transport (a `curl` child process; the key goes through stdin only)
//! * [`tools`]    the tool list, argument parsing and the tools that need no host state
//! * [`settings`] get / set of the Settings values the agent may touch
//! * [`history`]  saving chats under `~/.local/share/harbour/agent/`
//! * [`muse`]     Meta Muse as a provider, and Muse's `harbour.*` commands into the tools
//! * [`input`]    the input source trait (keyboard now, microphone later)
//! * [`app`]      the chat screen (a `LauncherApp`) and the conversation state machine

pub mod anthropic;
pub mod app;
pub mod config;
pub mod history;
pub mod http;
pub mod input;
pub mod muse;
pub mod openai;
pub mod provider;
pub mod settings;
pub mod tools;
pub mod types;
pub mod voice;

pub use app::AgentApp;
